//! Verifies MCP through the public headless adapter.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Client {
    child: Child,
    input: ChildStdin,
    responses: mpsc::Receiver<Value>,
    _cache: tempfile::TempDir,
}
impl Client {
    fn start(args: &[&std::ffi::OsStr]) -> Self {
        // Every MCP process gets its own cache root, so parallel tests and the
        // real user cache never share a lock.
        let cache = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_vibecut-cli"))
            .args(args)
            .env("HOME", cache.path())
            .env("LOCALAPPDATA", cache.path())
            .env("XDG_CACHE_HOME", cache.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(value) = serde_json::from_str(&line)
                    && send.send(value).is_err()
                {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            responses,
            _cache: cache,
        };
        let result = client.rpc(1, "initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"conformance","version":"1"}}));
        assert!(result.get("result").is_some(), "{result}");
        writeln!(
            client.input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        client
    }
    fn rpc(&mut self, id: u32, method: &str, params: Value) -> Value {
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        self.responses
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
    }
    fn call(
        &mut self,
        command: &str,
        params: Value,
        rev: Option<&Value>,
        key: Option<&str>,
    ) -> Value {
        let mut args = json!({"request_id":"request","command":command,"params":params});
        if let Some(rev) = rev {
            args["base_rev"] = rev.clone();
        }
        if let Some(key) = key {
            args["idempotency_key"] = json!(key);
        }
        self.rpc(
            2,
            "tools/call",
            json!({"name":"command_run","arguments":args}),
        )["result"]["structuredContent"]
            .clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn fixture(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    let project = project::Project::empty(project::ProjectId(1), "MCP".into()).unwrap();
    std::fs::write(&path, project::encode(&project, true).unwrap()).unwrap();
    path
}

fn wait_for_media_job(client: &mut Client, job: &Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = client.call("job_status", json!({"job": job}), None, None);
        match status
            .pointer("/value/value/snapshot/state")
            .and_then(Value::as_str)
        {
            Some("completed") => return status,
            Some("failed" | "cancelled") => panic!("media job ended early: {status}"),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "media job did not finish: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// An allowed agent call reaches the same media caches the CLI control session
/// lends; a warm rerun still completes after the source is deleted.
#[test]
fn media_generation_runs_through_the_attached_mcp_caches() {
    if !crate::media_fixture::tools() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("clip.mp4");
    crate::media_fixture::generate_av(&media, 3);
    let path = root.path().join("media.json");
    crate::media_fixture::media_project(&path, &media, None);
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let mut rev = client.call("inspect", json!({}), None, None)["rev_after"].clone();
    for (key, command, params) in [
        ("waveform", "media.generate_waveform", json!({"asset":1})),
        (
            "thumbnails",
            "media.generate_clip_thumbnails",
            json!({"item":0,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":71.5}),
        ),
    ] {
        let accepted = client.call(command, params, Some(&rev), Some(key));
        assert_eq!(
            accepted.pointer("/value/status"),
            Some(&json!("result")),
            "{accepted}"
        );
        let job = accepted.pointer("/value/value/job").unwrap().clone();
        wait_for_media_job(&mut client, &job);
        rev = accepted["rev_after"].clone();
    }
    std::fs::remove_file(&media).unwrap();
    for (key, command, params) in [
        (
            "waveform-warm",
            "media.generate_waveform",
            json!({"asset":1}),
        ),
        (
            "thumbnails-warm",
            "media.generate_clip_thumbnails",
            json!({"item":0,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":71.5}),
        ),
    ] {
        let accepted = client.call(command, params, Some(&rev), Some(key));
        assert_eq!(
            accepted.pointer("/value/status"),
            Some(&json!("result")),
            "{accepted}"
        );
        let job = accepted.pointer("/value/value/job").unwrap().clone();
        wait_for_media_job(&mut client, &job);
        rev = accepted["rev_after"].clone();
    }
}

/// A workflow job started over MCP returns its data through `job_status` as one tagged
/// `output`; the agent turns it into an ordinary edit bound to the revision the job started at.
#[test]
fn workflow_jobs_return_their_output_and_the_agent_applies_it_as_an_edit() {
    if !crate::media_fixture::tools() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("clip.mp4");
    crate::media_fixture::generate_av(&media, 3);
    let path = root.path().join("workflows.json");
    crate::media_fixture::media_project(&path, &media, Some(&media));
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let rev = client.call("inspect", json!({}), None, None)["rev_after"].clone();
    let targets = client.call(
        "audio.set_loudness",
        json!({"scope":"master","target_lufs":-14.0,"true_peak_db":-1.0}),
        Some(&rev),
        Some("targets"),
    );
    let rev = targets["rev_after"].clone();
    // (key, command, params, output kind, a field the output must carry)
    for (key, command, params, kind, field) in [
        (
            "scenes",
            "media.detect_scenes",
            json!({"asset":1}),
            "scenes",
            "candidates",
        ),
        (
            "loudness",
            "audio.measure_loudness",
            json!({}),
            "loudness",
            "gains",
        ),
    ] {
        let accepted = client.call(command, params, Some(&rev), Some(key));
        assert_eq!(
            accepted.pointer("/value/status"),
            Some(&json!("result")),
            "{accepted}"
        );
        let job = accepted.pointer("/value/value/job").unwrap().clone();
        let started_at = accepted.pointer("/value/value/revision").unwrap().clone();
        let status = wait_for_media_job(&mut client, &job);
        let output = status.pointer("/value/value/snapshot/output").unwrap();
        assert_eq!(output["kind"], kind, "{status}");
        assert!(
            output.get(field).is_some_and(|value| !value.is_null()),
            "{status}"
        );
        if kind == "loudness" {
            let applied = client.call(
                "audio.apply_loudness",
                json!({"gains": output["gains"]}),
                Some(&started_at),
                Some("apply"),
            );
            assert_eq!(
                applied.pointer("/value/value/outcome/changed"),
                Some(&json!(true)),
                "{applied}"
            );
        }
    }
}

#[test]
fn stdio_catalog_edits_receipts_and_file_permission_share_engine_admission() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), "project.json");
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let listed = client.rpc(2, "tools/list", json!({}));
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"command_run")
            && names.contains(&"batch")
            && !names.contains(&"project_rename")
    );
    let initial = client.call("inspect", json!({}), None, None);
    let rev = initial["rev_after"].clone();
    let applied = client.call(
        "project.rename",
        json!({"name":"changed"}),
        Some(&rev),
        Some("once"),
    );
    let replay = client.call(
        "project.rename",
        json!({"name":"changed"}),
        Some(&rev),
        Some("once"),
    );
    assert_eq!(applied, replay);
    let conflict = client.call(
        "project.rename",
        json!({"name":"twice"}),
        Some(&rev),
        Some("once"),
    );
    assert_eq!(conflict["value"]["value"], "idempotency_conflict");
    let stale = client.call(
        "project.rename",
        json!({"name":"stale"}),
        Some(&rev),
        Some("stale"),
    );
    assert_eq!(stale["value"]["value"], "stale_revision");
    let mut asset = engine::registry()
        .iter()
        .find(|spec| spec.id == "import.add_asset")
        .unwrap()
        .example
        .params
        .clone();
    asset["asset"]["path"] = json!(root.path().join("private.txt"));
    let denied = client.call(
        "import.add_asset",
        asset.clone(),
        Some(&applied["rev_after"]),
        Some("file"),
    );
    assert_eq!(denied["value"]["value"], "permission");
    let font_path = root.path().join("human-only.ttf");
    let font_bytes = include_bytes!(
        "../../../../crates/libraries/tests/fixtures/fonts/VibeCutFixtureSans-Regular.ttf"
    );
    std::fs::write(&font_path, font_bytes).unwrap();
    let denied = client.call(
        "fonts.import",
        json!({"path":font_path,"accept_license":true}),
        Some(&applied["rev_after"]),
        Some("font-import"),
    );
    assert_eq!(
        denied["value"]["value"], "permission",
        "accept_license cannot self-grant a source read"
    );
    assert_eq!(std::fs::read(font_path).unwrap(), font_bytes);
    let denied = client.rpc(4, "tools/call", json!({"name":"batch","arguments":{"request_id":"batch-file","params":{"commands":[{"command":"project.rename","params":{"name":"must not apply"}},{"command":"import.add_asset","params":asset}]},"base_rev":applied["rev_after"],"idempotency_key":"batch-file"}}));
    assert_eq!(
        denied.pointer("/result/structuredContent/value/value"),
        Some(&json!("permission"))
    );
    let denied = client.call("batch", json!({"commands":[{"command":"project.rename","params":{"name":"no partial edit"}},{"command":"begin_gesture","params":{}}]}), Some(&applied["rev_after"]), Some("batch"));
    assert_eq!(denied["value"]["value"], "refused");
    assert_eq!(
        client.call("inspect", json!({}), None, None)["rev_after"],
        applied["rev_after"]
    );
}

#[test]
fn caption_style_command_run_custom_values_stale_revision_and_replay() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), "captions.json");
    let ass = root.path().join("captions.ass");
    std::fs::write(&ass, "[Script Info]\nPlayResX: 1920\nPlayResY: 1080\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Hello\n").unwrap();
    let prepared = engine::prepare_ass_import(&engine::AssImportRequest {
        path: ass,
        language: engine::subtitles::LanguageTag::parse("en").unwrap(),
        start: time::Tick::ZERO,
        track: None,
        name: None,
        asset_id: media::AssetId(1),
    })
    .unwrap();
    let mut session = engine::Session::new(project::load(&path).unwrap().project, 10).unwrap();
    let imported = session.apply(prepared.command).unwrap();
    let item = imported
        .created
        .iter()
        .find_map(|id| {
            if let engine::CreatedId::Item(item) = id {
                Some(*item)
            } else {
                None
            }
        })
        .unwrap();
    std::fs::write(&path, project::encode(session.project(), true).unwrap()).unwrap();
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let initial = client.call("inspect", json!({}), None, None);
    let highlight = client.call(
        "captions.set_word_highlight",
        json!({"item":item,"enabled":true}),
        Some(&initial["rev_after"]),
        Some("word-highlight"),
    );
    assert_eq!(
        highlight.pointer("/value/value/outcome/changed"),
        Some(&json!(true)),
        "{highlight}"
    );
    assert_eq!(
        client.call(
            "captions.set_word_highlight",
            json!({"item":item,"enabled":true}),
            Some(&initial["rev_after"]),
            Some("word-highlight")
        ),
        highlight
    );
    let preview = client.call(
        "preview",
        json!({"frames":[0]}),
        Some(&highlight["rev_after"]),
        Some("word-preview"),
    );
    assert_eq!(
        preview.pointer("/value/value/word_highlight_warnings/warnings/0/reason"),
        Some(&json!("missing_timing")),
        "{preview}"
    );
    assert!(
        !preview["value"]["value"]["word_highlight_warnings"]
            .to_string()
            .contains("Hello")
    );
    let initial = client.call("inspect", json!({}), None, None);
    let params = json!({"item":item,"style":{"kind":"preset","preset":"box"}});
    let applied = client.call(
        "captions.apply_style",
        params.clone(),
        Some(&initial["rev_after"]),
        Some("style"),
    );
    assert_eq!(
        applied.pointer("/value/value/outcome/changed"),
        Some(&json!(true)),
        "{applied}"
    );
    assert_eq!(
        client.call(
            "captions.apply_style",
            params.clone(),
            Some(&initial["rev_after"]),
            Some("style")
        ),
        applied
    );
    assert_eq!(
        client.call(
            "captions.apply_style",
            params,
            Some(&initial["rev_after"]),
            Some("stale")
        )["value"]["value"],
        "stale_revision"
    );
    let inspected = client.call("inspect", json!({}), None, None);
    let data: project::ProjectData =
        serde_json::from_value(inspected["value"]["value"]["project"].clone()).unwrap();
    let project::ItemParams::Caption(params) = &data
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap()
        .content
    else {
        panic!("caption")
    };
    let mut style = params.cues.styles[0].clone();
    assert_eq!(style.border_style, 3);
    assert_eq!(style.background_color, [0, 0, 0, 255]);
    style.primary_color = [20, 240, 60, 255];
    let custom = client.call(
        "captions.apply_style",
        json!({"item":item,"style":{"kind":"custom","style":style}}),
        Some(&applied["rev_after"]),
        Some("custom"),
    );
    assert_eq!(
        custom.pointer("/value/value/outcome/changed"),
        Some(&json!(true)),
        "{custom}"
    );
    let inspected = client.call("inspect", json!({}), None, None);
    let data: project::ProjectData =
        serde_json::from_value(inspected["value"]["value"]["project"].clone()).unwrap();
    let found = data
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap();
    let project::ItemParams::Caption(p) = &found.content else {
        panic!("caption")
    };
    assert_eq!(p.cues.styles, vec![style]);
    let saved = client.call(
        "captions.save_preset",
        json!({"item":item,"name":"Project style"}),
        Some(&custom["rev_after"]),
        Some("bank-save"),
    );
    assert_eq!(
        saved["value"]["value"]["outcome"]["changed"], true,
        "{saved}"
    );
    let id = saved["value"]["value"]["outcome"]["created"][0]["caption_preset"]
        .as_u64()
        .unwrap();
    assert_eq!(
        client.call(
            "captions.save_preset",
            json!({"item":item,"name":"Project style"}),
            Some(&custom["rev_after"]),
            Some("bank-save")
        ),
        saved
    );
    let inspected = client.call("inspect", json!({}), None, None);
    let bank = &inspected["value"]["value"]["project"]["caption_presets"];
    assert_eq!(bank[0]["id"], id);
    assert_eq!(bank[0]["name"], "Project style");
    assert_eq!(bank[0]["style"]["color"], json!([20, 240, 60, 255]));
    let renamed = client.call(
        "captions.rename_preset",
        json!({"preset":id,"name":"Renamed"}),
        Some(&saved["rev_after"]),
        Some("bank-rename"),
    );
    assert_eq!(
        renamed["value"]["value"]["outcome"]["changed"], true,
        "{renamed}"
    );
    let copied = client.call(
        "captions.apply_style",
        json!({"item":item,"style":{"kind":"saved","preset":id,"overrides":{"size":66.0,"outline":null}}}),
        Some(&renamed["rev_after"]),
        Some("bank-copy"),
    );
    assert_eq!(
        copied["value"]["value"]["outcome"]["changed"], true,
        "{copied}"
    );
    let inspected = client.call("inspect", json!({}), None, None);
    let data: project::ProjectData =
        serde_json::from_value(inspected["value"]["value"]["project"].clone()).unwrap();
    let project::ItemParams::Caption(params) = &data
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap()
        .content
    else {
        panic!("caption")
    };
    assert_eq!(params.preset.map(|preset| preset.0), Some(id));
    assert_eq!(params.overrides.size, Some(66.0));
    assert_eq!(params.overrides.outline, Some(None));
    assert_eq!(
        client.call(
            "captions.delete_preset",
            json!({"preset":id}),
            Some(&custom["rev_after"]),
            Some("bank-stale")
        )["value"]["value"],
        "stale_revision"
    );
    // A referenced live preset is not deleted; materialize first, then delete.
    let detached = client.call(
        "captions.apply_style",
        json!({"item":item,"style":{"kind":"preset","preset":"outline"}}),
        Some(&copied["rev_after"]),
        Some("bank-detach"),
    );
    assert_eq!(
        detached["value"]["value"]["outcome"]["changed"], true,
        "{detached}"
    );
    let deleted = client.call(
        "captions.delete_preset",
        json!({"preset":id}),
        Some(&detached["rev_after"]),
        Some("bank-delete"),
    );
    assert_eq!(
        deleted["value"]["value"]["outcome"]["changed"], true,
        "{deleted}"
    );
    let undone = client.call(
        "undo",
        json!({}),
        Some(&deleted["rev_after"]),
        Some("bank-undo"),
    );
    assert_eq!(
        undone["value"]["value"]["outcome"]["changed"], true,
        "{undone}"
    );
    let inspected = client.call("inspect", json!({}), None, None);
    assert_eq!(
        inspected["value"]["value"]["project"]["caption_presets"][0]["name"],
        "Renamed"
    );
}

#[test]
fn attach_lost_response_replays_once_and_matches_stdio_results() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), "headless.json");
    let loaded = project::load(&path).unwrap();
    let mut session = engine::Session::new(loaded.project, 100).unwrap();
    let endpoint = if cfg!(windows) {
        PathBuf::from(format!("vibecut-mcp-test-{}", std::process::id()))
    } else {
        root.path().join("private/control.sock")
    };
    let mut server = mcp::Server::bind(
        &endpoint,
        Default::default(),
        engine::ByteBudget::new(2 << 30).unwrap(),
    )
    .unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let (committed, commits) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        while !stopping.load(std::sync::atomic::Ordering::Acquire) {
            let before = session.revision();
            server.inbox.poll(&mut session, |_, _| Ok(())).unwrap();
            if before != session.revision() {
                committed.send(session.revision()).unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        session
    });
    let args = [
        "mcp".as_ref(),
        "--attach".as_ref(),
        "--socket".as_ref(),
        endpoint.as_os_str(),
    ];
    let mut attach = Client::start(&args);
    let initial = attach.call("inspect", json!({}), None, None);
    let rev = initial["rev_after"].clone();
    // Submit without receiving its response. An engine-side gate proves commit
    // before we destroy the client; the original caller never obtains the result.
    writeln!(attach.input, "{}", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"command_run","arguments":{"request_id":"request","command":"project.rename","params":{"name":"once"},"base_rev":rev,"idempotency_key":"lost"}}})).unwrap();
    attach.input.flush().unwrap();
    let committed_revision =
        serde_json::to_value(commits.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
    drop(attach);
    let mut attach = Client::start(&args);
    let applied = attach.call(
        "project.rename",
        json!({"name":"once"}),
        Some(&rev),
        Some("lost"),
    );
    assert_eq!(applied.pointer("/value/status"), Some(&json!("result")));
    assert_eq!(
        applied.pointer("/value/value/outcome/changed"),
        Some(&json!(true))
    );
    assert_eq!(applied["rev_after"], committed_revision);
    assert_eq!(
        attach.call(
            "project.rename",
            json!({"name":"once"}),
            Some(&rev),
            Some("lost")
        ),
        applied
    );
    let mut stdio = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let stdio_initial = stdio.call("inspect", json!({}), None, None);
    let stdio_applied = stdio.call(
        "project.rename",
        json!({"name":"once"}),
        Some(&stdio_initial["rev_after"]),
        Some("lost"),
    );
    assert_eq!(applied["value"], stdio_applied["value"]);
    for command in ["inspect", "provider_catalog"] {
        let mut a = attach.call(command, json!({}), None, None);
        let mut b = stdio.call(command, json!({}), None, None);
        for field in ["rev_before", "rev_after", "session_id"] {
            a.as_object_mut().unwrap().remove(field);
            b.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(a, b);
    }
    let stale_attach = attach.call(
        "project.rename",
        json!({"name":"must not change"}),
        Some(&rev),
        Some("stale-after"),
    );
    let stale_stdio = stdio.call(
        "project.rename",
        json!({"name":"must not change"}),
        Some(&stdio_initial["rev_after"]),
        Some("stale-after"),
    );
    assert_eq!(stale_attach["value"], stale_stdio["value"]);
    assert_eq!(
        stale_attach.pointer("/value/value"),
        Some(&json!("stale_revision"))
    );
    assert!(
        commits.try_recv().is_err(),
        "retry caused a second engine revision"
    );
    drop(attach);
    drop(stdio);
    stop.store(true, std::sync::atomic::Ordering::Release);
    let session = worker.join().unwrap();
    assert_eq!(session.project().data().name.as_str(), "once");
}

#[test]
fn headless_preview_returns_inline_jpeg_and_save_replays_publication_receipt() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), "preview.json");
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let initial = client.call("inspect", json!({}), None, None);
    let edit = client.call(
        "project.rename",
        json!({"name":"saved"}),
        Some(&initial["rev_after"]),
        Some("rename"),
    );
    let saved = client.call(
        "project.save",
        json!({}),
        Some(&edit["rev_after"]),
        Some("save"),
    );
    assert_eq!(saved["value"]["status"], "result");
    assert_eq!(
        client.call(
            "project.save",
            json!({}),
            Some(&edit["rev_after"]),
            Some("save")
        ),
        saved
    );
    assert_eq!(
        project::load(&path).unwrap().project.data().name.as_str(),
        "saved"
    );
    let preview = client.call(
        "preview",
        json!({"frames":[0]}),
        Some(&saved["rev_after"]),
        Some("preview"),
    );
    let job = preview.pointer("/value/value/job").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let response = client.rpc(
            4,
            "tools/call",
            json!({"name":"job_status","arguments":{"request_id":"status","params":{"job":job}}}),
        );
        if let Some(content) = response
            .pointer("/result/content")
            .and_then(Value::as_array)
            && content.iter().any(|item| item["type"] == "image")
        {
            let frame = response
                .pointer("/result/structuredContent/value/value/snapshot/output/frames/0")
                .unwrap();
            assert_eq!(frame["frame"], 0);
            assert!(
                frame["width"].as_u64().unwrap() <= 640 && frame["height"].as_u64().unwrap() <= 640
            );
            assert!(frame.get("jpeg").is_none());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "preview did not complete: {response}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn full_tool_and_command_pages_cover_the_live_registry_without_schema_drift() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), "catalog.json");
    let mut client = Client::start(&[
        "mcp".as_ref(),
        path.as_os_str(),
        "--no-save".as_ref(),
        "--all-tools".as_ref(),
    ]);
    let mut cursor = Value::Null;
    let mut names = std::collections::BTreeSet::new();
    loop {
        let params = if cursor.is_null() {
            json!({})
        } else {
            json!({"cursor":cursor})
        };
        let page = client.rpc(5, "tools/list", params);
        for tool in page["result"]["tools"].as_array().unwrap() {
            assert!(names.insert(tool["name"].as_str().unwrap().to_owned()));
        }
        cursor = page["result"]["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(names.len(), engine::registry().len() + 2);
    for spec in engine::registry() {
        assert!(names.contains(&spec.id.replace('.', "_")));
    }
    let mut entries = Vec::new();
    let mut cursor = 0;
    loop {
        let page = client.rpc(
            6,
            "tools/call",
            json!({"name":"command_list","arguments":{"cursor":cursor,"limit":16}}),
        );
        let result = &page["result"]["structuredContent"];
        assert!(result.get("error").is_none(), "{result}");
        entries.extend(result["entries"].as_array().unwrap().iter().cloned());
        // Each page carries the one result schema every command shares.
        assert_eq!(result["output_schema"], *engine::control::output_schema());
        if result["next_cursor"].is_null() {
            break;
        }
        cursor = result["next_cursor"].as_u64().unwrap();
    }
    assert_eq!(
        json!(entries),
        serde_json::to_value(engine::registry()).unwrap()
    );
    for id in [
        "captions.save_preset",
        "captions.rename_preset",
        "captions.delete_preset",
    ] {
        let spec = entries.iter().find(|e| e["id"] == id).unwrap();
        assert_eq!(spec["effect"], "edit");
        assert!(spec.get("curated").is_none());
        assert_eq!(spec["params_schema"]["additionalProperties"], false);
        assert!(spec.get("output_schema").is_none());
    }
    assert!(
        engine::control::output_schema()["$defs"]["ProjectData"]["properties"]
            .get("caption_presets")
            .is_some()
    );
    let fade = entries
        .iter()
        .find(|e| e["id"] == "text.apply_animation")
        .unwrap();
    assert_eq!(
        fade["params_schema"]["$defs"]["TextAnimationPreset"]["enum"],
        json!([
            "fade_in",
            "fade_out",
            "fade_in_out",
            "slide_in",
            "slide_out",
            "slide_in_out",
            "scale_in",
            "scale_out",
            "scale_in_out"
        ])
    );
    let initial = client.call("inspect", json!({}), None, None);
    let inserted = client.call("timeline.insert",json!({"placement":{"mode":"auto"},"item":{
        "source_asset":null,"source_range":{"start":0,"end":254016000000_i64},"start":0,
        "animations":{},"content":project::ItemParams::Text(project::TextParams::new("Tiếng Việt\nEnglish"))}}),Some(&initial["rev_after"]),Some("title"));
    let item = inserted["value"]["value"]["outcome"]["placements"][0]["item"].clone();
    assert!(!item.is_null(), "{inserted}");
    let params = json!({"item":item,"preset":"fade_in_out"});
    let applied = client.call(
        "text.apply_animation",
        params.clone(),
        Some(&inserted["rev_after"]),
        Some("fade"),
    );
    assert_eq!(applied["value"]["status"], "result", "{applied}");
    let removed = client.call(
        "keyframes.remove",
        json!({"item":item,"path":"opacity","component":null,"time":0}),
        Some(&applied["rev_after"]),
        Some("remove"),
    );
    assert_eq!(removed["value"]["status"], "result");
    assert_eq!(
        client.call(
            "text.apply_animation",
            params.clone(),
            Some(&inserted["rev_after"]),
            Some("fade")
        ),
        applied
    );
    let stale = client.call(
        "text.apply_animation",
        params,
        Some(&inserted["rev_after"]),
        Some("stale-fade"),
    );
    assert_eq!(stale["value"]["value"], "stale_revision");
    let undo = client.call(
        "undo",
        json!({}),
        Some(&removed["rev_after"]),
        Some("undo-remove"),
    );
    assert_eq!(undo["value"]["status"], "result");
    let saved = client.call(
        "project.save",
        json!({}),
        Some(&undo["rev_after"]),
        Some("save-fade"),
    );
    assert_eq!(saved["value"]["status"], "result");
    let loaded = project::load(&path).unwrap().project;
    let keys = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| json!(i.id) == item)
        .unwrap()
        .animations
        .get(&project::BuiltinPath::Opacity.into())
        .unwrap()
        .channel(None)
        .unwrap()
        .keys();
    assert_eq!(
        keys.iter().map(|k| k.time.0).collect::<Vec<_>>(),
        [0, 76_204_800_000, 177_811_200_000, 254_016_000_000]
    );
}

pub(crate) struct CaptionAsr {
    pub(crate) config: ai::ProviderConfig,
    pub(crate) calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl ai::Transcriber for CaptionAsr {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: "transcribe".into(),
            operation: ai::Operation::Transcribe,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: [ai::TimingUnit::Segment, ai::TimingUnit::Word]
                .into_iter()
                .collect(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "mcp-offline".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<Vec<ai::RecognitionSegment>, ai::Error> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(vec![ai::RecognitionSegment {
            id: "mcp-cue".into(),
            text: "Hello from MCP".into(),
            range: Some(audio.range),
            timing: ["Hello", "from", "MCP"]
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let duration = audio.range.end() - audio.range.start();
                    ai::TimedText {
                        text: (*text).into(),
                        unit: ai::TimingUnit::Word,
                        range: ai::SampleRange::new(
                            audio.range.start() + duration * index as u64 / 3,
                            audio.range.start() + duration * (index as u64 + 1) / 3,
                        )
                        .unwrap(),
                        provenance: ai::Provenance::Native,
                        speaker: None,
                        confidence: None,
                    }
                })
                .collect(),
            raw_tokens: Vec::new(),
        }])
    }
}
struct CaptionTranslator {
    config: ai::ProviderConfig,
}
impl ai::Translator for CaptionTranslator {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: "context-window".into(),
            operation: ai::Operation::Translate,
            languages: vec![scope.source_language.clone()],
            target_languages: scope.target_language.iter().cloned().collect(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "mcp-offline".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn translate(
        &self,
        window: &ai::TranslationWindow,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::Translation, ai::Error> {
        Ok(ai::Translation {
            source_language: window.source_language.clone(),
            target_language: window.target_language.clone(),
            texts: window
                .segments
                .iter()
                .map(|s| (s.id.clone(), "Xin chao".into()))
                .collect(),
            number_warnings: Vec::new(),
        })
    }
}
#[test]
fn caption_jobs_generate_apply_and_export_over_mcp_and_deny_unapproved_uploads() {
    use engine::providers::{Providers, TranscriptionProvider};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    if !crate::media_fixture::tools() {
        return;
    }
    for allow in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("caption-source.mp4");
        crate::media_fixture::generate_av(&media, 3);
        let path = root.path().join("caption-project.vibecut");
        crate::media_fixture::media_project(&path, &media, None);
        let source_bytes = std::fs::read(&media).unwrap();
        let project_bytes = std::fs::read(&path).unwrap();
        let mut session = engine::Session::new(project::load(&path).unwrap().project, 100).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let config = ai::ProviderConfig::new(
            "offline-asr".into(),
            "offline-model".into(),
            "mcp-v1".into(),
            ai::Location::Remote {
                backend: "fixture".into(),
                base_url: "https://fixture.invalid".into(),
                auth_ref: "fixture".into(),
            },
        )
        .unwrap();
        let providers = Providers::from_parts(
            TranscriptionProvider::ElevenLabs {
                model: "offline-model".into(),
            },
            "transcribe".into(),
            Arc::new(CaptionAsr {
                config,
                calls: calls.clone(),
            }),
            None,
            Some(engine::providers::TranslationProvider::OpenAi {
                model: "offline-translation-model".into(),
            }),
            Some(Arc::new(CaptionTranslator {
                config: ai::ProviderConfig::new(
                    "offline-translator".into(),
                    "offline-translation-model".into(),
                    "mcp-v1".into(),
                    ai::Location::Remote {
                        backend: "fixture".into(),
                        base_url: "https://fixture.invalid".into(),
                        auth_ref: "fixture".into(),
                    },
                )
                .unwrap(),
            })),
        )
        .unwrap();
        let (cache, _) = engine::cache::ArtifactCache::open(
            &root.path().join("cache"),
            engine::ByteBudget::new(512 << 20).unwrap(),
            16,
        )
        .unwrap();
        let prompt: Arc<dyn engine::providers::ConsentPrompt + Send + Sync> = if allow {
            Arc::new(engine::providers::AllowAll)
        } else {
            Arc::new(engine::providers::DenyUploads)
        };
        let access = engine::subtitles::CaptionAccess::new(
            Arc::new(providers),
            prompt,
            Arc::new(std::sync::Mutex::new(cache)),
        );
        let endpoint = if cfg!(windows) {
            PathBuf::from(format!("vibecut-captions-{}-{allow}", std::process::id()))
        } else {
            root.path().join("private/captions.sock")
        };
        let mut server = mcp::Server::bind(
            &endpoint,
            mcp::Grants::output(root.path()).unwrap(),
            engine::ByteBudget::new(2 << 30).unwrap(),
        )
        .unwrap();
        server.inbox.attach_captions(Arc::new(access));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                server.inbox.poll(&mut session, |_, _| Ok(())).unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
            session
        });
        let mut client = Client::start(&[
            "mcp".as_ref(),
            "--attach".as_ref(),
            "--socket".as_ref(),
            endpoint.as_os_str(),
        ]);
        let initial = client.call("inspect", json!({}), None, None);
        let rev = initial["rev_after"].clone();
        let bad = client.rpc(
            2,
            "tools/call",
            json!({"name":"command_run","arguments":{
                "request_id":"self-consent", "command":"captions.generate",
                "params":{"asset":1,"source_language":"en","consent":true},
                "base_rev":rev, "idempotency_key":"self-consent"
            }}),
        );
        assert_eq!(bad["result"]["isError"], true);
        assert_eq!(
            bad["result"]["structuredContent"]["value"]["value"],
            "invalid_params"
        );
        let accepted = client.call(
            "captions.generate",
            json!({"asset":1,"source_language":"en"}),
            Some(&rev),
            Some("generate"),
        );
        assert_eq!(accepted["value"]["status"], "result", "{accepted}");
        assert_eq!(accepted["rev_after"], rev);
        let job = accepted["value"]["value"]["job"].clone();
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            let status = client.call("job_status", json!({"job":job}), None, None);
            if matches!(
                status["value"]["value"]["snapshot"]["state"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ) {
                break status;
            }
            assert!(Instant::now() < deadline, "{status}");
            std::thread::sleep(Duration::from_millis(2));
        };
        let output = root.path().join("captions.srt");
        if allow {
            assert_eq!(
                status["value"]["value"]["snapshot"]["state"], "completed",
                "{status}"
            );
            assert_eq!(
                status["value"]["value"]["snapshot"]["output"]["kind"],
                "captions"
            );
            let applied = client.call(
                "captions.apply",
                json!({"job":job}),
                Some(&rev),
                Some("apply"),
            );
            assert_eq!(
                applied["value"]["value"]["outcome"]["changed"], true,
                "{applied}"
            );
            let track = applied["value"]["value"]["outcome"]["created"]
                .as_array()
                .unwrap()
                .iter()
                .find_map(|id| id.get("track"))
                .unwrap();
            let item = applied["value"]["value"]["outcome"]["created"]
                .as_array()
                .unwrap()
                .iter()
                .find_map(|id| id.get("item"))
                .unwrap();
            let accepted_translation = client.call(
                "captions.translate",
                json!({"item":item,"target_language":"vi"}),
                Some(&applied["rev_after"]),
                Some("translate"),
            );
            let translated =
                wait_for_media_job(&mut client, &accepted_translation["value"]["value"]["job"]);
            let translated = &translated["value"]["value"]["snapshot"]["output"];
            assert_eq!(translated["kind"], "translations");
            let retranslated=client.call("retranslate",json!({"item":item,"target_language":translated["target_language"],"texts":translated["texts"],"translation_ref":translated["translation_ref"],"force":false}),Some(&applied["rev_after"]),Some("apply-translation"));
            assert_eq!(
                retranslated["value"]["value"]["outcome"]["changed"], true,
                "{retranslated}"
            );
            let exported=client.call("export_subtitles",json!({"track":track,"format":"srt","languages":["en","vi"],"path":output,"overwrite":false}),Some(&retranslated["rev_after"]),Some("export"));
            let export_job = exported["value"]["value"]["job"].clone();
            wait_for_media_job(&mut client, &export_job);
            assert!(
                std::fs::read_to_string(&output)
                    .unwrap()
                    .contains("Hello from MCP")
            );
            assert_eq!(calls.load(Ordering::Acquire), 1);
        } else {
            assert_eq!(
                status["value"]["value"]["snapshot"]["state"], "failed",
                "{status}"
            );
            assert_eq!(status["value"]["value"]["snapshot"]["error"], "permission");
            assert!(status["value"]["value"]["snapshot"]["output"].is_null());
            assert!(!output.exists());
            assert_eq!(calls.load(Ordering::Acquire), 0);
        }
        drop(client);
        stop.store(true, Ordering::Release);
        let session = worker.join().unwrap();
        assert_eq!(std::fs::read(&media).unwrap(), source_bytes);
        assert_eq!(std::fs::read(&path).unwrap(), project_bytes);
        assert_eq!(session.history_len().0, 2 * usize::from(allow));
    }
}

/// Approves only the ASR upload: a clip suggestion upload must refuse without a human.
struct AsrOnlyConsent;
impl engine::providers::ConsentPrompt for AsrOnlyConsent {
    fn approve(&self, request: &engine::providers::UploadRequest) -> engine::providers::Result<()> {
        match request.operation {
            engine::providers::CatalogOperation::Transcribe => Ok(()),
            _ => Err(engine::providers::Error::ConsentRequired),
        }
    }
}

#[test]
fn clip_jobs_suggest_then_export_over_mcp_and_refuse_an_unapproved_remote_suggestion() {
    use engine::providers::{Credentials, Providers, TranscriptionProvider};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    if !crate::media_fixture::tools() {
        return;
    }
    // Rows: the local heuristic needs no upload and exports; a remote suggestion whose
    // upload nobody approved is a typed permission failure and writes nothing.
    for remote in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("clips-source.mp4");
        crate::media_fixture::generate_av(&media, 3);
        let path = root.path().join("clips-project.vibecut");
        crate::media_fixture::media_project(&path, &media, None);
        let mut session = engine::Session::new(project::load(&path).unwrap().project, 100).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let providers = Providers::from_parts(
            TranscriptionProvider::ElevenLabs {
                model: "offline-model".into(),
            },
            "transcribe".into(),
            Arc::new(CaptionAsr {
                config: ai::ProviderConfig::new(
                    "offline-asr".into(),
                    "offline-model".into(),
                    "mcp-v1".into(),
                    ai::Location::Remote {
                        backend: "fixture".into(),
                        base_url: "https://fixture.invalid".into(),
                        auth_ref: "fixture".into(),
                    },
                )
                .unwrap(),
                calls: calls.clone(),
            }),
            None,
            None,
            None,
        )
        .unwrap();
        let (cache, _) = engine::cache::ArtifactCache::open(
            &root.path().join("cache"),
            engine::ByteBudget::new(512 << 20).unwrap(),
            16,
        )
        .unwrap();
        let access = engine::subtitles::CaptionAccess::new(
            Arc::new(providers),
            Arc::new(AsrOnlyConsent),
            Arc::new(std::sync::Mutex::new(cache)),
        )
        .with_clips(Arc::new(shorts::ClipJobs::new(Credentials::new(
            Some("fixture-key".into()),
            None,
            None,
        ))));
        let endpoint = if cfg!(windows) {
            PathBuf::from(format!("vibecut-clips-{}-{remote}", std::process::id()))
        } else {
            root.path().join("private/clips.sock")
        };
        let mut server = mcp::Server::bind(
            &endpoint,
            mcp::Grants::output(root.path()).unwrap(),
            engine::ByteBudget::new(2 << 30).unwrap(),
        )
        .unwrap();
        server.inbox.attach_captions(Arc::new(access));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                server.inbox.poll(&mut session, |_, _| Ok(())).unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let mut client = Client::start(&[
            "mcp".as_ref(),
            "--attach".as_ref(),
            "--socket".as_ref(),
            endpoint.as_os_str(),
        ]);
        let initial = client.call("inspect", json!({}), None, None);
        let rev = initial["rev_after"].clone();
        let seconds = |value: f64| (value * time::TICKS_PER_SECOND as f64) as i64;
        let accepted = client.call(
            "clips.suggest",
            json!({
                "asset": 1,
                "source_language": "en",
                "provider": if remote { "openai" } else { "heuristic" },
                "count": 1,
                "min_duration": seconds(0.3),
                "max_duration": seconds(4.0),
                "max_segments": 1,
                "min_segment_duration": seconds(0.1),
            }),
            Some(&rev),
            Some("suggest"),
        );
        assert_eq!(accepted["value"]["status"], "result", "{accepted}");
        let job = accepted["value"]["value"]["job"].clone();
        if remote {
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                let status = client.call("job_status", json!({"job": job}), None, None);
                let state = status.pointer("/value/value/snapshot/state").cloned();
                if state != Some(json!("queued")) && state != Some(json!("running")) {
                    break status;
                }
                assert!(Instant::now() < deadline, "{status}");
                std::thread::sleep(Duration::from_millis(25));
            };
            let snapshot = &status["value"]["value"]["snapshot"];
            assert_eq!(snapshot["state"], "failed", "{status}");
            assert_eq!(snapshot["error"], "permission", "{status}");
            assert!(snapshot["output"].is_null());
            let published = std::fs::read_dir(root.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "mp4"))
                .count();
            assert_eq!(published, 1, "only the source media exists");
        } else {
            let status = wait_for_media_job(&mut client, &job);
            let output = &status["value"]["value"]["snapshot"]["output"];
            assert_eq!(output["kind"], "clips", "{status}");
            assert!(!output["clips"].as_array().unwrap().is_empty(), "{status}");
            let exported = client.call(
                "clips.export",
                json!({"job": job, "select": [0], "directory": root.path(), "overwrite": false}),
                Some(&rev),
                Some("export"),
            );
            assert_eq!(exported["value"]["status"], "result", "{exported}");
            let shorts = exported["value"]["value"]["shorts"].as_array().unwrap();
            assert_eq!(shorts.len(), 1, "{exported}");
            wait_for_media_job(&mut client, &shorts[0]["job"]);
            let video = PathBuf::from(shorts[0]["video"].as_str().unwrap());
            assert!(video.starts_with(root.path()));
            assert!(video.is_file(), "{exported}");
            assert_eq!(calls.load(Ordering::Acquire), 1);
        }
        drop(client);
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
    }
}

#[test]
fn optional_asr_setup_refusal_keeps_headless_mcp_inspection_and_edits_usable() {
    if !crate::media_fixture::tools() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("clip.mp4");
    crate::media_fixture::generate_av(&media, 3);
    let path = root.path().join("optional-asr.vibecut");
    crate::media_fixture::media_project(&path, &media, None);
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let initial = client.call("inspect", json!({}), None, None);
    let rejected = client.call(
        "captions.generate",
        json!({"asset":1,"source_language":"en"}),
        Some(&initial["rev_after"]),
        Some("missing-asr"),
    );
    assert_eq!(rejected["value"]["value"], "setup", "{rejected}");
    assert_eq!(rejected["rev_after"], initial["rev_after"]);
    let edit = client.call(
        "project.rename",
        json!({"name":"still usable"}),
        Some(&initial["rev_after"]),
        Some("rename"),
    );
    assert_eq!(edit["value"]["value"]["outcome"]["changed"], true, "{edit}");
}

#[test]
fn clipboard_command_run_freezes_keys_replays_once_and_shares_only_one_session() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("clipboard.vibecut");
    crate::cli::fixtures::overlay_project_color(
        &path,
        64,
        64,
        time::Tick(time::TICKS_PER_SECOND),
        [255; 4],
    );
    crate::cli::fixtures::exec(
        &path,
        "keyframes.set_param",
        json!({"item":0,"path":"opacity","param":{"kind":"value","channels":{"value":{"kind":"scalar","value":0.7},"keys":[{"time":-3,"value":{"kind":"scalar","value":0.25},"segment":"bezier","tangent":"aligned","incoming":{"dt":-2,"dv":0.1},"outgoing":{"dt":4,"dv":0.2}}],"before":"linear","after":"hold"}}}),
    );
    let other = root.path().join("other.vibecut");
    std::fs::copy(&path, &other).unwrap();
    let mut client = Client::start(&["mcp".as_ref(), path.as_os_str(), "--no-save".as_ref()]);
    let empty = client.call("clipboard.read", json!({}), None, None);
    assert!(empty["value"]["value"]["payload"].is_null());
    let params = json!({"keys":[{"item":0,"path":"opacity","time":-3}]});
    let copy = client.call(
        "clipboard.copy_keyframes",
        params.clone(),
        Some(&empty["rev_after"]),
        Some("copy"),
    );
    assert_eq!(
        copy["value"]["value"]["outcome"]["changed"], false,
        "{copy}"
    );
    let read = client.call("clipboard.read", json!({}), None, None);
    let payload = read["value"]["value"]["payload"].clone();
    assert_eq!(payload["origin"], -3);
    let removed = client.call(
        "keyframes.remove",
        json!({"item":0,"path":"opacity","component":null,"time":-3}),
        Some(&copy["rev_after"]),
        Some("remove"),
    );
    assert_eq!(
        client.call(
            "clipboard.copy_keyframes",
            params,
            Some(&empty["rev_after"]),
            Some("copy")
        ),
        copy
    );
    let paste_params = json!({"item":0,"at":40,"source":{"kind":"stored"}});
    let paste = client.call(
        "clipboard.paste_keyframes",
        paste_params.clone(),
        Some(&removed["rev_after"]),
        Some("paste"),
    );
    assert_eq!(
        paste["value"]["value"]["outcome"]["changed"], true,
        "{paste}"
    );
    assert_eq!(
        client.call(
            "clipboard.paste_keyframes",
            paste_params.clone(),
            Some(&removed["rev_after"]),
            Some("paste")
        ),
        paste
    );
    assert_eq!(
        client.call(
            "clipboard.paste_keyframes",
            paste_params,
            Some(&empty["rev_after"]),
            Some("stale")
        )["value"]["value"],
        "stale_revision"
    );
    let undo = client.call("undo", json!({}), Some(&paste["rev_after"]), Some("undo"));
    let undone = client.call("inspect", json!({}), None, None);
    let undone: project::ProjectData =
        serde_json::from_value(undone["value"]["value"]["project"].clone()).unwrap();
    assert!(
        undone
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == project::ItemId(0))
            .unwrap()
            .animations
            .get(&project::BuiltinPath::Opacity.into())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .is_empty()
    );
    let redo = client.call("redo", json!({}), Some(&undo["rev_after"]), Some("redo"));
    assert_eq!(redo["value"]["value"]["outcome"]["changed"], true);
    let inspected = client.call("inspect", json!({}), None, None);
    let data: project::ProjectData =
        serde_json::from_value(inspected["value"]["value"]["project"].clone()).unwrap();
    let channel = data.scene().timeline.overlays[0].items[0]
        .animations
        .get(&project::BuiltinPath::Opacity.into())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys().len(), 1);
    assert_eq!(channel.keys()[0].time, time::Tick(40));
    assert_eq!(channel.keys()[0].value, project::ParamValue::Scalar(0.25));
    assert_eq!(
        channel.keys()[0].outgoing,
        Some(project::CurveHandle {
            dt: time::Tick(4),
            dv: 0.2
        })
    );
    let mut other = Client::start(&["mcp".as_ref(), other.as_os_str(), "--no-save".as_ref()]);
    let empty = other.call("clipboard.read", json!({}), None, None);
    assert!(empty["value"]["value"]["payload"].is_null());
    let explicit = other.call(
        "clipboard.paste_keyframes",
        json!({"item":0,"at":40,"source":{"kind":"payload","payload":payload}}),
        Some(&empty["rev_after"]),
        Some("explicit"),
    );
    assert_eq!(
        explicit["value"]["value"]["outcome"]["changed"], true,
        "{explicit}"
    );
}

#[test]
fn graphic_tools_publish_typed_shapes_keys_masks_effects_undo_and_persistence() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), "graphics.vibecut");
    let mut client = Client::start(&[
        "mcp".as_ref(),
        path.as_os_str(),
        "--no-save".as_ref(),
        "--all-tools".as_ref(),
    ]);
    let mut cursor = 0;
    let mut insert_schema = None;
    loop {
        let page = client.rpc(
            397,
            "tools/call",
            json!({"name":"command_list","arguments":{"cursor":cursor,"limit":16}}),
        );
        let page = &page["result"]["structuredContent"];
        for entry in page["entries"].as_array().unwrap() {
            if entry["id"] == "timeline.insert" {
                insert_schema = Some(entry["params_schema"].clone());
            }
        }
        if page["next_cursor"].is_null() {
            break;
        }
        cursor = page["next_cursor"].as_u64().unwrap();
    }
    assert_eq!(
        insert_schema.unwrap()["$defs"]["GraphicParams"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let mut rev = client.call("inspect", json!({}), None, None)["rev_after"].clone();
    for (n, graphic) in [
        project::GraphicParams::rectangle(),
        project::GraphicParams::ellipse(),
        project::GraphicParams::polygon(),
        project::GraphicParams::star(),
    ]
    .into_iter()
    .enumerate()
    {
        let insertion_base = rev.clone();
        let params = json!({"placement":{"mode":"auto"},"item":{"source_asset":null,"source_range":{"start":0,"end":time::TICKS_PER_SECOND},"start":n as i64*time::TICKS_PER_SECOND,"animations":{},"content":project::ItemParams::Graphic(graphic)}});
        let inserted = client.call(
            "timeline.insert",
            params.clone(),
            Some(&insertion_base),
            Some(&format!("graphic{n}")),
        );
        assert_eq!(inserted["value"]["status"], "result", "{inserted}");
        rev = inserted["rev_after"].clone();
        let item = inserted["value"]["value"]["outcome"]["placements"][0]["item"].clone();
        assert!(item.is_u64());
        for (step,(command,params)) in [
            ("timeline.set_item_param",json!({"item":item,"patch":{"field":"graphic_stroke_width","value":12.}})),
            ("keyframes.set_param",json!({"item":item,"path":"params.fill","param":project::Param::new(project::ParamValue::Color([0.25,0.5,1.,0.5])).unwrap()})),
            ("keyframes.add",json!({"item":item,"path":"params.fill","component":"a","key":project::Keyframe::new(time::Tick(0),project::ParamValue::Scalar(0.25))})),
            ("masks.add",json!({"item":item,"mask":project::Mask::new("graphic-mask",project::MaskShape::Ellipse)})),
            ("effects.add",json!({"item":item,"effect":project::effect_definition("blur").unwrap().instance("graphic-effect")})),
        ].into_iter().enumerate() {
            let response=client.call(command,params,Some(&rev),Some(&format!("graphic{n}-{step}")));
            assert_eq!(response["value"]["status"],"result","{response}"); rev=response["rev_after"].clone();
        }
        assert_eq!(
            client.call(
                "timeline.insert",
                params.clone(),
                Some(&insertion_base),
                Some(&format!("graphic{n}"))
            ),
            inserted
        );
        let stale = client.call(
            "timeline.insert",
            params,
            Some(&insertion_base),
            Some(&format!("stale-graphic{n}")),
        );
        assert_eq!(stale["value"]["value"], "stale_revision");
        let undo = client.call("undo", json!({}), Some(&rev), Some(&format!("undo{n}")));
        assert_eq!(undo["value"]["status"], "result");
        rev = undo["rev_after"].clone();
        let redo = client.call("redo", json!({}), Some(&rev), Some(&format!("redo{n}")));
        assert_eq!(redo["value"]["status"], "result");
        rev = redo["rev_after"].clone();
    }
    let saved = client.call("project.save", json!({}), Some(&rev), Some("graphics-save"));
    assert_eq!(saved["value"]["status"], "result", "{saved}");
    let p = project::load(&path).unwrap().project;
    let items = &p.data().scene().timeline.main.items;
    assert_eq!(items.len(), 4);
    for item in items {
        assert!(matches!(item.content, project::ItemParams::Graphic(_)));
        assert_eq!(item.masks.len(), 1);
        assert_eq!(item.effects.len(), 1);
        assert_eq!(
            item.animations
                .value_at(
                    &project::AnimationPath::new("params.fill").unwrap(),
                    time::Tick(0)
                )
                .unwrap(),
            Some(project::ParamValue::Color([0.25, 0.5, 1., 0.25]))
        );
    }
}
