//! Verifies native export command behaviour.
use crate::cli::fixtures::overlay_project;
use crate::cli::fixtures::{decoded_rgb, generate_audio};
use crate::cli::process::{cli, code, error_json, os, stdout_json};
use crate::media_fixture::{generate_av, media_project, tool_command, tools};
use serde_json::Value;
use std::fs;
use time::{Tick, TimeRange};

#[test]
fn export_writes_a_completed_file_and_keeps_progress_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("overlay.vibecut");
    let output_path = dir.path().join("export.mp4");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    // A longer red main scene must not leak into the selected green scene's export.
    let mut data = project::load(&project).unwrap().project.into_data();
    let mut secondary = data.scene().clone();
    secondary.id = project::SceneId(2);
    secondary.name = "Export scene".into();
    secondary.is_main = false;
    secondary.timeline.main.id = project::TrackId(3);
    secondary
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .content = project::ItemParams::Overlay(project::OverlayParams {
        color: project::Color([0, 255, 0, 255]),
    });
    secondary.timeline.overlays.first_mut().unwrap().id = project::TrackId(4);
    secondary
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .id = project::ItemId(5);
    let item = data
        .scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap();
    item.source_range = TimeRange::new(Tick::ZERO, Tick(2 * time::TICKS_PER_SECOND)).unwrap();
    item.content = project::ItemParams::Overlay(project::OverlayParams {
        color: project::Color([255, 0, 0, 255]),
    });
    data.scenes.insert(secondary).unwrap();
    data.current_scene_id = project::SceneId(2);
    fs::write(
        &project,
        project::encode(&project::Project::new(data).unwrap(), false).unwrap(),
    )
    .unwrap();
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_json(&output)["status"], "completed");
    assert_eq!(
        stdout_json(&output)["path"],
        output_path.to_string_lossy().as_ref()
    );
    assert!(fs::metadata(&output_path).unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("progress"));
    if tools() {
        let frame = decoded_rgb(&output_path, 64, 64, &[0]);
        let rgb = &frame.first().unwrap()[..3];
        for (actual, expected) in rgb.iter().zip([0, 255, 0]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 12,
                "decoded {rgb:?} expected green"
            );
        }
        let probe = tool_command("ffprobe")
            .args([
                os("-v"),
                os("error"),
                os("-show_entries"),
                os("stream=nb_frames"),
                os("-of"),
                os("json"),
                output_path.as_os_str(),
            ])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(value["streams"][0]["nb_frames"], "30");
    }
}

#[test]
fn export_aspect_selects_portrait_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("aspect.vibecut");
    let output_path = dir.path().join("portrait.mp4");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--aspect"),
        os("9:16"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if tools() {
        let probe = tool_command("ffprobe")
            .args([
                os("-v"),
                os("error"),
                os("-select_streams"),
                os("v:0"),
                os("-show_entries"),
                os("stream=width,height"),
                os("-of"),
                os("csv=p=0"),
                output_path.as_os_str(),
            ])
            .output()
            .unwrap();
        assert!(probe.status.success());
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1080,1920");
    }
}

#[test]
fn export_refuses_missing_changed_and_missing_audio_sources() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("audio.wav");
    let project = dir.path().join("media.vibecut");
    let output_path = dir.path().join("media.mp4");
    generate_av(&video);
    generate_audio(&audio);
    media_project(&project, &video, Some(&audio));
    let video_bytes = fs::read(&video).unwrap();

    fs::remove_file(&video).unwrap();
    let missing = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&missing), 4);
    let error = error_json(&missing);
    assert_eq!(error["error"]["code"], "unavailable");
    let unavailable = error["error"]["unavailable"].as_array().unwrap();
    assert_eq!(unavailable[0]["asset"], 1);
    assert_eq!(unavailable[0]["item"], 0);
    assert_eq!(unavailable[0]["reason"], "missing");
    assert!(!output_path.exists());

    fs::write(&video, &video_bytes).unwrap();
    let mut changed_bytes = video_bytes.clone();
    changed_bytes.extend_from_slice(b"changed content");
    fs::write(&video, changed_bytes).unwrap();
    let changed = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&changed), 4);
    let error = error_json(&changed);
    assert!(
        error["error"]["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["reason"] == "changed")
    );
    assert!(!output_path.exists());

    fs::write(&video, &video_bytes).unwrap();
    fs::remove_file(&audio).unwrap();
    let missing_audio = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&missing_audio), 4);
    let error = error_json(&missing_audio);
    let unavailable = error["error"]["unavailable"].as_array().unwrap();
    assert!(unavailable.iter().all(|entry| entry["asset"] == 2));
    assert!(!output_path.exists());
}

#[test]
fn export_allow_degraded_is_explicit_consent() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("degraded.mp4");
    let project = dir.path().join("degraded.vibecut");
    let output_path = dir.path().join("degraded-out.mp4");
    generate_av(&video);
    media_project(&project, &video, None);
    fs::remove_file(&video).unwrap();
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--allow-degraded"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::metadata(&output_path).unwrap().len() > 0);
}
