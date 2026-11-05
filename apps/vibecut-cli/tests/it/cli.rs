#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::common::*;
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
struct ControlSession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    stderr: std::process::ChildStderr,
}
impl ControlSession {
    fn start(project: &Path, extra: &[&OsStr]) -> Self {
        let mut child = Command::new(bin())
            .arg("control")
            .arg(project)
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let stderr = child.stderr.take().unwrap();
        Self {
            child,
            stdin,
            stdout,
            stderr,
        }
    }
    fn send(&mut self, request: Value) -> Value {
        self.send_raw(&request.to_string())
    }
    fn send_raw(&mut self, line: &str) -> Value {
        use std::io::Write as _;
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(&mut self.stdout, &mut line).unwrap();
        assert!(!line.is_empty(), "control session closed before a response");
        serde_json::from_str(&line).unwrap()
    }
    fn finish(mut self) -> (i32, String) {
        drop(self.stdin);
        let status = self.child.wait().unwrap();
        let mut stderr = String::new();
        use std::io::Read as _;
        self.stderr.read_to_string(&mut stderr).unwrap();
        (status.code().unwrap(), stderr)
    }
}
fn control_request(
    request_id: &str,
    command: &str,
    params: Value,
    base_rev: Option<&str>,
    key: Option<&str>,
) -> Value {
    let mut request = json!({
        "request_id": request_id,
        "command": command,
        "params": params,
    });
    if let Value::Object(fields) = &mut request {
        if let Some(base_rev) = base_rev {
            fields.insert("base_rev".into(), json!(base_rev));
        }
        if let Some(key) = key {
            fields.insert("idempotency_key".into(), json!(key));
        }
    }
    request
}
fn response_error(response: &Value) -> &str {
    response["value"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("not an engine error: {response}"))
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
    let params = json!({"name": name, "kind": "video", "placement":{"mode":"auto"}}).to_string();
    let output = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.add_track"),
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
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Overlay".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
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
    let placed = exec(
        &project,
        "timeline.insert",
        json!({
            "placement":{"mode":"auto"},
            "item":{"source_asset":null,"source_range":{"start":0,"end":100},"start":50,
                "animations":{},"content":{"kind":"overlay","params":{"color":[10,20,30,255]}}}
        }),
    );
    assert_eq!(placed["placements"][0]["track"], 0);
    assert_eq!(placed["placements"][0]["item"], 2);
    let loaded = project::load(&project).unwrap().project;
    assert_eq!(
        loaded.data().scene().timeline.main.items[0].start,
        Tick::ZERO
    );
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
        os("timeline.add_track"),
        os("{not json"),
        os("--json"),
    ]);
    assert_eq!(code(&invalid), 2);
    assert_eq!(error_json(&invalid)["error"]["code"], "usage");

    let params = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.add_track"),
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
fn commands_prints_the_generated_registry() {
    let output = cli(&[os("commands"), os("--json")]);
    assert_eq!(code(&output), 0);
    let value = stdout_json(&output);
    assert_eq!(value["api_version"], engine::control::API_VERSION);
    assert_eq!(value["registry_hash"], engine::control::registry_hash());
    assert_eq!(
        value["limits"],
        serde_json::to_value(engine::control::LIMITS).unwrap()
    );
    let entries = value["entries"].as_array().unwrap();
    let add_track = entries
        .iter()
        .find(|entry| entry["id"] == "timeline.add_track")
        .unwrap();
    assert_eq!(
        add_track["params_schema"]["$defs"]["NfcText"]["type"],
        "string"
    );
    assert!(entries.iter().any(|entry| entry["id"] == "batch"));
}

#[test]
fn generated_catalog_is_deterministic_and_matches_checked_in_output() {
    let first = cli(&[os("commands"), os("--json")]);
    let second = cli(&[os("commands"), os("--json")]);
    assert_eq!(code(&first), 0);
    assert_eq!(code(&second), 0);
    assert_eq!(
        first.stdout, second.stdout,
        "catalog generation must be byte-identical twice"
    );
    let value = stdout_json(&first);
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(keys, ["api_version", "entries", "limits", "registry_hash"]);
    assert_eq!(
        value["entries"],
        serde_json::to_value(engine::registry()).unwrap()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let digest = fs::read_to_string(root.join("../../crates/engine/catalog.digest")).unwrap();
    assert_eq!(value["registry_hash"].as_str().unwrap(), digest.trim());
    assert_eq!(
        fs::read_to_string(root.join("../../docs/engineering/commands.md"))
            .unwrap()
            .replace("\r\n", "\n"),
        engine::control::reference_markdown().unwrap()
    );

    let first_human = cli(&[os("commands")]);
    let second_human = cli(&[os("commands")]);
    assert_eq!(code(&first_human), 0);
    assert_eq!(first_human.stdout, second_human.stdout);
    let text = String::from_utf8_lossy(&first_human.stdout);
    assert!(text.contains(engine::control::registry_hash()));
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
    assert_eq!(value["metadata"]["kind"], "video");
    assert_eq!(value["metadata"]["width"], 128);
    assert_eq!(value["metadata"]["height"], 96);
    assert_eq!(value["metadata"]["has_audio"], true);
    assert_eq!(value["metadata"]["frame_rate"]["num"], 30);
    assert_eq!(value["metadata"]["frame_rate"]["den"], 1);
    assert_eq!(value["metadata"]["frame_rate"]["display_rounded"], 30);
    let capabilities = value["capabilities"].as_array().unwrap();
    assert!(
        capabilities
            .iter()
            .any(|stream| stream["kind"] == "video" && stream["decodable"] == true)
    );
    assert!(
        capabilities
            .iter()
            .any(|stream| stream["kind"] == "audio" && stream["decodable"] == true)
    );
}

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
fn cli_pixel_modifiers_execute_and_export_instead_of_copying() {
    if !tools() {
        return;
    }
    for masked in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("input.mp4");
        let project = dir.path().join("blur.vibecut");
        let output_path = dir.path().join("blur.mp4");
        generate_av(&video);
        media_project(&project, &video, None);
        if masked {
            exec(
                &project,
                "masks.add",
                json!({"item":0,"mask":project::Mask::new("rectangle",project::MaskShape::Rectangle)}),
            );
        } else {
            exec(
                &project,
                "effects.add",
                json!({"item":0,"effect":{"id":"blur","enabled":true,"params":{"kind":"blur","intensity":100.0}}}),
            );
        }
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
        assert!(
            difference > 10000,
            "pixel modifier was bypassed: {difference}"
        );
        if masked {
            for (x, y) in [(8usize, 8usize), (64, 8), (120, 88)] {
                let offset = (y * 128 + x) * 3;
                assert!(
                    actual[0][offset..offset + 3].iter().all(|c| *c < 20),
                    "rectangle did not clip outside"
                );
            }
        }
    }
}

#[test]
fn exec_shared_field_id_list_persists_only_compatible_item_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Lựa chọn.json");
    let mut data = project::Project::empty(project::ProjectId(0), "Batch".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: (0..2)
            .map(|i| project::Item {
                blend_mode: Default::default(),
                masks: Vec::new(),
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
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .zip(saved.data().scene().timeline.iter().flat_map(|t| &t.items))
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

#[test]
fn exec_scrub_equivalent_params_use_the_single_item_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scrub.vibecut");
    let mut data = project::Project::empty(project::ProjectId(0), "Scrub".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: vec![],
            animations: Default::default(),
            id: project::ItemId(10),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, Tick(100)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Text(project::TextParams {
                content: "Scrub".into(),
                font_size: 32.0,
                color: project::Color([255; 4]),
            }),
        }],
    });
    let original = project::Project::new(data).unwrap();
    fs::write(&path, project::encode(&original, true).unwrap()).unwrap();

    let changed = exec(
        &path,
        "timeline.set_item_param",
        json!({"item":10,"patch":{"field":"text_size","value":64}}),
    );
    assert_eq!(changed["changed"], true);
    let changed = exec(
        &path,
        "timeline.set_item_param",
        json!({"item":10,"patch":{"field":"opacity","value":0.37}}),
    );
    assert_eq!(changed["changed"], true);
    let saved = project::decode(&fs::read(&path).unwrap()).unwrap().project;
    let item = saved
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == project::ItemId(10))
        .unwrap();
    let project::ItemParams::Text(params) = &item.content else {
        panic!("text")
    };
    assert_eq!(params.font_size, 64.0);
    let opacity = item
        .animations
        .value_at(&project::AnimationPath::new("opacity").unwrap(), Tick::ZERO)
        .unwrap();
    assert!(matches!(
        opacity,
        Some(project::ParamValue::Scalar(value)) if (value - 0.37).abs() < 1e-9
    ));

    // Hostile clamp inputs refuse before writing the project.
    let before = fs::read(&path).unwrap();
    let params = json!({"item":10,"patch":{"field":"text_size","value":0}}).to_string();
    let output = cli(&[
        os("exec"),
        path.as_os_str(),
        OsStr::new("timeline.set_item_param"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_ne!(code(&output), 0);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn clips_validation_refuses_before_media_or_network_work() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.mp4");
    let export = root.path().join("exports");
    fs::create_dir_all(&export).unwrap();
    let cases: &[(&[&str], i32, Option<&str>)] = &[
        (&["--export"], 2, None),
        (&["--subs", "vi"], 2, None),
        (&["--select", "0"], 2, None),
        (&["--provider", "unknown"], 5, Some("rejected")),
        (&["--min", "0s"], 2, None),
        (
            &["--export", "--subs", "vi", "--aspect", "16:9"],
            5,
            Some("unsupported"),
        ),
        (&["--export", "--subs", "vi", "--select", "0"], 2, None),
    ];
    for (flags, expected_code, expected_kind) in cases {
        let mut args: Vec<&OsStr> = vec![os("clips"), missing.as_os_str()];
        for flag in *flags {
            match *flag {
                "--export" => {
                    args.push(os("--export"));
                    args.push(export.as_os_str());
                }
                other => args.push(os(other)),
            }
        }
        args.push(os("--json"));
        let output = cli(&args);
        assert_eq!(
            code(&output),
            *expected_code,
            "{flags:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(kind) = expected_kind {
            assert_eq!(error_json(&output)["error"]["code"], *kind, "{flags:?}");
        }
    }
}

#[test]
fn clips_missing_suggestion_credential_refuses_before_media_work() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.mp4");
    let output = Command::new(bin())
        .env_remove("OPENAI_API_KEY")
        .args([
            os("clips"),
            missing.as_os_str(),
            os("--provider"),
            os("openai"),
            os("--yes-upload"),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 4);
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "setup");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|value| value.contains("OPENAI_API_KEY"))
    );
}

#[test]
fn clips_provider_catalog_is_explicit_about_local_and_remote() {
    let output = cli(&[
        os("providers"),
        os("--operation"),
        os("clips"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let providers = stdout_json(&output)["providers"]
        .as_array()
        .unwrap()
        .clone();
    let heuristic = providers
        .iter()
        .find(|value| value["id"] == "heuristic")
        .unwrap();
    assert_eq!(heuristic["location"], "local");
    assert!(heuristic["policy_url"].is_null());
    for remote in ["openai", "anthropic"] {
        let value = providers
            .iter()
            .find(|value| value["id"] == remote)
            .unwrap();
        assert_eq!(value["location"], "remote");
        assert!(value["policy_url"].is_string());
    }
}

#[test]
fn control_session_tracks_revisions_receipts_and_saves_on_exit() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("control.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    assert_eq!(inspect["exit_code"], 0);
    assert_eq!(inspect["value"]["value"]["kind"], "inspection");
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();

    let catalog = control.send(control_request("rc", "catalog", json!({}), None, None));
    assert_eq!(catalog["exit_code"], 0);
    assert_eq!(catalog["value"]["value"]["kind"], "catalog");
    assert!(
        catalog["value"]["value"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "timeline.add_track")
    );

    let add = control.send(control_request(
        "r1",
        "timeline.add_track",
        json!({"name":"Control","kind":"video"}),
        Some(&rev),
        Some("k1"),
    ));
    assert_eq!(add["exit_code"], 0);
    assert_eq!(add["value"]["value"]["kind"], "edit");
    assert_eq!(add["value"]["value"]["outcome"]["changed"], true);
    let rev1 = add["rev_after"].as_str().unwrap().to_owned();

    let stale = control.send(control_request(
        "r2",
        "timeline.add_track",
        json!({"name":"Stale","kind":"video"}),
        Some(&rev),
        Some("k2"),
    ));
    assert_eq!(response_error(&stale), "stale_revision");
    assert_eq!(stale["exit_code"], 3);

    let conflict = control.send(control_request(
        "r3",
        "timeline.add_track",
        json!({"name":"Different","kind":"video"}),
        Some(&rev),
        Some("k1"),
    ));
    assert_eq!(response_error(&conflict), "idempotency_conflict");
    assert_eq!(conflict["exit_code"], 5);

    let replay = control.send(control_request(
        "r4",
        "timeline.add_track",
        json!({"name":"Control","kind":"video"}),
        Some(&rev),
        Some("k1"),
    ));
    assert_eq!(replay["exit_code"], 0);
    assert_eq!(replay["value"]["value"]["kind"], "edit");
    assert_eq!(replay["value"]["value"]["outcome"]["changed"], true);

    let save = control.send(control_request(
        "r5",
        "project.save",
        json!({}),
        Some(&rev1),
        Some("save1"),
    ));
    assert_eq!(save["exit_code"], 0);
    assert_eq!(save["value"]["value"]["kind"], "edit");
    assert_eq!(save["value"]["value"]["outcome"]["changed"], false);

    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");
    let info = info(&project);
    assert_eq!(info["tracks"], 2);
}

#[test]
fn control_refuses_unknown_extra_and_out_of_range_frames() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("refusals.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();

    let mut extra = control_request("x1", "inspect", json!({}), None, None);
    extra["extra"] = json!(1);
    let refused = control.send(extra);
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["code"], "invalid_params");
    assert_eq!(refused["exit_code"], 2);

    let mut version = control_request("x2", "inspect", json!({}), None, None);
    version["api_version"] = json!(2);
    let refused = control.send(version);
    assert_eq!(response_error(&refused), "schema_mismatch");
    assert_eq!(refused["exit_code"], 2);

    let mut hash = control_request("x3", "inspect", json!({}), None, None);
    hash["registry_hash"] = json!("not-the-registry");
    let refused = control.send(hash);
    assert_eq!(response_error(&refused), "schema_mismatch");

    let refused = control.send(control_request(
        "x4",
        "no.such_command",
        json!({}),
        Some(&rev),
        Some("k1"),
    ));
    assert_eq!(response_error(&refused), "unknown_command");

    let refused = control.send(control_request(
        "x5",
        "timeline.add_track",
        json!({"name":"Extra","kind":"video","unknown":1}),
        Some(&rev),
        Some("k2"),
    ));
    assert_eq!(response_error(&refused), "invalid_params");

    let refused = control.send(control_request(
        "x6",
        "inspect",
        json!({"value":9007199254740992_i64}),
        None,
        None,
    ));
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["code"], "invalid_params");
    assert_eq!(refused["exit_code"], 2);

    let (code, stderr) = control.finish();
    assert_eq!(code, 2, "{stderr}");
}

#[test]
fn control_refuses_new_effects_at_receipt_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("capacity.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let mut rev = inspect["rev_after"].as_str().unwrap().to_owned();
    for index in 0..engine::control::MAX_RECEIPTS {
        let zoom = if index % 2 == 0 { 2.0 } else { 3.0 };
        let response = control.send(control_request(
            &format!("r{index}"),
            "project.set_view_state",
            json!({"zoom":zoom,"scroll":0.0,"playhead":0}),
            Some(&rev),
            Some(&format!("k{index}")),
        ));
        assert_eq!(response["exit_code"], 0, "{response}");
        rev = response["rev_after"].as_str().unwrap().to_owned();
    }
    let refused = control.send(control_request(
        "over-capacity",
        "project.set_view_state",
        json!({"zoom":4.0,"scroll":0.0,"playhead":0}),
        Some(&rev),
        Some("over-capacity"),
    ));
    assert_eq!(response_error(&refused), "receipt_capacity");
    assert_eq!(refused["exit_code"], 5);
    let save = control.send(control_request(
        "capacity-save",
        "project.save",
        json!({}),
        Some(&rev),
        Some("capacity-save"),
    ));
    assert_eq!(response_error(&save), "receipt_capacity");
    assert_eq!(save["exit_code"], 5);
    let (code, stderr) = control.finish();
    assert_eq!(code, 5, "{stderr}");
}

#[test]
fn control_cancels_a_queued_preview_job() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("cancel.vibecut");
    overlay_project(&project, 640, 640, Tick(time::TICKS_PER_SECOND));
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let mut last = 0u64;
    for index in 0..32 {
        let response = control.send(control_request(
            &format!("p{index}"),
            "preview",
            json!({"frames":[0,1,2,3]}),
            Some(&rev),
            Some(&format!("preview-{index}")),
        ));
        assert_eq!(response["exit_code"], 0, "{response}");
        last = response["value"]["value"]["job"].as_u64().unwrap();
    }
    let cancel = control.send(control_request(
        "cancel",
        "job_cancel",
        json!({"job":last}),
        Some(&rev),
        Some("cancel"),
    ));
    assert_eq!(cancel["exit_code"], 0);
    assert_eq!(cancel["value"]["value"]["kind"], "cancel_requested");

    let mut state = String::new();
    for index in 0..200 {
        let status = control.send(control_request(
            &format!("s{index}"),
            "job_status",
            json!({"job":last}),
            None,
            None,
        ));
        assert_eq!(status["exit_code"], 0, "{status}");
        state = status["value"]["value"]["snapshot"]["state"]
            .as_str()
            .unwrap()
            .to_owned();
        if state == "cancelled" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(state, "cancelled");
    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn control_no_save_discards_edits_on_exit() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("no-save.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[os("--no-save")]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let add = control.send(control_request(
        "r1",
        "timeline.add_track",
        json!({"name":"Discarded","kind":"video"}),
        Some(&rev),
        Some("k1"),
    ));
    assert_eq!(add["exit_code"], 0);
    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(info(&project)["tracks"], 1);
}

#[test]
fn control_refuses_a_lease_held_by_the_desktop_app() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("busy-control.vibecut");
    new_project(&project);
    let lease = project::WriterLease::acquire(&project).unwrap();
    let output = Command::new(bin())
        .arg("control")
        .arg(&project)
        .arg("--json")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&output), 3);
    assert_eq!(error_json(&output)["error"]["code"], "busy");
    drop(lease);
}

#[test]
fn control_requires_an_explicit_overwrite_grant() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("overwrite.vibecut");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_path = dir.path().join("existing.mp4");
    fs::write(&output_path, b"existing").unwrap();
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let path = output_path.to_str().unwrap().to_owned();
    let refused = control.send(control_request(
        "r1",
        "export",
        json!({
            "range":{"start":0,"end":time::TICKS_PER_SECOND},
            "path":path,
            "overwrite":true,
        }),
        Some(&rev),
        Some("k1"),
    ));
    assert_eq!(response_error(&refused), "permission");
    assert_eq!(refused["exit_code"], 5);
    let (code, stderr) = control.finish();
    assert_eq!(code, 5, "{stderr}");
    assert_eq!(fs::read(&output_path).unwrap(), b"existing");
}

#[test]
fn schema_prints_the_generated_contract_envelope() {
    let first = cli(&[os("schema"), os("--json")]);
    let second = cli(&[os("schema"), os("--json")]);
    assert_eq!(code(&first), 0);
    assert_eq!(code(&second), 0);
    assert_eq!(first.stdout, second.stdout);
    let value = stdout_json(&first);
    assert_eq!(value["api_version"], engine::control::API_VERSION);
    assert_eq!(value["registry_hash"], engine::control::registry_hash());
    assert_eq!(value["contract_schema"], engine::control::contract_schema());
    assert!(value["contract_schema"]["request"].is_object());
    assert!(value["contract_schema"]["response"].is_object());
    assert!(value["contract_schema"]["event"].is_object());
}

#[test]
fn control_refused_save_keeps_dirty_state_for_exit() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("refused-save.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let add = control.send(control_request(
        "r1",
        "timeline.add_track",
        json!({"name":"Retained","kind":"video"}),
        Some(&rev),
        Some("add"),
    ));
    assert_eq!(add["exit_code"], 0);
    let refused = control.send(control_request(
        "r2",
        "project.save",
        json!({}),
        Some(&rev),
        Some("stale-save"),
    ));
    assert_eq!(response_error(&refused), "stale_revision");
    assert_eq!(refused["exit_code"], 3);
    let (code, stderr) = control.finish();
    assert_eq!(code, 3, "{stderr}");
    assert_eq!(info(&project)["tracks"], 2);
}

#[test]
fn control_save_uses_engine_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("save-receipts.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev0 = inspect["rev_after"].as_str().unwrap().to_owned();
    let add = control.send(control_request(
        "r1",
        "timeline.add_track",
        json!({"name":"Saved","kind":"video"}),
        Some(&rev0),
        Some("same"),
    ));
    assert_eq!(add["exit_code"], 0);
    let rev1 = add["rev_after"].as_str().unwrap().to_owned();

    let conflict = control.send(control_request(
        "r2",
        "project.save",
        json!({}),
        Some(&rev1),
        Some("same"),
    ));
    assert_eq!(response_error(&conflict), "idempotency_conflict");
    assert_eq!(conflict["exit_code"], 5);

    let save = control.send(control_request(
        "r3",
        "project.save",
        json!({}),
        Some(&rev1),
        Some("save"),
    ));
    assert_eq!(save["exit_code"], 0);
    assert_eq!(save["value"]["value"]["kind"], "edit");
    let rev2 = save["rev_after"].as_str().unwrap().to_owned();

    let replay = control.send(control_request(
        "replay",
        "project.save",
        json!({}),
        Some(&rev1),
        Some("save"),
    ));
    assert_eq!(replay["exit_code"], 0);
    assert_eq!(replay["rev_after"], rev2);

    let inspect = control.send(control_request("r4", "inspect", json!({}), None, None));
    assert_eq!(inspect["rev_after"], rev2);

    let empty_id = control.send(control_request(
        "",
        "project.save",
        json!({}),
        Some(&rev2),
        Some("save-2"),
    ));
    assert_eq!(empty_id["status"], "error");
    assert_eq!(empty_id["error"]["code"], "limit");
    assert_eq!(empty_id["exit_code"], 2);

    let (code, stderr) = control.finish();
    assert_eq!(code, 2, "{stderr}");
    assert_eq!(info(&project)["tracks"], 2);
}

#[test]
fn control_oversized_frame_is_refused_in_band_and_keeps_edits() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("oversized.vibecut");
    new_project(&project);
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let first = control.send(control_request(
        "r1",
        "timeline.add_track",
        json!({"name":"Before","kind":"video"}),
        Some(&rev),
        Some("before"),
    ));
    assert_eq!(first["exit_code"], 0);

    let oversized = "x".repeat(engine::control::MAX_FRAME_BYTES + 10);
    let refused = control.send_raw(&oversized);
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["code"], "limit");
    assert_eq!(refused["exit_code"], 2);

    let inspect = control.send(control_request("r2", "inspect", json!({}), None, None));
    assert_eq!(inspect["exit_code"], 0);
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let second = control.send(control_request(
        "r3",
        "timeline.add_track",
        json!({"name":"After","kind":"video"}),
        Some(&rev),
        Some("after"),
    ));
    assert_eq!(second["exit_code"], 0);
    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(info(&project)["tracks"], 3);
}

#[test]
fn control_save_failure_is_receipted_and_retry_needs_fresh_key() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("save-failure.vibecut");
    new_project(&project);
    let metadata_path = project.with_extension("vibecut.meta.json");
    fs::remove_file(&metadata_path).unwrap();
    fs::create_dir(&metadata_path).unwrap();

    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev0 = inspect["rev_after"].as_str().unwrap().to_owned();
    let add = control.send(control_request(
        "r1",
        "timeline.add_track",
        json!({"name":"Published before metadata failure","kind":"video"}),
        Some(&rev0),
        Some("add"),
    ));
    assert_eq!(add["exit_code"], 0);
    let rev1 = add["rev_after"].as_str().unwrap().to_owned();

    let save_request = control_request(
        "save-fail",
        "project.save",
        json!({}),
        Some(&rev1),
        Some("save-fail"),
    );
    let failed = control.send(save_request.clone());
    assert_eq!(response_error(&failed), "failed");
    assert_eq!(failed["exit_code"], 7);
    // Project bytes publish before the metadata directory refuses the sidecar.
    let published = project::decode(&fs::read(&project).unwrap()).unwrap();
    assert_eq!(published.project.data().scene().timeline.track_count(), 2);

    let replay = control.send(save_request);
    assert_eq!(response_error(&replay), "failed");
    assert_eq!(replay["exit_code"], 7);
    assert_eq!(replay["rev_after"], failed["rev_after"]);

    let conflict = control.send(control_request(
        "conflict",
        "timeline.add_track",
        json!({"name":"Conflicting key","kind":"video"}),
        Some(&rev1),
        Some("save-fail"),
    ));
    assert_eq!(response_error(&conflict), "idempotency_conflict");
    assert_eq!(conflict["exit_code"], 5);

    fs::remove_dir(&metadata_path).unwrap();
    let retry = control.send(control_request(
        "save-retry",
        "project.save",
        json!({}),
        Some(&rev1),
        Some("save-fresh"),
    ));
    assert_eq!(retry["exit_code"], 0);
    assert_eq!(retry["value"]["value"]["kind"], "edit");
    assert_ne!(retry["rev_after"], rev1);

    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(info(&project)["tracks"], 2);
}

#[test]
fn cli_sets_blend_and_opacity_and_renders_a_blended_frame() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("blended.vibecut");
    let output_path = dir.path().join("blended.mp4");
    overlay_project_color(
        &project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [128, 64, 192, 128],
    );
    let mut data = project::load(&project).unwrap().project.into_data();
    data.canvas.background = project::Color([64, 128, 32, 255]);
    fs::write(
        &project,
        project::encode(&project::Project::new(data).unwrap(), false).unwrap(),
    )
    .unwrap();
    exec(
        &project,
        "timeline.set_item_param",
        json!({"item":0,"patch":{"field":"blendMode","value":"screen"}}),
    );
    exec(
        &project,
        "timeline.set_item_param",
        json!({"item":0,"patch":{"field":"opacity","value":0.5}}),
    );
    let loaded = project::load(&project).unwrap().project;
    let item = loaded
        .data()
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap();
    assert_eq!(item.blend_mode, project::BlendMode::Screen);
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
    if !tools() {
        return;
    }
    let frames = decoded_rgb(&output_path, 64, 64, &[0]);
    let expected = [75, 118, 92];
    for (actual, expected) in frames[0][..3].iter().zip(expected) {
        assert!(
            (i32::from(*actual) - expected).abs() <= 12,
            "decoded blend {actual} != {expected}"
        );
    }
}

#[test]
fn exec_import_and_insert_share_persisted_media_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy.vibecut");
    project::atomic_write(
        &path,
        &project::encode(
            &project::Project::empty(project::ProjectId(1), "policy".into()).unwrap(),
            false,
        )
        .unwrap(),
    )
    .unwrap();
    let asset = serde_json::json!({
        "ephemeral":true,"id":1,"path":dir.path().join("source.mp4"),"content_hash":vec![0;32],
        "probe":{"format":"mp4","duration":1000,"streams":[{
            "index":0,"kind":"Video","codec":"synthetic","time_base":[1,1000],
            "duration":1000,"start":null,"frame_rate":{"num":60000,"den":1001},
            "frame_count":null,"variable_frame_rate":null,
            "video":{"width":640,"height":360,"color":media::ColorInfo::default(),"bit_depth":8},"audio":null
        }]}
    });
    exec(&path, "import.add_asset", json!({"asset":asset}));
    let imported = project::load(&path).unwrap().project;
    assert_eq!(imported.data().canvas.fps, time::FrameRate::FPS_59_94);
    assert_eq!(imported.data().original_canvas_size, None);
    exec(
        &path,
        "timeline.insert",
        json!({"item":{
            "animations":{},"source_asset":1,"source_range":{"start":0,"end":1000},"start":0,
            "content":{"kind":"video","params":{"source_audio_enabled":true}}
        }}),
    );
    let inserted = project::load(&path).unwrap().project;
    assert_eq!(inserted.data().canvas.fps, time::FrameRate::FPS_59_94);
    assert_eq!(
        (inserted.data().canvas.width, inserted.data().canvas.height),
        (640, 360)
    );
    assert_eq!(
        inserted.data().original_canvas_size,
        Some(project::CanvasSize {
            width: 640,
            height: 360
        })
    );
    assert!(inserted.data().assets[0].ephemeral());
    assert_eq!(inserted.data().scene().timeline.main.items.len(), 1);
}

fn write_job(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn batch_refusal(output: &Output, exit: i32) -> Value {
    assert_eq!(
        code(output),
        exit,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    error_json(output)
}

fn batch_result(output: &Output, exit: i32) -> Value {
    assert_eq!(
        code(output),
        exit,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(output)
}

#[test]
fn batch_exports_in_file_order_without_mutating_projects() {
    let dir = tempfile::tempdir().unwrap();
    let first_project = dir.path().join("đầu tiên.vibecut");
    let second_project = dir.path().join("thứ hai.vibecut");
    overlay_project_color(
        &first_project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [255, 0, 0, 255],
    );
    overlay_project_color(
        &second_project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [0, 255, 0, 255],
    );
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({
            "version": 1,
            "jobs": [
                {"project": "đầu tiên.vibecut", "output": "out/kết quả 1.mp4"},
                {"project": "thứ hai.vibecut", "output": "out/kết quả 2.mp4"}
            ]
        }),
    );
    let first_before = fs::read(&first_project).unwrap();
    let second_before = fs::read(&second_project).unwrap();
    let output = cli(&[os("batch"), job_file.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    assert_eq!(value["status"], "completed");
    let jobs = value["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["index"], 0);
    assert_eq!(jobs[0]["status"], "completed");
    assert_eq!(jobs[1]["index"], 1);
    assert_eq!(jobs[1]["status"], "completed");
    let first_output = output_dir.join("kết quả 1.mp4");
    let second_output = output_dir.join("kết quả 2.mp4");
    assert_eq!(
        jobs[0]["output"],
        json_path_string(&first_output),
        "result order must follow job-file order"
    );
    assert_eq!(jobs[1]["output"], json_path_string(&second_output));
    assert!(fs::metadata(&first_output).unwrap().len() > 0);
    assert!(fs::metadata(&second_output).unwrap().len() > 0);
    assert_eq!(fs::read(&first_project).unwrap(), first_before);
    assert_eq!(fs::read(&second_project).unwrap(), second_before);
    if tools() {
        let first = decoded_rgb(&first_output, 64, 64, &[0]);
        let second = decoded_rgb(&second_output, 64, 64, &[0]);
        for (actual, expected) in first[0][..3].iter().zip([255, 0, 0]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 12,
                "first job exported the wrong project: {actual}"
            );
        }
        for (actual, expected) in second[0][..3].iter().zip([0, 255, 0]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 12,
                "second job exported the wrong project: {actual}"
            );
        }
    }
}

fn json_path_string(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn batch_refuses_malformed_oversized_and_hostile_job_files() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("a.vibecut");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();

    let case = |name: &str, bytes: Vec<u8>, exit: i32| {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        let output = cli(&[os("batch"), path.as_os_str(), os("--json")]);
        batch_refusal(&output, exit);
    };
    case("truncated.json", b"{".to_vec(), 2);
    case(
        "duplicate.json",
        br#"{"version":1,"version":1,"jobs":[]}"#.to_vec(),
        2,
    );
    case(
        "unknown.json",
        br#"{"version":1,"jobs":[],"extra":true}"#.to_vec(),
        2,
    );
    case("empty.json", br#"{"version":1,"jobs":[]}"#.to_vec(), 2);
    case("version.json", br#"{"version":2,"jobs":[]}"#.to_vec(), 5);
    case(
        "nul.json",
        json!({"version":1,"jobs":[{"project":"a.vibecut","output":"bad\u{0}name.mp4"}]})
            .to_string()
            .into_bytes(),
        2,
    );
    case(
        "long.json",
        json!({
            "version":1,
            "jobs":[{"project":"a.vibecut","output":"a".repeat(4097)}]
        })
        .to_string()
        .into_bytes(),
        2,
    );
    let too_many: Vec<Value> = (0..1001)
        .map(|index| {
            json!({
                "project": "a.vibecut",
                "output": format!("out/{index}.mp4")
            })
        })
        .collect();
    case(
        "too-many.json",
        json!({"version":1,"jobs":too_many})
            .to_string()
            .into_bytes(),
        2,
    );
    case("oversized.json", vec![b' '; (1 << 20) + 1], 2);
    let directory = dir.path().join("directory.json");
    fs::create_dir(&directory).unwrap();
    let output = cli(&[os("batch"), directory.as_os_str(), os("--json")]);
    batch_refusal(&output, 2);
    let missing = cli(&[
        os("batch"),
        dir.path().join("missing.json").as_os_str(),
        os("--json"),
    ]);
    batch_refusal(&missing, 7);
}

#[test]
fn batch_refuses_unsafe_or_ambiguous_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("a.vibecut");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let run = |name: &str, value: Value, flags: &[&str], exit: i32| {
        let path = dir.path().join(name);
        write_job(&path, value);
        let mut args = vec![os("batch"), path.as_os_str(), os("--json")];
        args.extend(flags.iter().map(|flag| os(flag)));
        let output = cli(&args);
        batch_refusal(&output, exit);
    };
    run(
        "duplicate-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/same.mp4"},
            {"project":"a.vibecut","output":"out/same.mp4"}
        ]}),
        &[],
        2,
    );
    run(
        "aliases-project.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"a.vibecut","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    run(
        "aliases-job-file.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"aliases-job-file.json","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    run(
        "aliases-metadata.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"a.vibecut.meta.json","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    run(
        "reserved-lock.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/.export.lock","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        5,
    );
    run(
        "missing-directory.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"missing/out.mp4"}
        ]}),
        &[],
        2,
    );
    let nested = output_dir.join("nested.mp4");
    fs::create_dir(&nested).unwrap();
    run(
        "non-regular-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/nested.mp4","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    let existing = output_dir.join("existing.mp4");
    fs::write(&existing, b"previous").unwrap();
    run(
        "existing-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/existing.mp4"}
        ]}),
        &[],
        5,
    );
    assert_eq!(fs::read(&existing).unwrap(), b"previous");
    run(
        "overwrite-without-grant.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/existing.mp4","overwrite":true}
        ]}),
        &[],
        5,
    );
    assert_eq!(fs::read(&existing).unwrap(), b"previous");
    run(
        "degraded-without-grant.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/degraded.mp4","allow_degraded":true}
        ]}),
        &[],
        5,
    );
    assert!(!dir.path().join("out/degraded.mp4").exists());
    let output = cli(&[
        os("batch"),
        dir.path().join("overwrite-without-grant.json").as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(fs::read(&existing).unwrap(), b"previous");
}

#[test]
fn batch_reports_missing_project_and_empty_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let missing_job = dir.path().join("missing-project.json");
    write_job(
        &missing_job,
        json!({"version":1,"jobs":[
            {"project":"missing.vibecut","output":"out/missing.mp4"}
        ]}),
    );
    let output = cli(&[os("batch"), missing_job.as_os_str(), os("--json")]);
    batch_refusal(&output, 7);

    let empty = dir.path().join("empty.vibecut");
    let project = project::Project::empty(project::ProjectId(0), "empty".into()).unwrap();
    fs::write(&empty, project::encode(&project, false).unwrap()).unwrap();
    let empty_job = dir.path().join("empty-project.json");
    write_job(
        &empty_job,
        json!({"version":1,"jobs":[
            {"project":"empty.vibecut","output":"out/empty.mp4"}
        ]}),
    );
    let output = cli(&[os("batch"), empty_job.as_os_str(), os("--json")]);
    let value = batch_result(&output, 5);
    assert_eq!(value["jobs"][0]["error"]["code"], "rejected");
    assert!(!dir.path().join("out/empty.mp4").exists());
}

#[test]
fn batch_stops_after_failure_without_promoting_partial_output() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let good = dir.path().join("good.vibecut");
    let broken = dir.path().join("broken.vibecut");
    overlay_project(&good, 64, 64, Tick(time::TICKS_PER_SECOND));
    generate_av(&source);
    media_project(&broken, &source, None);
    fs::remove_file(&source).unwrap();
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let preserved = output_dir.join("broken.mp4");
    fs::write(&preserved, b"previous").unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"good.vibecut","output":"out/good.mp4"},
            {"project":"broken.vibecut","output":"out/broken.mp4","overwrite":true},
            {"project":"good.vibecut","output":"out/after.mp4"}
        ]}),
    );
    let output = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    let value = batch_result(&output, 4);
    assert_eq!(value["jobs"][1]["error"]["code"], "unavailable");
    assert!(output_dir.join("good.mp4").exists());
    assert_eq!(fs::read(&preserved).unwrap(), b"previous");
    assert!(!output_dir.join("after.mp4").exists());
}

#[test]
fn batch_allow_degraded_is_explicit_consent() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let project = dir.path().join("degraded.vibecut");
    generate_av(&source);
    media_project(&project, &source, None);
    fs::remove_file(&source).unwrap();
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"degraded.vibecut","output":"out/degraded.mp4","allow_degraded":true}
        ]}),
    );
    let refused = cli(&[os("batch"), job_file.as_os_str(), os("--json")]);
    batch_refusal(&refused, 5);
    assert!(!output_dir.join("degraded.mp4").exists());
    let accepted = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-degraded"),
        os("--json"),
    ]);
    assert_eq!(
        code(&accepted),
        0,
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert!(output_dir.join("degraded.mp4").exists());
}

#[test]
fn batch_refuses_an_output_that_aliases_another_jobs_source() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let media = dir.path().join("media.vibecut");
    let overlay = dir.path().join("overlay.vibecut");
    generate_av(&source);
    media_project(&media, &source, None);
    overlay_project(&overlay, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"media.vibecut","output":"out/media.mp4"},
            {"project":"overlay.vibecut","output":"source.mp4","overwrite":true}
        ]}),
    );
    let output = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    batch_refusal(&output, 2);
    assert!(source.exists());
    assert!(!output_dir.join("media.mp4").exists());
}

#[cfg(unix)]
#[test]
fn batch_refuses_an_output_hard_linked_to_another_jobs_source() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let media = dir.path().join("media.vibecut");
    let overlay = dir.path().join("overlay.vibecut");
    generate_av(&source);
    media_project(&media, &source, None);
    overlay_project(&overlay, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let linked = output_dir.join("linked.mp4");
    fs::hard_link(&source, &linked).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"media.vibecut","output":"out/media.mp4"},
            {"project":"overlay.vibecut","output":"out/linked.mp4","overwrite":true}
        ]}),
    );
    let output = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    batch_refusal(&output, 2);
    assert!(source.exists());
    assert!(!output_dir.join("media.mp4").exists());
}
