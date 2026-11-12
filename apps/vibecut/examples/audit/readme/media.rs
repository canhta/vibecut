//! Encodes, decodes, probes, measures README media outputs.
#![allow(
    clippy::indexing_slicing,
    reason = "an offline audit capture indexes its own manifest and fixed-size frame buffers; a panic aborts the tool, never a person's project"
)]
use super::super::Failure;
use super::process::run_tool;
use super::publication::publish;
use super::source::README_GIF_FRAMES;
use super::source::README_MAX_GIF;
use super::source::README_MAX_JPG;
use super::source::TOOL_METADATA_TIMEOUT;
use super::source::TOOL_STDERR_CAP;
use super::source::TOOL_STDOUT_CAP;
use super::source::TOOL_WORK_TIMEOUT;
use std::fs;
use std::path::Path;

pub(super) fn ffmpeg(args: &[&str]) -> Result<(), Failure> {
    let mut all = vec!["-hide_banner", "-loglevel", "error", "-y"];
    all.extend_from_slice(args);
    let output = run_tool("ffmpeg", &all, TOOL_WORK_TIMEOUT, TOOL_STDOUT_CAP)?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        )
        .into());
    }
    Ok(())
}

pub(super) fn decode_rgb(path: &Path, filter: Option<&str>) -> Result<Vec<u8>, Failure> {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["-hide_banner", "-loglevel", "error", "-i", path.as_str()];
    if let Some(filter) = filter {
        args.extend(["-vf", filter]);
    }
    args.extend(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    let output = run_tool("ffmpeg", &args, TOOL_WORK_TIMEOUT, TOOL_STDOUT_CAP)?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg decode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

pub(super) fn probe_field(path: &Path, select: &str, entry: &str) -> Result<String, Failure> {
    let path_value = path.to_string_lossy().into_owned();
    let entry = format!("stream={entry}");
    let output = run_tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            select,
            "-show_entries",
            &entry,
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            &path_value,
        ],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("ffprobe field failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(super) fn decoded_frame_count(path: &Path) -> Result<u64, Failure> {
    let path_value = path.to_string_lossy().into_owned();
    let output = run_tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            &path_value,
        ],
        TOOL_WORK_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("ffprobe frame count failed".into());
    }
    String::from_utf8(output.stdout)?
        .trim()
        .parse::<u64>()
        .map_err(|_| "ffprobe frame count parse".into())
}

pub(super) fn image_stats(pixels: &[u8]) -> serde_json::Value {
    let channels = pixels.len() / 3;
    if channels == 0 {
        return serde_json::json!({"mean": 0.0, "min": 0, "max": 0, "spread": 0});
    }
    let sum: u64 = pixels.iter().map(|v| u64::from(*v)).sum();
    let min = pixels.iter().copied().min().unwrap_or(0);
    let max = pixels.iter().copied().max().unwrap_or(0);
    serde_json::json!({
        "mean": sum as f64 / pixels.len() as f64,
        "min": min,
        "max": max,
        "spread": max - min
    })
}

pub(super) fn difference(left: &[u8], right: &[u8]) -> serde_json::Value {
    let mut max = 0u8;
    let mut sum = 0u64;
    for (a, b) in left.iter().zip(right) {
        let delta = a.abs_diff(*b);
        max = max.max(delta);
        sum += u64::from(delta);
    }
    serde_json::json!({
        "mean_abs": sum as f64 / left.len().max(1) as f64,
        "max_abs": max
    })
}
pub(super) fn encode_jpg(raw: &Path, out: &Path) -> Result<serde_json::Value, Failure> {
    let name = out.file_name().ok_or("jpg filename")?;
    let temp = out.with_file_name(format!(".{}.tmp.jpg", name.to_string_lossy()));
    let mut bytes = 0;
    for quality in ["2", "4", "6", "8", "12"] {
        ffmpeg(&[
            "-i",
            &raw.to_string_lossy(),
            "-vf",
            "scale=960:-2:flags=lanczos",
            "-frames:v",
            "1",
            "-q:v",
            quality,
            &temp.to_string_lossy(),
        ])?;
        bytes = fs::metadata(&temp)?.len();
        if bytes <= README_MAX_JPG {
            break;
        }
    }
    if bytes > README_MAX_JPG {
        return Err(format!("jpg exceeds 512 KiB: {bytes}").into());
    }
    let width = probe_field(&temp, "v:0", "width")?.parse::<u32>()?;
    let height = probe_field(&temp, "v:0", "height")?.parse::<u32>()?;
    if (width, height) != (960, 600) {
        return Err(format!("jpg dimensions {width}x{height}").into());
    }
    let decoded = decode_rgb(&temp, None)?;
    let source = decode_rgb(raw, Some("scale=960:-2:flags=lanczos"))?;
    let stats = image_stats(&decoded);
    if stats["spread"].as_u64().unwrap_or(0) < 16 {
        return Err("jpg is blank".into());
    }
    let loss = difference(&source, &decoded);
    publish(&temp, out)?;
    Ok(serde_json::json!({
        "bytes": bytes,
        "width": width,
        "height": height,
        "decoded_pixels": decoded.len() / 3,
        "stats": stats,
        "source_vs_decoded": loss
    }))
}

pub(super) fn gif_timing(path: &Path) -> Result<(usize, f64, f64), Failure> {
    let path_value = path.to_string_lossy().into_owned();
    let output = run_tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=pts_time,duration_time",
            "-of",
            "csv=p=0",
            &path_value,
        ],
        TOOL_METADATA_TIMEOUT,
        1 << 20,
    )?;
    if !output.status.success() {
        return Err("gif timing probe failed".into());
    }
    let mut frames = 0;
    let mut max_delay_error = 0.0f64;
    let mut end = 0.0f64;
    for line in String::from_utf8(output.stdout)?.lines() {
        let mut fields = line.split(',');
        let Some(pts) = fields.next().and_then(|v| v.trim().parse::<f64>().ok()) else {
            continue;
        };
        let delay = fields
            .next()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
        frames += 1;
        max_delay_error = max_delay_error.max((delay - 0.1).abs());
        end = pts + delay;
    }
    Ok((frames, max_delay_error, end))
}

pub(super) fn encode_gif(run: &Path, out: &Path) -> Result<serde_json::Value, Failure> {
    let pattern = run.join("frame-%05d.png");
    let pattern = pattern.to_str().ok_or("frame pattern path")?;
    let temp = run.join("timeline.gif");
    let scale = "scale=960:-2:flags=lanczos";
    ffmpeg(&[
        "-framerate",
        "10",
        "-i",
        pattern,
        "-frames:v",
        "80",
        "-vf",
        scale,
        "-loop",
        "0",
        &temp.to_string_lossy(),
    ])?;
    let mut palette = false;
    if fs::metadata(&temp)?.len() > README_MAX_GIF {
        palette = true;
        let palette_file = run.join("palette.png");
        ffmpeg(&[
            "-framerate",
            "10",
            "-i",
            pattern,
            "-vf",
            &format!("{scale},palettegen=stats_mode=diff"),
            &palette_file.to_string_lossy(),
        ])?;
        ffmpeg(&[
            "-framerate",
            "10",
            "-i",
            pattern,
            "-i",
            &palette_file.to_string_lossy(),
            "-lavfi",
            &format!("{scale}[x];[x][1:v]paletteuse=dither=sierra2_4a"),
            "-loop",
            "0",
            &temp.to_string_lossy(),
        ])?;
    }
    let bytes = fs::metadata(&temp)?.len();
    if bytes > README_MAX_GIF {
        return Err(format!("gif exceeds 5,000,000 bytes: {bytes}").into());
    }
    let width = probe_field(&temp, "v:0", "width")?.parse::<u32>()?;
    let height = probe_field(&temp, "v:0", "height")?.parse::<u32>()?;
    let frames = decoded_frame_count(&temp)?;
    let (_, delay_error, end) = gif_timing(&temp)?;
    if (width, height) != (960, 600) || frames != README_GIF_FRAMES as u64 {
        return Err(format!("gif geometry/frames {width}x{height} {frames}").into());
    }
    if delay_error > 0.001 || (end - 8.0).abs() > 0.05 {
        return Err(format!("gif timing delay_error={delay_error} end={end}").into());
    }
    let decoded = decode_rgb(&temp, Some("select=eq(n\\,40)"))?;
    let source = decode_rgb(
        &run.join("boundary-2.png"),
        Some("scale=960:-2:flags=lanczos"),
    )?;
    let stats = image_stats(&decoded);
    if stats["spread"].as_u64().unwrap_or(0) < 16 {
        return Err("gif frame is blank".into());
    }
    let loss = difference(&source, &decoded);
    ffmpeg(&["-i", &temp.to_string_lossy(), "-f", "null", "-"])?;
    publish(&temp, out)?;
    Ok(serde_json::json!({
        "bytes": bytes,
        "width": width,
        "height": height,
        "frames": frames,
        "frame_delay_seconds": 0.1,
        "duration_seconds": end,
        "palette_optimized": palette,
        "max_delay_error": delay_error,
        "decoded_pixels": decoded.len() / 3,
        "stats": stats,
        "source_vs_decoded": loss
    }))
}
