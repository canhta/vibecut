//! One reproducible input recipe with observed backend speed and quality.
use crate::ffmpeg;
use std::{fs, path::Path, process::Command};
fn run(command: &mut Command) -> Result<(), String> {
    let output = command.output().map_err(|e| e.to_string())?;
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    print!("{}", String::from_utf8_lossy(&output.stdout));
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{command:?}: {}", output.status))
    }
}
pub fn media(root: &Path) -> Result<(), String> {
    let dir = root.join("target/fixtures/media/benchmark");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let input = dir.join("source-1080p60s.mp4");
    let output = dir.join("export-1080p60s.mp4");
    if !input.exists() {
        run(Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=30:duration=60",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
            ])
            .arg(&input))?;
    }
    run(Command::new("ffmpeg").arg("-version"))?;
    let mut command = Command::new(std::env::var_os("CARGO").ok_or("run via cargo xtask")?);
    command
        .current_dir(root)
        .args([
            "run",
            "--release",
            "-p",
            "media",
            "--example",
            "benchmark",
            "--",
        ])
        .arg(&input)
        .arg(&output);
    ffmpeg::environment(&mut command)?;
    run(&mut command)?;
    for metric in ["ssim", "psnr"] {
        run(Command::new("ffmpeg")
            .args(["-hide_banner", "-i"])
            .arg(&output)
            .arg("-i")
            .arg(&input)
            .args(["-lavfi", metric, "-an", "-f", "null", "-"]))?;
    }
    Ok(())
}
