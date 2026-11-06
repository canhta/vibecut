//! Discovers and drives the external FFmpeg/FFprobe oracles the tests compare against.
use crate::fixture;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Mutex,
};

/// A tool command with the production runtime search path removed so the
/// external oracle's own libraries win.
pub fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    if let (Some(prefix), Some(paths)) = (std::env::var_os("FFMPEG_DIR"), std::env::var_os("PATH"))
    {
        let production = PathBuf::from(prefix).join("bin");
        let paths: Vec<_> = std::env::split_paths(&paths)
            .filter(|path| path != &production)
            .collect();
        command.env("PATH", std::env::join_paths(paths).expect("oracle paths"));
    }
    command
}

/// True when both external tools run; a found tool that fails is a failure.
pub fn available() -> bool {
    ["ffmpeg", "ffprobe"]
        .iter()
        .all(|tool| match command(tool).arg("-version").output() {
            Ok(output) => {
                assert_success(&output, &format!("{tool} -version"));
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("cannot run {tool}: {error}"),
        })
}

/// Skip with a message when the oracles are absent, unless the environment
/// demands them, in which case this fails.
pub fn require(test: &str) -> bool {
    if available() {
        return true;
    }
    assert_ne!(
        std::env::var("VIBECUT_REQUIRE_ORACLES").as_deref(),
        Ok("1"),
        "{test}: VIBECUT_REQUIRE_ORACLES=1 but ffmpeg/ffprobe are missing"
    );
    eprintln!("skipping {test}: ffmpeg/ffprobe not found");
    false
}

/// Generates `name` in `dir` unless a non-empty fixture already exists.
/// Publication is atomic, so a concurrent reader never sees a partial file.
pub fn generate(dir: &Path, test: &str, name: &str, args: &[&str]) -> Option<PathBuf> {
    if !require(test) {
        return None;
    }
    static GENERATION: Mutex<()> = Mutex::new(());
    let _generation = GENERATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = dir.join(name);
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0) {
        return Some(path);
    }
    let path = fixture::publish(dir, name, |temp| {
        let output = command("ffmpeg")
            .args(["-y", "-hide_banner", "-loglevel", "error"])
            .args(args)
            .arg(temp)
            .output()
            .expect("run ffmpeg");
        assert_success(&output, &format!("ffmpeg fixture {name}"));
    });
    Some(path)
}

/// Runs ffmpeg with the common quiet flags and fails with its diagnostics.
pub fn ffmpeg(args: &[&OsStr]) {
    let output = command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("run ffmpeg");
    assert_success(&output, "ffmpeg");
}

/// One stream field (for example `codec_name` or `width`) of a selected stream.
pub fn stream_field(path: &Path, select: &str, entry: &str) -> String {
    ffprobe(
        path,
        &[
            "-select_streams",
            select,
            "-show_entries",
            &format!("stream={entry}"),
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ],
    )
}

/// Decoded sample count of a selected stream.
pub fn decoded_frames(path: &Path, select: &str) -> u64 {
    ffprobe(
        path,
        &[
            "-select_streams",
            select,
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ],
    )
    .lines()
    .next()
    .and_then(|value| value.trim().parse().ok())
    .unwrap_or(0)
}

/// Container duration in seconds.
pub fn duration(path: &Path) -> f64 {
    ffprobe(
        path,
        &[
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ],
    )
    .parse()
    .expect("duration")
}

/// True when ffmpeg decodes every stream without error.
pub fn decodes(path: &Path) -> bool {
    let output = command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .expect("run ffmpeg decode");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_success(&output, "ffmpeg decode");
    stderr.trim().is_empty()
}

/// Full ffprobe stream and format description as JSON.
pub fn probe(path: &Path) -> serde_json::Value {
    let output = command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert_success(&output, "ffprobe");
    serde_json::from_slice(&output.stdout).expect("ffprobe json")
}

/// First decoded RGBA frame of `path`.
pub fn rgba(path: &Path) -> Vec<u8> {
    let output = command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .expect("run ffmpeg rgba");
    assert_success(&output, "ffmpeg rgba");
    output.stdout
}

pub fn approx(observed: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (observed - expected).abs() <= tolerance,
        "{what}: observed {observed}, expected {expected} +/- {tolerance}"
    );
}

fn ffprobe(path: &Path, args: &[&str]) -> String {
    let output = command("ffprobe")
        .args(["-v", "error"])
        .args(args)
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert_success(&output, "ffprobe");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn assert_success(output: &Output, context: &str) {
    const DIAGNOSTIC_BYTES: usize = 4096;
    assert!(
        output.status.success(),
        "{context} failed: status={}; stdout (first {} of {} bytes)={:?}; stderr (first {} of {} bytes)={:?}",
        output.status,
        output.stdout.len().min(DIAGNOSTIC_BYTES),
        output.stdout.len(),
        String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(DIAGNOSTIC_BYTES)]),
        output.stderr.len().min(DIAGNOSTIC_BYTES),
        output.stderr.len(),
        String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(DIAGNOSTIC_BYTES)])
    );
}
