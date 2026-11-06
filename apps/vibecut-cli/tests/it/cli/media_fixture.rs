use super::process::os;
use crate::common::{ffmpeg, tool_command};
use std::path::Path;

pub(super) fn decoded_rgb(path: &Path, width: u32, height: u32, frames: &[usize]) -> Vec<Vec<u8>> {
    let select = frames
        .iter()
        .map(|frame| format!("eq(n\\,{frame})"))
        .collect::<Vec<_>>()
        .join("+");
    let output = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("select='{select}'"),
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = width as usize * height as usize * 3;
    assert_eq!(output.stdout.len(), bytes * frames.len());
    output.stdout.chunks(bytes).map(<[u8]>::to_vec).collect()
}

/// BT.709 OETF used by render's opaque video packing.
pub(super) fn video_oetf(value: f64) -> f64 {
    if value < 0.018 {
        4.5 * value
    } else {
        1.099 * value.powf(0.45) - 0.099
    }
}

pub(super) fn generate_audio(path: &Path) {
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("sine=frequency=440:sample_rate=48000:duration=1"),
        os("-c:a"),
        os("pcm_f32le"),
        path.as_os_str(),
    ]);
}

pub(super) fn decoded_mono(path: &Path) -> Vec<f32> {
    let output = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "f32le", "-ac", "1", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}
