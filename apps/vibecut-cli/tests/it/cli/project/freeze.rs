//! Verifies native freeze-frame and replace-media command behaviour.
use crate::cli::fixtures::{exec, new_project};
use crate::cli::process::{cli, code, os, stdout_json};
use crate::media_fixture::{generate_av, media_project, tools};
use serde_json::json;
use std::fs;

#[test]
fn freeze_requires_an_item_a_playhead_and_a_directory() {
    assert_eq!(code(&cli(&[os("freeze")])), 2);
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("freeze.vibecut");
    new_project(&project);
    let no_dir = cli(&[
        os("freeze"),
        project.as_os_str(),
        os("--item"),
        os("0"),
        os("--at"),
        os("1"),
    ]);
    assert_eq!(code(&no_dir), 2, "--dir is required");
}

#[test]
fn freeze_refuses_a_relative_directory_and_changes_nothing() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("freeze.mp4");
    generate_av(&video, 3);
    let project = dir.path().join("freeze.vibecut");
    new_project(&project);
    media_project(&project, &video, None);
    let before = fs::read(&project).unwrap();
    let output = cli(&[
        os("freeze"),
        project.as_os_str(),
        os("--item"),
        os("0"),
        os("--at"),
        os("1"),
        os("--dir"),
        os("relative-out"),
    ]);
    assert_ne!(code(&output), 0, "relative directory refuses");
    assert_eq!(fs::read(&project).unwrap(), before);
}

#[test]
fn freeze_holds_the_playhead_frame_and_reports_the_still() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("freeze.mp4");
    generate_av(&video, 3);
    let project = dir.path().join("freeze.vibecut");
    new_project(&project);
    media_project(&project, &video, None);
    let out = dir.path().join("stills");
    let output = cli(&[
        os("freeze"),
        project.as_os_str(),
        os("--item"),
        os("0"),
        os("--at"),
        os("1"),
        os("--duration"),
        os("2"),
        os("--dir"),
        out.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reported = stdout_json(&output);
    assert!(reported["still_item"].is_u64(), "{reported}");
    assert_eq!(reported["changed"], json!(true));
    let loaded = project::load(&project).unwrap().project;
    let items = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .count();
    assert_eq!(items, 3, "host splits into two halves around the still");
    let stills = fs::read_dir(&out)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("freeze-")
        })
        .count();
    assert_eq!(stills, 1, "one content-addressed still");
}

#[test]
fn replace_media_through_exec_accepts_a_registered_source_and_refuses_unknown_assets() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("replace.mp4");
    generate_av(&video, 3);
    let project = dir.path().join("replace.vibecut");
    new_project(&project);
    media_project(&project, &video, None);
    let replaced = exec(
        &project,
        "timeline.replace_media",
        json!({"items": [0], "asset": 1}),
    );
    assert!(replaced.is_object(), "{replaced}");
    let refused = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.replace_media"),
        os(r#"{"items":[0],"asset":99}"#),
        os("--json"),
    ]);
    assert_ne!(code(&refused), 0, "unknown asset refuses");
}
