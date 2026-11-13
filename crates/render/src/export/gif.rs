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
