//! Shared fixture generation and ffprobe helpers for oracle tests.

#![allow(
    dead_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    if let (Some(prefix), Some(paths)) = (std::env::var_os("FFMPEG_DIR"), std::env::var_os("PATH"))
    {
        let production = PathBuf::from(prefix).join("bin");
        let paths: Vec<_> = std::env::split_paths(&paths)
            .filter(|p| p != &production)
            .collect();
        command.env("PATH", std::env::join_paths(paths).expect("oracle paths"));
    }
    command
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/media")
}

fn tool_available(tool: &str) -> bool {
    match command(tool).arg("-version").output() {
        Ok(output) => {
            const DIAGNOSTIC_BYTES: usize = 4096;
            assert!(
                output.status.success(),
                "found {tool} but -version failed: status={}; stdout (first {} of {} bytes)={:?}; stderr (first {} of {} bytes)={:?}",
                output.status,
                output.stdout.len().min(DIAGNOSTIC_BYTES),
                output.stdout.len(),
                String::from_utf8_lossy(
                    &output.stdout[..output.stdout.len().min(DIAGNOSTIC_BYTES)]
                ),
                output.stderr.len().min(DIAGNOSTIC_BYTES),
                output.stderr.len(),
                String::from_utf8_lossy(
                    &output.stderr[..output.stderr.len().min(DIAGNOSTIC_BYTES)]
                )
            );
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!("cannot run {tool}: {error}"),
    }
}

/// Skip with a message when ffmpeg/ffprobe are absent, unless the environment
/// demands oracles, in which case this fails.
pub fn require_oracles(test: &str) -> bool {
    if tool_available("ffmpeg") && tool_available("ffprobe") {
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

/// Generate `name` with ffmpeg unless a non-empty file already exists.
pub fn generate(test: &str, name: &str, args: &[&str]) -> Option<PathBuf> {
    if !require_oracles(test) {
        return None;
    }
    // Parallel tests reuse filenames; readers must wait for the mux trailer.
    static GENERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _generation = GENERATION.lock().expect("fixture generation lock");
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).expect("fixture directory");
    let path = dir.join(name);
    let exists = std::fs::metadata(&path)
        .map(|meta| meta.len() > 0)
        .unwrap_or(false);
    if exists {
        return Some(path);
    }
    let output = command("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(args)
        .arg(&path)
        .output()
        .expect("run ffmpeg");
    assert!(
        output.status.success(),
        "ffmpeg failed for {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(path)
}

fn ffprobe(path: &Path, args: &[&str]) -> String {
    let output = command("ffprobe")
        .args(["-v", "error"])
        .args(args)
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(
        output.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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
    assert!(output.status.success(), "decode failed: {stderr}");
    stderr.trim().is_empty()
}

pub fn approx(observed: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (observed - expected).abs() <= tolerance,
        "{what}: observed {observed}, expected {expected} +/- {tolerance}"
    );
}
