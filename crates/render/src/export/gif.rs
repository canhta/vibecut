//! Runs the silent animated-GIF pass of an export over the shared frame producer.

use super::execution::frame_offset;
use super::{ExportReceipt, ExportSettings};
use crate::{
    Error, ExecutionHooks, Image, MemoryKind, Renderer, Result, SourceProvider, Stage, check,
};
use media::{Container, GifEncoder, GifEncoderSettings, Muxer};
use time::{Tick, TimeRange};

fn encoder_settings(settings: &ExportSettings) -> GifEncoderSettings {
    GifEncoderSettings {
        width: settings.render.width,
        height: settings.render.height,
        frame_rate: settings.fps,
        options: settings.gif,
    }
}

/// Output frames of `range` on the export grid, rounded up to whole frames.
pub(super) fn frame_count(range: TimeRange, settings: &ExportSettings) -> Result<u64> {
    let frame = settings.fps.frame_duration()?;
    if frame <= Tick::ZERO {
        return Err(Error::Invalid("export settings/range"));
    }
    let frames = (i128::from(range.duration().0) + i128::from(frame.0) - 1) / i128::from(frame.0);
    u64::try_from(frames).map_err(|_| Error::Invalid("export frame expansion"))
}

/// Refuses palette, dimension, rate and duration choices outside the GIF bounds.
/// Non-GIF containers pass untouched. Runs before any output file exists.
pub(super) fn admission(range: TimeRange, settings: &ExportSettings) -> Result<()> {
    if settings.container != Container::Gif {
        return Ok(());
    }
    let gif = encoder_settings(settings);
    gif.validate()
        .and_then(|()| gif.admits_frames(frame_count(range, settings).unwrap_or(u64::MAX)))
        .map_err(|limit| Error::Media(limit.into()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gif_export(
    range: TimeRange,
    settings: &ExportSettings,
    file: project::atomic::StreamingWriter,
    renderer: &mut Renderer<'_>,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    count: u64,
) -> Result<ExportReceipt> {
    let _scratch = hooks.reserve(
        MemoryKind::Export,
        Image::bytes(settings.render.width, settings.render.height)? + (16 << 20),
    )?;
    let mut encoder = GifEncoder::new(encoder_settings(settings))?;
    let mut mux = Muxer::new(file, Container::Gif)?;
    let track = mux.add_parameters(&encoder.parameters(), encoder.time_base())?;
    let dt = settings.fps.frame_duration()?;
    for n in 0..count {
        check(hooks)?;
        hooks.progress(Stage::Render, n as f32 / count as f32)?;
        let offset = frame_offset(dt, n)?;
        let image = renderer.render_frame(
            range
                .start()
                .checked_add(offset)
                .ok_or(Error::Invalid("render tick"))?,
            sources,
            hooks,
        )?;
        // A GIF has no colour tags: pack with the sRGB transfer so project colours survive.
        let rgba = image.video_rgba8(13, hooks)?;
        hooks.progress(Stage::Encode, n as f32 / count as f32)?;
        for packet in encoder.encode_rgba8(&rgba.bytes, offset)? {
            mux.write_packet(track, &packet)?;
        }
    }
    for packet in encoder.flush()? {
        mux.write_packet(track, &packet)?;
    }
    check(hooks)?;
    mux.finish()?;
    Ok(ExportReceipt {
        video_frames: count,
        audio_samples: 0,
        stream_copied: false,
        degraded: Vec::new(),
    })
}

/// Encoded bytes per pixel per frame at a 256-colour palette and the default dither: one
/// measurement (Windows, 2026-11-14), 3 s of 1080p29.97 camera footage at the 240P default
/// (426x240, 90 frames) wrote 6,053,178 bytes. Busier footage writes more (another 3 s of the
/// same source wrote 6,994,691 bytes) and flat graphics far less.
const BYTES_PER_PIXEL_FRAME_AT_256: f64 = 6_053_178.0 / (426.0 * 240.0 * 90.0);

/// Expected bytes of a GIF of `seconds`: pixels x frames x a measured ratio, scaled by the
/// palette's code width (`log2(colors) / 8`, since LZW codes are that many bits). GIF is
/// silent, so there is no audio term, and the video bit-rate model does not apply.
/// Dither and content are not modelled. This is the one GIF branch of
/// `ExportSettings::estimated_bytes` (`estimate.rs`).
pub(super) fn estimated_bytes(settings: &ExportSettings, seconds: f64) -> f64 {
    let pixels = f64::from(settings.render.width) * f64::from(settings.render.height);
    let fps = settings.fps.as_f64().min(media::GIF_MAX_FPS as f64);
    let width = f64::from(settings.gif.max_colors.max(2)).log2() / 8.0;
    seconds.max(0.0) * fps * pixels * BYTES_PER_PIXEL_FRAME_AT_256 * width
}
