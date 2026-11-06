//! Verifies native project command behaviour.
use crate::cli::fixtures::{info, new_project};
use crate::cli::process::{bin, cli, code, error_json, os, spawn};
use serde_json::json;
use std::ffi::OsStr;
use std::fs;
use std::process::{Command, Stdio};
use time::Tick;

#[test]
fn project_new_and_info_round_trip_unicode_space_path() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("dự án có dấu cách.vibecut");
    let created = new_project(&project);
    assert_eq!(created["content_hash"].as_str().unwrap().len(), 64);
    let value = info(&project);
    assert_eq!(value["name"], "Untitled");
    assert_eq!(value["canvas"]["width"], 1920);
    assert_eq!(value["canvas"]["height"], 1080);
    assert_eq!(value["schema_version"], project::SCHEMA_VERSION);
    assert_eq!(value["tracks"], 1);

    let second = cli(&[os("project"), os("new"), project.as_os_str()]);
    assert_eq!(code(&second), 2);
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
}

#[test]
fn busy_writer_refuses_with_typed_exit_and_no_write() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("busy.vibecut");
    new_project(&project);
    let before = fs::read(&project).unwrap();
    let lease = project::WriterLease::acquire(&project).unwrap();

    let params = json!({"name": "Blocked", "kind": "video"}).to_string();
    let output = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.add_track"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_eq!(code(&output), 3);
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "busy");
    assert_eq!(error["error"]["exit_code"], 3);
    assert_eq!(fs::read(&project).unwrap(), before);

    let relative = Command::new(bin())
        .current_dir(dir.path())
        .args([
            os("exec"),
            os("busy.vibecut"),
            os("timeline.add_track"),
            OsStr::new(&params),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(code(&relative), 3);

    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.vibecut");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        let linked = cli(&[
            os("exec"),
            alias.as_os_str(),
            os("timeline.add_track"),
            OsStr::new(&params),
            os("--json"),
        ]);
        assert_eq!(code(&linked), 3);
    }

    drop(lease);

    let value = info(&project);
    assert_eq!(value["tracks"], 1);
}

#[test]
fn two_subprocesses_interleave_one_commit_one_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vibecut");
    let mut data = project::Project::empty(project::ProjectId(0), "base".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.overlays = (0..250)
        .map(|index| project::Track {
            id: project::TrackId(index + 1),
            name: format!("Base {index}").into(),
            kind: project::TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: Vec::new(),
        })
        .collect();
    data.scene_mut().bookmarks = (0..10_000)
        .map(|index| project::Bookmark {
            id: project::BookmarkId(index),
            start: Tick(index as i64),
            duration: Tick(1),
            note: format!("B{index}").into(),
            color: project::Color([0, 0, 0, 255]),
        })
        .collect();
    let base_project = project::Project::new(data).unwrap();
    fs::write(&base, project::encode(&base_project, true).unwrap()).unwrap();
    let base_tracks = info(&base)["tracks"].as_u64().unwrap();

    for attempt in 0..8 {
        let trial = dir.path().join(format!("trial-{attempt}.vibecut"));
        fs::copy(&base, &trial).unwrap();
        let params_a = json!({"name": "A", "kind": "video"}).to_string();
        let params_b = json!({"name": "B", "kind": "video"}).to_string();
        let first = spawn(&[
            os("exec"),
            trial.as_os_str(),
            os("timeline.add_track"),
            OsStr::new(&params_a),
        ]);
        let second = Command::new(bin())
            .args([
                os("exec"),
                trial.as_os_str(),
                os("timeline.add_track"),
                OsStr::new(&params_b),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let first_output = first.wait_with_output().unwrap();
        let second_output = second.wait_with_output().unwrap();
        let first_code = code(&first_output);
        let second_code = code(&second_output);
        if first_code == 0 && second_code == 0 {
            continue;
        }
        let mut codes = [first_code, second_code];
        codes.sort_unstable();
        assert_eq!(
            codes,
            [0, 3],
            "first: {}\nsecond: {}",
            String::from_utf8_lossy(&first_output.stderr),
            String::from_utf8_lossy(&second_output.stderr)
        );
        let loaded = project::load(&trial).unwrap();
        let tracks: Vec<_> = loaded.project.data().scene().timeline.iter().collect();
        assert_eq!(tracks.len() as u64, base_tracks + 1);
        let winner = if first_code == 0 { "A" } else { "B" };
        let loser = if first_code == 0 { "B" } else { "A" };
        assert!(tracks.iter().any(|track| track.name.as_str() == winner));
        assert!(!tracks.iter().any(|track| track.name.as_str() == loser));
        return;
    }
    panic!("no interleaved read-edit-save conflict observed in eight attempts");
}

#[test]
fn view_state_persists_in_optional_metadata_without_rewriting_project() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("view state.vibecut");
    new_project(&path);
    let before = fs::read(&path).unwrap();
    let params = json!({"zoom":2.5,"scroll":40,"playhead":254016000000_i64}).to_string();
    let result = cli(&[
        os("exec"),
        path.as_os_str(),
        os("project.set_view_state"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_eq!(
        code(&result),
        0,
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value = info(&path);
    assert_eq!(value["metadata"]["view"]["zoom"], 2.5);
    assert_eq!(value["metadata"]["view"]["scroll"], 40.0);
    assert_eq!(fs::read(&path).unwrap(), before);
    let lease = project::WriterLease::acquire(&path).unwrap();
    let busy = cli(&[
        os("exec"),
        path.as_os_str(),
        os("project.set_view_state"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_eq!(code(&busy), 3);
    assert_eq!(error_json(&busy)["error"]["code"], "busy");
    drop(lease);
}
