#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Client {
    child: Child,
    input: ChildStdin,
    responses: mpsc::Receiver<Value>,
}
impl Client {
    fn start(args: &[&std::ffi::OsStr]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vibecut-cli"))
            .args(args)
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
    let mut server = vibecut_cli::mcp::Server::bind(
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
                .pointer("/result/structuredContent/value/value/snapshot/result/frames/0")
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
        if result["next_cursor"].is_null() {
            break;
        }
        cursor = result["next_cursor"].as_u64().unwrap();
    }
    assert_eq!(
        json!(entries),
        serde_json::to_value(engine::registry()).unwrap()
    );
}
