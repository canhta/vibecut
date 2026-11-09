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
