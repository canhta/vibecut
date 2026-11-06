use super::process::{bin, code, os, stdout_json};
use crate::common::{ffmpeg, tools};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

fn scene_clip(path: &Path, filters: &str) {
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("color=c=black:s=64x64:r=10:d=1"),
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("color=c=white:s=64x64:r=10:d=1"),
        os("-filter_complex"),
        os(filters),
        os("-c:v"),
        os("ffv1"),
        path.as_os_str(),
    ]);
}

#[test]
fn scenes_reports_the_hard_cut_and_honours_min_scene() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("cut.mkv");
    scene_clip(&clip, "[0:v][1:v]concat=n=2:v=1:a=0");
    let run = |extra: &[&OsStr]| {
        let mut args = vec![os("scenes"), clip.as_os_str(), os("--json")];
        args.extend_from_slice(extra);
        // Keep the artifact cache out of the real user cache.
        let output = Command::new(bin())
            .args(&args)
            .env("HOME", dir.path())
            .env("LOCALAPPDATA", dir.path())
            .env("XDG_CACHE_HOME", dir.path())
            .output()
            .unwrap();
        assert_eq!(
            code(&output),
            0,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout_json(&output)
    };
    let found = run(&[]);
    let cuts = found["candidates"].as_array().unwrap();
    assert_eq!(cuts.len(), 1, "{found}");
    assert_eq!(cuts[0]["at"], time::TICKS_PER_SECOND);
    assert_eq!(cuts[0]["kind"], "hard_cut");
    assert_eq!(found["decoded_frames"], 20);
    // A warm second run matches; a 2 s minimum scene length leaves the 1 s cut out.
    assert_eq!(run(&[]), found);
    let strict = run(&[os("--min-scene-ticks"), os("508032000000")]);
    assert!(strict["candidates"].as_array().unwrap().is_empty());
}
