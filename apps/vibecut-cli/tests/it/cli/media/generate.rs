//! Verifies native media generation commands.
use crate::cli::process::{bin, code, os, stdout_json};
use crate::media_fixture::{generate_av, media_project, tools};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Runs one command with every cache location redirected into the fixture.
fn run(dir: &Path, args: &[&OsStr]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .env("HOME", dir)
        .env("LOCALAPPDATA", dir)
        .env("XDG_CACHE_HOME", dir)
        .output()
        .unwrap()
}

#[test]
fn media_generation_warms_waveform_and_thumbnail_caches() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("clip.mp4");
    generate_av(&media, 3);
    let project = dir.path().join("media.vibecut");
    media_project(&project, &media, None);

    let waveform = run(
        dir.path(),
        &[
            os("media"),
            os("waveform"),
            project.as_os_str(),
            os("--asset"),
            os("1"),
            os("--json"),
        ],
    );
    assert_eq!(
        code(&waveform),
        0,
        "{}",
        String::from_utf8_lossy(&waveform.stderr)
    );
    let summary = stdout_json(&waveform);
    assert_eq!(summary["asset"], 1);
    assert_eq!(summary["sample_rate"], 44_100);
    assert_eq!(summary["channels"], 1);
    assert!(summary["buckets"].as_u64().unwrap() > 0, "{summary}");

    let thumbnails = run(
        dir.path(),
        &[
            os("media"),
            os("thumbnails"),
            project.as_os_str(),
            os("--item"),
            os("0"),
            os("--pixels-per-second"),
            os("71.5"),
            os("--json"),
        ],
    );
    assert_eq!(
        code(&thumbnails),
        0,
        "{}",
        String::from_utf8_lossy(&thumbnails.stderr)
    );
    let strip = stdout_json(&thumbnails);
    assert_eq!(strip["item"], 0);
    assert_eq!(strip["pixels_per_second"], 71.5);
    assert!(
        !strip["positions"].as_array().unwrap().is_empty(),
        "{strip}"
    );

    // Both caches answer a second run with the source gone.
    std::fs::remove_file(&media).unwrap();
    for args in [
        vec![
            os("media"),
            os("waveform"),
            project.as_os_str(),
            os("--asset"),
            os("1"),
            os("--json"),
        ],
        vec![
            os("media"),
            os("thumbnails"),
            project.as_os_str(),
            os("--item"),
            os("0"),
            os("--pixels-per-second"),
            os("71.5"),
            os("--json"),
        ],
    ] {
        let second = run(dir.path(), &args);
        assert_eq!(
            code(&second),
            0,
            "{}",
            String::from_utf8_lossy(&second.stderr)
        );
    }

    // A missing asset and an unbounded range are typed refusals.
    let missing = run(
        dir.path(),
        &[
            os("media"),
            os("waveform"),
            project.as_os_str(),
            os("--asset"),
            os("99"),
            os("--json"),
        ],
    );
    assert_eq!(code(&missing), 5);
    let missing_zoom = run(
        dir.path(),
        &[
            os("media"),
            os("thumbnails"),
            project.as_os_str(),
            os("--item"),
            os("0"),
            os("--json"),
        ],
    );
    assert_eq!(
        code(&missing_zoom),
        2,
        "{}",
        String::from_utf8_lossy(&missing_zoom.stderr)
    );
    let reversed = run(
        dir.path(),
        &[
            os("media"),
            os("thumbnails"),
            project.as_os_str(),
            os("--item"),
            os("0"),
            os("--pixels-per-second"),
            os("71.5"),
            os("--start"),
            os("254016000001"),
            os("--end"),
            os("254016000000"),
            os("--json"),
        ],
    );
    assert_eq!(
        code(&reversed),
        5,
        "{}",
        String::from_utf8_lossy(&reversed.stderr)
    );
}

#[test]
fn unsafe_cache_lock_reports_exact_process_diagnostics_without_publication() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("private-source.mp4");
    generate_av(&media, 1);
    let project = dir.path().join("private-project.vibecut");
    media_project(&project, &media, None);
    // Match CacheRoot's platform-selected base under the child's isolated env.
    let base = if cfg!(target_os = "macos") {
        dir.path().join("Library/Caches")
    } else {
        dir.path().to_path_buf()
    };
    let root = base.join("VibeCut/models/waveform");
    let lock = root.join(".cache.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let source_bytes = std::fs::read(&media).unwrap();
    let project_bytes = std::fs::read(&project).unwrap();
    for json in [false, true] {
        let mut args = vec![
            os("media"),
            os("waveform"),
            project.as_os_str(),
            os("--asset"),
            os("1"),
        ];
        if json {
            args.push(os("--json"));
        }
        let output = run(dir.path(), &args);
        assert_eq!(code(&output), 7);
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        if json {
            let value: serde_json::Value = serde_json::from_str(&stderr).unwrap();
            assert_eq!(
                value,
                serde_json::json!({"status":"error","error":{"code":"job","exit_code":7,"message":"corrupt cache entry skipped"}})
            );
        } else {
            assert_eq!(stderr.trim_end(), "error: corrupt cache entry skipped");
        }
        assert_eq!(std::fs::read(&media).unwrap(), source_bytes);
        assert_eq!(std::fs::read(&project).unwrap(), project_bytes);
        assert!(lock.is_dir());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
}
