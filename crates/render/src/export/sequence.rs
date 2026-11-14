//! Runs the PNG image-sequence pass of an export over the shared frame producer.

use super::execution::frame_offset;
use super::gif::frame_count;
use super::{ExportReceipt, ExportSettings};
use crate::{Error, ExecutionHooks, MemoryKind, Renderer, Result, SourceProvider, Stage, check};
use png_codec::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use project::atomic::{DirectoryStage, Overwrite};
use std::path::{Path, PathBuf};
use time::TimeRange;

/// Most frames one sequence holds, so every name stays `<name>_%05d.png`.
pub const MAX_SEQUENCE_FRAMES: u64 = 99_999;
const PATTERN: &str = "_%05d.png";

/// `<prefix>_<number as 5 digits>.png`; numbering starts at 1.
pub fn sequence_frame_name(prefix: &str, number: u64) -> String {
    format!("{prefix}_{number:05}.png")
}

/// Whether `name` is one of `prefix`'s frames.
fn is_frame(prefix: &str, name: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('_'))
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|digits| digits.len() >= 5 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The frame-name prefix a destination directory carries: its own file name.
pub fn sequence_prefix(directory: &Path) -> Result<String> {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .ok_or(Error::Invalid("image sequence name"))?;
    Ok(name.to_owned())
}

/// The destination directory a `<name>_%05d.png` output pattern selects, for CLI and
/// control callers. Other paths are not sequences. The pattern's directory part is kept,
/// and the directory is named `<name>`.
pub fn sequence_destination(path: &Path) -> Result<Option<PathBuf>> {
    let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let Some(name) = file.strip_suffix(PATTERN) else {
        return Ok(None);
    };
    if name.is_empty() || name.contains('%') {
        return Err(Error::Invalid("image sequence pattern"));
    }
    let directory = path.with_file_name(name);
    sequence_prefix(&directory)?;
    Ok(Some(directory))
}

/// Stage for a sequence destination: new directory name, or an earlier sequence of the
/// same name under `Replace`. Returns the frame prefix with it.
pub fn sequence_stage(
    directory: &Path,
    overwrite: Overwrite,
    max_bytes: u64,
) -> Result<(DirectoryStage, String)> {
    let prefix = sequence_prefix(directory)?;
    let owner = prefix.clone();
    let stage = DirectoryStage::new(
        directory,
        overwrite,
        max_bytes,
        Box::new(move |name| is_frame(&owner, name)),
    )?;
    Ok((stage, prefix))
}

/// Refuses a range longer than a sequence can name. Other outputs pass untouched.
pub(super) fn admission(range: TimeRange, settings: &ExportSettings) -> Result<()> {
    if !settings.image_sequence {
        return Ok(());
    }
    if frame_count(range, settings).unwrap_or(u64::MAX) > MAX_SEQUENCE_FRAMES {
        return Err(Error::Invalid("image sequence frame count"));
    }
    Ok(())
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve(rgba.len() / 2 + 1024)?;
    PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|_| Error::Invalid("png frame encoding"))?;
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sequence_export(
    range: TimeRange,
    settings: &ExportSettings,
    stage: &mut DirectoryStage,
    prefix: &str,
    renderer: &mut Renderer<'_>,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    count: u64,
) -> Result<ExportReceipt> {
    let (width, height) = (settings.render.width, settings.render.height);
    let _scratch = hooks.reserve(
        MemoryKind::Export,
        u64::from(width) * u64::from(height) * 4 + (16 << 20),
    )?;
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
        // PNG is an sRGB image boundary that keeps straight alpha, like a still snapshot.
        let rgba = image.rgba8(13, hooks)?;
        hooks.progress(Stage::Encode, n as f32 / count as f32)?;
        let png = encode_png(&rgba.bytes, width, height)?;
        stage.write_file(&sequence_frame_name(prefix, n + 1), &png)?;
    }
    check(hooks)?;
    Ok(ExportReceipt {
        video_frames: count,
        audio_samples: 0,
        stream_copied: false,
        degraded: Vec::new(),
    })
}
