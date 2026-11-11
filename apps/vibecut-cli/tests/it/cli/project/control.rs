//! Verifies native control command behaviour.
use crate::cli::fixtures::decoded_rgb;
use crate::cli::fixtures::{info, new_project, overlay_project, overlay_project_color};
use crate::cli::process::{bin, code, error_json, os};
use crate::media_fixture::tools;
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use time::Tick;

pub(super) struct ControlSession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    stderr: std::process::ChildStderr,
}
impl ControlSession {
    pub(super) fn start(project: &Path, extra: &[&OsStr]) -> Self {
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
    pub(super) fn send(&mut self, request: Value) -> Value {
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
    pub(super) fn finish(mut self) -> (i32, String) {
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
fn control_snapshot_writes_the_requested_frame_and_refuses_project_alias() {
    let dir = tempfile::tempdir().unwrap();
    let project_path = dir.path().join("snapshot.vibecut");
    overlay_project(&project_path, 40, 24, Tick(time::TICKS_PER_SECOND));
    let before = fs::read(&project_path).unwrap();
    let out = dir.path().join("frame.png");
    let mut control = ControlSession::start(&project_path, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();

    let snapshot = control.send(control_request(
        "s1",
        "preview.snapshot",
        json!({"time":0,"path":out}),
        Some(&rev),
        Some("snapshot-1"),
    ));
    assert_eq!(snapshot["exit_code"], 0);
    assert_eq!(snapshot["value"]["value"]["kind"], "snapshot");
    assert_eq!(snapshot["value"]["value"]["snapshot"]["width"], 40);
    assert_eq!(snapshot["value"]["value"]["snapshot"]["height"], 24);
    let bytes = fs::read(&out).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 40);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 24);

    let refused = control.send(control_request(
        "s2",
        "preview.snapshot",
        json!({"time":0,"path":project_path}),
        Some(&rev),
        Some("snapshot-2"),
    ));
    assert_eq!(response_error(&refused), "refused");
    assert_eq!(fs::read(&project_path).unwrap(), before);
    let (code, stderr) = control.finish();
    assert_eq!(code, 5, "{stderr}");

    // Replacement is granted only by the trusted process flag, never by JSON.
    for allow in [false, true] {
        let out = dir.path().join("existing.png");
        fs::write(&out, b"sentinel").unwrap();
        let flags = if allow {
            vec![os("--allow-overwrite")]
        } else {
            vec![]
        };
        let mut control = ControlSession::start(&project_path, &flags);
        let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
        let rev = inspect["rev_after"].as_str().unwrap();
        let response = control.send(control_request(
            "s1",
            "preview.snapshot",
            json!({"time":0,"path":out}),
            Some(rev),
            Some("replace"),
        ));
        if allow {
            assert_eq!(response["exit_code"], 0);
            assert_eq!(&fs::read(&out).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        } else {
            assert_ne!(response["exit_code"], 0);
            assert_eq!(fs::read(&out).unwrap(), b"sentinel");
        }
        let (code, stderr) = control.finish();
        assert_eq!(code == 0, allow, "{stderr}");
    }

    let empty_path = dir.path().join("empty.vibecut");
    new_project(&empty_path);
    let mut empty = ControlSession::start(&empty_path, &[]);
    let inspect = empty.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let refused = empty.send(control_request(
        "s1",
        "preview.snapshot",
        json!({"time":0,"path":dir.path().join("empty.png")}),
        Some(&rev),
        Some("empty-snapshot"),
    ));
    assert_eq!(response_error(&refused), "empty_project");
    let (code, stderr) = empty.finish();
    assert_eq!(code, 5, "{stderr}");
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
    version["api_version"] = json!(engine::control::API_VERSION + 1);
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
            "settings":{"version":1,"resolution":null,"fps":null,"bit_rate":null,"codec":"automatic","profile":"automatic","scene":{"kind":"active"},"range":{"kind":"in_out","start":0,"end":time::TICKS_PER_SECOND}},
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
    assert_eq!(response_error(&failed), "io");
    assert_eq!(failed["exit_code"], 7);
    // Project bytes publish before the metadata directory refuses the sidecar.
    let published = project::decode(&fs::read(&project).unwrap()).unwrap();
    assert_eq!(published.project.data().scene().timeline.track_count(), 2);

    let replay = control.send(save_request);
    assert_eq!(response_error(&replay), "io");
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
fn control_commands_render_a_transition_midpoint_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("transition.vibecut");
    overlay_project_color(&path, 4, 4, Tick(100), [255, 0, 0, 255]);
    let mut d = project::load(&path).unwrap().project.into_data();
    let track = &mut d.scene_mut().timeline.overlays[0];
    let mut next = track.items[0].clone();
    next.id = project::ItemId(2);
    next.start = Tick(100);
    next.content = project::ItemParams::Overlay(project::OverlayParams {
        color: project::Color([0, 0, 255, 255]),
    });
    track.items.push(next);
    fs::write(
        &path,
        project::encode(&project::Project::new(d).unwrap(), true).unwrap(),
    )
    .unwrap();
    let mut control = ControlSession::start(&path, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap();
    let added=control.send(control_request("a1","transition.add",json!({"track":1,"from":0,"to":2,"kind":"cross_dissolve","duration":40,"alignment":"center"}),Some(rev),Some("add")));
    assert_eq!(added["exit_code"], 0, "{added}");
    let out = dir.path().join("midpoint.png");
    let snapshot = control.send(control_request(
        "s1",
        "preview.snapshot",
        json!({"time":100,"path":out}),
        Some(added["rev_after"].as_str().unwrap()),
        Some("snapshot"),
    ));
    assert_eq!(snapshot["exit_code"], 0, "{snapshot}");
    let bytes = fs::read(&out).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    if tools() {
        let pixels = decoded_rgb(&out, 4, 4, &[0]);
        assert!(
            pixels[0]
                .as_chunks::<3>()
                .0
                .iter()
                .all(|p| p[0].abs_diff(188) <= 1 && p[1] == 0 && p[2].abs_diff(188) <= 1),
            "{:?}",
            pixels[0]
        );
    }
    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn control_sets_every_text_style_key_and_the_snapshot_shows_it() {
    use time::TimeRange;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text.vibecut");
    let mut data = project::Project::empty(project::ProjectId(0), "Text".into())
        .unwrap()
        .into_data();
    data.canvas.width = 320;
    data.canvas.height = 180;
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            retime: Default::default(),
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: vec![],
            animations: Default::default(),
            id: project::ItemId(10),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Text(project::TextParams {
                font_size: 20.0,
                ..project::TextParams::new("Xin chào")
            }),
        }],
    });
    fs::write(
        &path,
        project::encode(&project::Project::new(data).unwrap(), true).unwrap(),
    )
    .unwrap();

    let mut control = ControlSession::start(&path, &[]);
    let inspect = control.send(control_request("r0", "inspect", json!({}), None, None));
    let mut rev = inspect["rev_after"].as_str().unwrap().to_owned();
    let snapshot = |control: &mut ControlSession, rev: &str, name: &str| {
        let out = dir.path().join(name);
        let response = control.send(control_request(
            name,
            "preview.snapshot",
            json!({"time":0,"path":out}),
            Some(rev),
            Some(name),
        ));
        assert_eq!(response["exit_code"], 0, "{response}");
        fs::read(out).unwrap()
    };
    let before = snapshot(&mut control, &rev, "before.png");

    for (index, (field, value)) in [
        ("text_weight", json!(700)),
        ("text_italic", json!(true)),
        ("text_decoration", json!("underline")),
        ("text_align", json!("left")),
        ("text_letter_spacing", json!(2.5)),
        ("text_line_height", json!(30.0)),
        ("text_background_padding_x", json!(10.0)),
        ("text_background_padding_y", json!(6.0)),
        ("text_background_offset_x", json!(8.0)),
        ("text_background_offset_y", json!(-4.0)),
        ("text_background_corner_radius", json!(5.0)),
        ("text_background_color", json!([200, 0, 0, 255])),
    ]
    .into_iter()
    .enumerate()
    {
        let response = control.send(control_request(
            &format!("e{index}"),
            "timeline.set_item_param",
            json!({"item":10,"patch":{"field":field,"value":value}}),
            Some(&rev),
            Some(&format!("edit-{index}")),
        ));
        assert_eq!(response["exit_code"], 0, "{field}: {response}");
        assert_eq!(
            response["value"]["value"]["outcome"]["changed"], true,
            "{field}"
        );
        rev = response["rev_after"].as_str().unwrap().to_owned();
    }
    // Unknown words and out-of-range values are refused and leave the revision alone.
    for (index, (field, value)) in [
        ("text_align", json!("justify")),
        ("text_decoration", json!("overline")),
        ("text_background_padding_x", json!(500.0)),
        ("text_background_offset_y", json!(-5000.0)),
        ("text_weight", json!(1000)),
    ]
    .into_iter()
    .enumerate()
    {
        let response = control.send(control_request(
            &format!("bad{index}"),
            "timeline.set_item_param",
            json!({"item":10,"patch":{"field":field,"value":value}}),
            Some(&rev),
            Some(&format!("bad-{index}")),
        ));
        assert_ne!(response["exit_code"], 0, "{field}: {response}");
    }
    let after = snapshot(&mut control, &rev, "after.png");
    assert_eq!(&after[..8], b"\x89PNG\r\n\x1a\n");
    assert_ne!(before, after, "the styled frame differs from the plain one");
    let (code, stderr) = control.finish();
    assert_eq!(code, 0, "{stderr}");

    let saved = project::load(&path).unwrap().project;
    let item = saved
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .next()
        .unwrap();
    let project::ItemParams::Text(text) = &item.content else {
        panic!("text item")
    };
    assert_eq!(
        (text.weight, text.italic, text.decoration, text.align),
        (
            700,
            true,
            project::TextDecoration::Underline,
            project::TextAlign::Left
        )
    );
    let background = text.background.as_ref().unwrap();
    assert_eq!(
        (
            background.padding_x,
            background.padding_y,
            background.offset_x,
            background.offset_y
        ),
        (10.0, 6.0, 8.0, -4.0)
    );
}

#[test]
fn snapshot_io_class_preserves_control_exit_and_existing_parent_file() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("snapshot.vibecut");
    overlay_project(&project, 64, 48, Tick(time::TICKS_PER_SECOND));
    let parent = dir.path().join("parent-file");
    fs::write(&parent, b"preserved").unwrap();
    let mut control = ControlSession::start(&project, &[]);
    let inspect = control.send(control_request("inspect", "inspect", json!({}), None, None));
    let rev = inspect["rev_after"].as_str().unwrap();
    let response = control.send(control_request(
        "snapshot",
        "preview.snapshot",
        json!({"time":0,"path":parent.join("frame.png")}),
        Some(rev),
        Some("snapshot"),
    ));
    assert_eq!(response_error(&response), "io");
    assert_eq!(response["exit_code"], 7);
    assert_eq!(response["rev_before"], response["rev_after"]);
    assert_eq!(fs::read(&parent).unwrap(), b"preserved");
    let (code, stderr) = control.finish();
    assert_eq!(code, 7, "{stderr}");
}
