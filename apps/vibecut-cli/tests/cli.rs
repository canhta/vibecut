#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use time::{Tick, TimeRange};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vibecut")
}
fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}
fn cli(args: &[&OsStr]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}
fn spawn(args: &[&OsStr]) -> std::process::Child {
    Command::new(bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}
fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}
fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}
fn error_json(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON error on stderr: {text}"));
    serde_json::from_str(line).unwrap()
}
fn new_project(path: &Path) -> Value {
    let output = cli(&[os("project"), os("new"), path.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}
fn add_track(project: &Path, name: &str) -> Value {
    let params = json!({"name": name, "kind": "video"}).to_string();
    let output = cli(&[
        os("exec"),
        project.as_os_str(),
        os("add_track"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}
fn info(project: &Path) -> Value {
    let output = cli(&[os("project"), os("info"), project.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}
fn overlay_project(project_path: &Path, width: u32, height: u32) {
    let mut data = project::Project::empty(project::ProjectId(0), "overlay".into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.timeline.tracks.push(project::Track {
        id: project::TrackId(0),
        name: "Overlay".into(),
        kind: project::TrackKind::Overlay,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([30, 160, 90, 255]),
            }),
        }],
    });
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}

fn tools() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        Command::new(tool)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}
fn ffmpeg(args: &[&OsStr]) {
    let status = Command::new("ffmpeg")
        .arg("-v")
        .arg("error")
        .arg("-y")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}
fn generate_av(path: &Path) {
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("testsrc2=size=128x96:rate=30:duration=1"),
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("sine=frequency=997:sample_rate=44100:duration=1"),
        os("-c:v"),
        os("libx264"),
        os("-pix_fmt"),
        os("yuv420p"),
        os("-g"),
        os("30"),
        os("-color_primaries"),
        os("bt709"),
        os("-color_trc"),
        os("bt709"),
        os("-colorspace"),
        os("bt709"),
        os("-c:a"),
        os("aac"),
        os("-ac"),
        os("1"),
        os("-shortest"),
        path.as_os_str(),
    ]);
}
fn media_project(project_path: &Path, video: &Path, audio: Option<&Path>) {
    let video_asset = media::SourceAsset::import(media::AssetId(1), video.to_owned()).unwrap();
    let stream = video_asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())
        .unwrap();
    let fps = video_asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
        .unwrap();
    let duration = video_asset.probe().duration.unwrap();
    let mut data = project::Project::empty(project::ProjectId(0), "media".into())
        .unwrap()
        .into_data();
    data.canvas.width = stream.width;
    data.canvas.height = stream.height;
    data.canvas.fps = fps;
    data.assets.push(video_asset);
    data.timeline.tracks.push(project::Track {
        id: project::TrackId(0),
        name: "Video".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Video(project::VideoParams {
                source_audio_enabled: false,
            }),
        }],
    });
    if let Some(audio) = audio {
        let audio_asset = media::SourceAsset::import(media::AssetId(2), audio.to_owned()).unwrap();
        let audio_duration = audio_asset.probe().duration.unwrap();
        data.assets.push(audio_asset);
        data.timeline.tracks.push(project::Track {
            id: project::TrackId(1),
            name: "Audio".into(),
            kind: project::TrackKind::Audio,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![project::Item {
                animations: Default::default(),
                id: project::ItemId(1),
                source_asset: Some(media::AssetId(2)),
                source_range: TimeRange::new(Tick::ZERO, audio_duration).unwrap(),
                start: Tick::ZERO,
                content: project::ItemParams::Audio(project::AudioParams {
                    gain_db: 0.0,
                    muted: false,
                }),
            }],
        });
    }
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}
fn generate_audio(path: &Path) {
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
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["tracks"], 0);

    let second = cli(&[os("project"), os("new"), project.as_os_str()]);
    assert_eq!(code(&second), 2);
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
}

#[test]
fn exec_mutates_through_engine_and_reports_created_ids() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("exec.vibecut");
    new_project(&project);
    let outcome = add_track(&project, "Video");
    assert_eq!(outcome["changed"], true);
    assert_eq!(outcome["created"][0]["track"], 0);
    let value = info(&project);
    assert_eq!(value["tracks"], 1);
    assert_eq!(value["items"], 0);
}

#[test]
fn exec_refuses_bad_input_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("bad.vibecut");
    new_project(&project);
    let before = fs::read(&project).unwrap();

    let unknown = cli(&[
        os("exec"),
        project.as_os_str(),
        os("no_such_command"),
        os("{}"),
        os("--json"),
    ]);
    assert_eq!(code(&unknown), 2);
    assert_eq!(error_json(&unknown)["error"]["code"], "usage");

    let invalid = cli(&[
        os("exec"),
        project.as_os_str(),
        os("add_track"),
        os("{not json"),
        os("--json"),
    ]);
    assert_eq!(code(&invalid), 2);
    assert_eq!(error_json(&invalid)["error"]["code"], "usage");

    let params = cli(&[
        os("exec"),
        project.as_os_str(),
        os("add_track"),
        os("{}"),
        os("--json"),
    ]);
    assert_eq!(code(&params), 2);
    assert_eq!(error_json(&params)["error"]["code"], "usage");
    assert_eq!(fs::read(&project).unwrap(), before);
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
        os("add_track"),
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
            os("add_track"),
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
            os("add_track"),
            OsStr::new(&params),
            os("--json"),
        ]);
        assert_eq!(code(&linked), 3);
    }

    drop(lease);

    let value = info(&project);
    assert_eq!(value["tracks"], 0);
}

#[test]
fn two_subprocesses_interleave_one_commit_one_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vibecut");
    let mut data = project::Project::empty(project::ProjectId(0), "base".into())
        .unwrap()
        .into_data();
    data.timeline.tracks = (0..250)
        .map(|index| project::Track {
            id: project::TrackId(index),
            name: format!("Base {index}").into(),
            kind: project::TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: Vec::new(),
        })
        .collect();
    data.bookmarks = (0..10_000)
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
            os("add_track"),
            OsStr::new(&params_a),
        ]);
        let second = Command::new(bin())
            .args([
                os("exec"),
                trial.as_os_str(),
                os("add_track"),
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
        let tracks = &loaded.project.data().timeline.tracks;
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
fn commands_prints_the_generated_registry() {
    let output = cli(&[os("commands"), os("--json")]);
    assert_eq!(code(&output), 0);
    let value = stdout_json(&output);
    let entries = value.as_array().unwrap();
    let add_track = entries
        .iter()
        .find(|entry| entry["id"] == "add_track")
        .unwrap();
    assert_eq!(add_track["params_schema"][0]["name"], "name");
    assert!(entries.iter().any(|entry| entry["id"] == "batch"));
}

#[test]
fn probe_reports_decoded_stream_metadata() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("probe source.mp4");
    generate_av(&video);
    let output = cli(&[os("probe"), video.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    assert_eq!(value["format"], "mp4");
    assert!(value["duration"]["ticks"].as_i64().unwrap() > 0);
    assert!(
        value["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["kind"] == "video" && stream["video"]["width"] == 128)
    );
    assert!(
        value["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["kind"] == "audio")
    );
}

#[test]
fn export_writes_a_completed_file_and_keeps_progress_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("overlay.vibecut");
    let output_path = dir.path().join("export.mp4");
    overlay_project(&project, 64, 64);
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
    assert!(fs::metadata(&output_path).unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("progress"));
}

#[test]
fn export_aspect_selects_portrait_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("aspect.vibecut");
    let output_path = dir.path().join("portrait.mp4");
    overlay_project(&project, 64, 64);
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
        let probe = Command::new("ffprobe")
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

#[test]
fn export_refuses_when_a_source_disappears_mid_job() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("midjob.mp4");
    let project = dir.path().join("midjob.vibecut");
    let output_path = dir.path().join("midjob-out.mp4");
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("testsrc2=size=640x360:rate=30:duration=3"),
        os("-c:v"),
        os("libx264"),
        os("-pix_fmt"),
        os("yuv420p"),
        os("-color_primaries"),
        os("bt709"),
        os("-color_trc"),
        os("bt709"),
        os("-colorspace"),
        os("bt709"),
        video.as_os_str(),
    ]);
    media_project(&project, &video, None);

    let mut child = Command::new(bin())
        .args([
            os("export"),
            project.as_os_str(),
            os("--out"),
            output_path.as_os_str(),
            os("--json"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut removed = false;
    for line in BufReader::new(stderr).lines() {
        let line = line.unwrap();
        if !removed && line.contains("progress render") {
            fs::remove_file(&video).unwrap();
            removed = true;
        }
    }
    let status = child.wait().unwrap();
    assert!(
        removed,
        "render progress was not observed before completion"
    );
    assert_eq!(status.code(), Some(4));
    assert!(!output_path.exists());
}
