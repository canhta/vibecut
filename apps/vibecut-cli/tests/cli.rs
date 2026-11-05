#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use time::{Tick, TimeRange};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vibecut-cli")
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
        os("track.add"),
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
fn overlay_project(project_path: &Path, width: u32, height: u32, duration: Tick) {
    overlay_project_color(project_path, width, height, duration, [30, 160, 90, 255]);
}

fn overlay_project_color(
    project_path: &Path,
    width: u32,
    height: u32,
    duration: Tick,
    color: [u8; 4],
) {
    let mut data = project::Project::empty(project::ProjectId(0), "overlay".into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Overlay".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color(color),
            }),
        }],
    });
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}

fn exec(project: &Path, command: &str, params: Value) -> Value {
    let params = params.to_string();
    let output = cli(&[
        os("exec"),
        project.as_os_str(),
        OsStr::new(command),
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

fn decoded_rgb(path: &Path, width: u32, height: u32, frames: &[usize]) -> Vec<Vec<u8>> {
    let select = frames
        .iter()
        .map(|frame| format!("eq(n\\,{frame})"))
        .collect::<Vec<_>>()
        .join("+");
    let output = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("select='{select}'"),
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = width as usize * height as usize * 3;
    assert_eq!(output.stdout.len(), bytes * frames.len());
    output.stdout.chunks(bytes).map(<[u8]>::to_vec).collect()
}

/// BT.709 OETF used by render's opaque video packing.
fn video_oetf(value: f64) -> f64 {
    if value < 0.018 {
        4.5 * value
    } else {
        1.099 * value.powf(0.45) - 0.099
    }
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
    assert_eq!(value["schema_version"], project::SCHEMA_VERSION);
    assert_eq!(value["tracks"], 1);

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
    assert_eq!(outcome["created"][0]["track"], 1);
    let value = info(&project);
    assert_eq!(value["tracks"], 2);
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
        os("track.add"),
        os("{not json"),
        os("--json"),
    ]);
    assert_eq!(code(&invalid), 2);
    assert_eq!(error_json(&invalid)["error"]["code"], "usage");

    let params = cli(&[
        os("exec"),
        project.as_os_str(),
        os("track.add"),
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
        os("track.add"),
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
            os("track.add"),
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
            os("track.add"),
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
    data.timeline.overlays = (0..250)
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
            os("track.add"),
            OsStr::new(&params_a),
        ]);
        let second = Command::new(bin())
            .args([
                os("exec"),
                trial.as_os_str(),
                os("track.add"),
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
        let tracks: Vec<_> = loaded.project.data().timeline.iter().collect();
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
        .find(|entry| entry["id"] == "track.add")
        .unwrap();
    assert_eq!(
        add_track["params_schema"]["$defs"]["NfcText"]["type"],
        "string"
    );
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
    assert!(
        value["format"]
            .as_str()
            .unwrap()
            .split(',')
            .any(|format| format == "mp4")
    );
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
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
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
fn cli_animates_opacity_and_exported_frames_follow_the_curve() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("animated.vibecut");
    let output_path = dir.path().join("animated.mp4");
    // The animation covers one second; the extra 0.2 s makes the 1.0 s key
    // land exactly on an exported frame.
    let duration = Tick(time::TICKS_PER_SECOND * 6 / 5);
    overlay_project_color(&project, 64, 64, duration, [255, 255, 255, 255]);
    let key = |time: i64, value: f64| {
        json!({
            "time": time,
            "value": {"kind": "scalar", "value": value},
            "segment": "linear",
            "tangent": "flat",
            "incoming": null,
            "outgoing": null,
        })
    };
    exec(
        &project,
        "keyframes.set_param",
        json!({
            "item": 0,
            "path": "opacity",
            "param": {
                "kind": "value",
                "channels": {
                    "value": {"kind": "scalar", "value": 0.0},
                    "keys": [key(0, 0.0)],
                    "before": "hold",
                    "after": "hold",
                },
            },
        }),
    );
    exec(
        &project,
        "keyframes.add",
        json!({
            "item": 0,
            "path": "opacity",
            "component": null,
            "key": key(time::TICKS_PER_SECOND, 1.0),
        }),
    );
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
    if !tools() {
        return;
    }
    let frames = decoded_rgb(&output_path, 64, 64, &[0, 15, 30]);
    for (frame, opacity) in frames.iter().zip([0.0, 0.5, 1.0]) {
        let expected = (video_oetf(opacity) * 255.0).round() as i32;
        for channel in &frame[..3] {
            assert!(
                (i32::from(*channel) - expected).abs() <= 12,
                "decoded {channel} != expected {expected} for opacity {opacity}"
            );
        }
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
fn providers_list_is_offline_and_capability_filtered() {
    let output = cli(&[
        os("providers"),
        os("--operation"),
        os("transcribe"),
        os("--language"),
        os("vi"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    let providers = value["providers"].as_array().unwrap();
    assert!(providers.iter().any(|value| value["id"] == "whisper.cpp"));
    assert!(providers.iter().any(|value| value["id"] == "gipformer"));
    assert!(providers.iter().any(|value| value["id"] == "openai"));
    assert!(providers.iter().any(|value| value["id"] == "elevenlabs"));
    let english = cli(&[
        os("providers"),
        os("--operation"),
        os("transcribe"),
        os("--language"),
        os("en"),
        os("--json"),
    ]);
    assert_eq!(code(&english), 0);
    let value = stdout_json(&english);
    assert!(
        !value["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value["id"] == "gipformer")
    );
}

#[test]
fn model_catalog_is_offline_and_download_requires_explicit_consent() {
    let list = cli(&[os("model"), os("list"), os("--json")]);
    assert_eq!(code(&list), 0, "{}", String::from_utf8_lossy(&list.stderr));
    let value = stdout_json(&list);
    let models = value["models"].as_array().unwrap();
    assert!(
        models
            .iter()
            .any(|model| model["id"] == "whisper-large-v3-turbo")
    );
    assert!(
        models
            .iter()
            .any(|model| model["id"] == "whisper-small-dtw")
    );
    let info = cli(&[
        os("model"),
        os("info"),
        os("whisper-large-v3-turbo"),
        os("--json"),
    ]);
    assert_eq!(code(&info), 0);
    let value = stdout_json(&info);
    assert!(
        value["license_url"]
            .as_str()
            .is_some_and(|v| v.starts_with("https://"))
    );
    assert!(!value["files"].as_array().unwrap().is_empty());
    let refused = cli(&[
        os("model"),
        os("download"),
        os("whisper-large-v3-turbo"),
        os("--json"),
    ]);
    assert_eq!(code(&refused), 5);
    let error = error_json(&refused);
    assert_eq!(error["error"]["code"], "rejected");
}

#[test]
fn subtitle_refuses_missing_output_and_unknown_language_without_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.mp4");
    fs::write(&input, b"not opened before argument validation").unwrap();
    let missing = cli(&[
        os("subtitle"),
        input.as_os_str(),
        os("--to"),
        os("en"),
        os("--json"),
    ]);
    assert_eq!(code(&missing), 2);
    assert_eq!(error_json(&missing)["error"]["code"], "usage");
    let language = cli(&[
        os("subtitle"),
        input.as_os_str(),
        os("--from"),
        os("fr"),
        os("--to"),
        os("en"),
        os("--srt"),
        directory.path().join("out.srt").as_os_str(),
        os("--json"),
    ]);
    // The CLI restricts source languages to the admitted vi/en/zh set.
    assert_eq!(code(&language), 2);
    assert_eq!(error_json(&language)["error"]["code"], "usage");
}

#[test]
fn subtitle_remote_provider_without_key_refuses_before_upload() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.mp4");
    fs::write(&input, b"never read").unwrap();
    // A parallel CLI run must not hold this credential check's cache lease.
    let output = Command::new(bin())
        .env("HOME", directory.path())
        .env("LOCALAPPDATA", directory.path())
        .env_remove("OPENAI_API_KEY")
        .args([
            os("subtitle"),
            input.as_os_str(),
            os("--from"),
            os("en"),
            os("--to"),
            os("vi"),
            os("--provider"),
            os("openai"),
            os("--srt"),
            directory.path().join("out.srt").as_os_str(),
            os("--yes-upload"),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        4,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "setup");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("OPENAI_API_KEY")
    );
}

#[test]
fn auto_detection_does_not_require_cloud_translation_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let input = directory.path().join("input.mp4");
    fs::write(&input, b"not read before model setup").unwrap();
    let output = Command::new(bin())
        .args([
            os("subtitle"),
            input.as_os_str(),
            os("--from"),
            os("auto"),
            os("--to"),
            os("en"),
            os("--provider"),
            os("whisper.cpp"),
            os("--srt"),
            directory.path().join("out.srt").as_os_str(),
            os("--json"),
        ])
        .env("HOME", &home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ELEVENLABS_API_KEY")
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        4,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "setup");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("whisper-large-v3-turbo"), "{message}");
    assert!(!message.contains("OPENAI_API_KEY"), "{message}");
    assert!(!message.contains("ANTHROPIC_API_KEY"), "{message}");
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

#[test]
fn cli_blur_executes_and_export_encodes_instead_of_copying() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("input.mp4");
    let project = dir.path().join("blur.vibecut");
    let output_path = dir.path().join("blur.mp4");
    generate_av(&video);
    media_project(&project, &video, None);
    exec(
        &project,
        "effects.add",
        json!({"item":0,"effect":{"id":"blur","enabled":true,"params":{"kind":"blur","intensity":100.0}}}),
    );
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
    let source = decoded_rgb(&video, 128, 96, &[0]);
    let actual = decoded_rgb(&output_path, 128, 96, &[0]);
    let difference: u64 = source[0]
        .iter()
        .zip(&actual[0])
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    assert!(difference > 10000, "blur was bypassed: {difference}");
}

#[test]
fn exec_shared_field_id_list_persists_only_compatible_item_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Lựa chọn.json");
    let mut data = project::Project::empty(project::ProjectId(0), "Batch".into())
        .unwrap()
        .into_data();
    data.timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: (0..2)
            .map(|i| project::Item {
                effects: vec![],
                animations: Default::default(),
                id: project::ItemId(i + 10),
                source_asset: None,
                source_range: TimeRange::new(Tick::ZERO, Tick(100)).unwrap(),
                start: Tick(i as i64 * 100),
                content: project::ItemParams::Text(project::TextParams {
                    content: format!("Text {i}").into(),
                    font_size: 24.0 + i as f32,
                    color: project::Color([255; 4]),
                }),
            })
            .collect(),
    });
    let original = project::Project::new(data).unwrap();
    fs::write(&path, project::encode(&original, true).unwrap()).unwrap();
    let result = exec(
        &path,
        "timeline.set_item_param_many",
        json!({"items":[10,11],"patch":{"field":"text_size","value":60}}),
    );
    assert_eq!(result["changed"], true);
    let saved = project::decode(&fs::read(&path).unwrap()).unwrap().project;
    for (before, after) in original
        .data()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .zip(saved.data().timeline.iter().flat_map(|t| &t.items))
    {
        let project::ItemParams::Text(p) = &after.content else {
            panic!("text")
        };
        let project::ItemParams::Text(old) = &before.content else {
            panic!("text")
        };
        assert_eq!(p.font_size, 60.0);
        assert_eq!(p.content, old.content);
        assert_eq!(p.color, old.color);
    }
    let before = fs::read(&path).unwrap();
    let result = exec(
        &path,
        "timeline.set_item_param_many",
        json!({"items":[10,11],"patch":{"field":"audio_gain","value":-6}}),
    );
    assert_eq!(result["changed"], false);
    assert_eq!(fs::read(&path).unwrap(), before);
}
