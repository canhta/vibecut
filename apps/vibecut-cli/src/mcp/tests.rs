//! Verifies the owning module interface.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use engine::{
    ByteBudget, Command, EditError, Session,
    control::{self, Policy},
};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

#[test]
fn output_grant_rejects_unselected_parent_before_filesystem_resolution() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let grant = Grants::output(&output).unwrap();
    let project = project::Project::empty(project::ProjectId(1), "grant".into()).unwrap();
    let export = |path| Command::Export {
        settings: render::OutputOptions::default(),
        path,
        overwrite: false,
    };
    assert!(
        grant
            .authorize(&export(output.join("out.mp4")), &project)
            .is_ok()
    );
    assert!(
        grant
            .authorize(
                &export(std::fs::canonicalize(&output).unwrap().join("out.mp4")),
                &project
            )
            .is_ok()
    );
    assert!(matches!(
        grant.authorize(&export(root.path().join("other/out.mp4")), &project),
        Err(EditError::Permission)
    ));
    // Every path-publishing command shares the grant; none may write outside it.
    let snapshot = |path| Command::Snapshot {
        time: time::Tick::ZERO,
        path,
    };
    assert!(
        grant
            .authorize(&snapshot(output.join("frame.png")), &project)
            .is_ok()
    );
    assert!(matches!(
        grant.authorize(&snapshot(root.path().join("other/frame.png")), &project),
        Err(EditError::Permission)
    ));
    assert!(matches!(
        Grants::default().authorize(&snapshot(output.join("frame.png")), &project),
        Err(EditError::Permission)
    ));
    #[cfg(unix)]
    {
        let alias = root.path().join("unselected-alias");
        std::os::unix::fs::symlink(&output, &alias).unwrap();
        assert!(matches!(
            grant.authorize(&export(alias.join("out.mp4")), &project),
            Err(EditError::Permission)
        ));
    }
}

async fn send(write: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, value: Value) {
    write
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
}
async fn response(read: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> Value {
    let mut line = String::new();
    read.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

#[tokio::test(start_paused = true)]
async fn hung_calls_expire_and_cancelled_queued_mutations_never_execute() {
    let (handler, mut inbox) =
        Inbox::new(Grants::default(), ByteBudget::new(2 << 30).unwrap()).unwrap();
    let (server, client) = tokio::io::duplex(65536);
    let (server_read, server_write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        handler
            .serve(transport::Framed::new(server_read, server_write))
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let (read, mut write) = tokio::io::split(client);
    let mut read = BufReader::new(read);
    send(&mut write, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"conformance","version":"1"}}})).await;
    assert!(response(&mut read).await.get("result").is_some());
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(1), "initial".into()).unwrap(),
        100,
    )
    .unwrap();
    let initial = session.revision();
    let request = json!({"command":"project.rename","params":{"name":"must not execute"},"base_rev":initial,"idempotency_key":"queued","request_id":"q"});
    send(&mut write, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_run","arguments":request}})).await;
    // Poll until queued, without allowing the engine owner to process it.
    while inbox.receive.is_empty() {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(DEADLINE + Duration::from_secs(1)).await;
    let timeout = response(&mut read).await;
    assert_eq!(
        timeout.pointer("/result/structuredContent/error/code"),
        Some(&json!("timeout_unknown_outcome"))
    );
    inbox.poll(&mut session, |_, _| Ok(())).unwrap();
    assert_eq!(session.revision(), initial);

    let request = json!({"command":"project.rename","params":{"name":"must not execute"},"base_rev":initial,"idempotency_key":"cancelled","request_id":"c"});
    send(&mut write, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"command_run","arguments":request}})).await;
    while inbox.receive.is_empty() {
        tokio::task::yield_now().await;
    }
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}),
    )
    .await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    inbox.poll(&mut session, |_, _| Ok(())).unwrap();
    assert_eq!(session.revision(), initial);
    task.abort();
}

#[tokio::test]
async fn partial_frames_survive_receive_cancellation_and_hostile_framing_closes() {
    use rmcp::transport::Transport as _;
    for hostile in [
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"id\":2,\"method\":\"ping\"}\n".to_vec(),
        vec![b'x'; control::MAX_FRAME_BYTES + 1],
    ] {
        let (server, mut client) = tokio::io::duplex(control::MAX_FRAME_BYTES + 4096);
        let (read, write) = tokio::io::split(server);
        let mut transport = transport::Framed::new(read, write);
        client.write_all(&hostile).await.unwrap();
        assert!(transport.receive().await.is_none());
    }
    let (server, mut client) = tokio::io::duplex(1024);
    let (read, write) = tokio::io::split(server);
    let mut transport = transport::Framed::new(read, write);
    client.write_all(b"{\"jsonrpc\":\"2.0\",").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), transport.receive())
            .await
            .is_err()
    );
    client
        .write_all(b"\"id\":1,\"method\":\"ping\"}\n")
        .await
        .unwrap();
    assert!(transport.receive().await.is_some());
}

#[tokio::test]
async fn message_flood_closes_before_unbounded_sdk_task_creation() {
    use rmcp::transport::Transport as _;
    let (server, mut client) = tokio::io::duplex(65536);
    let (read, write) = tokio::io::split(server);
    let mut transport = transport::Framed::new(read, write);
    let message = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
    for _ in 0..65 {
        client.write_all(message).await.unwrap();
    }
    for _ in 0..64 {
        assert!(transport.receive().await.is_some());
    }
    assert!(transport.receive().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn slow_reader_bounds_output_and_close_cancels_the_writer() {
    use rmcp::transport::Transport as _;
    let (server, _client) = tokio::io::duplex(1);
    let (read, write) = tokio::io::split(server);
    let mut transport = transport::Framed::new(read, write);
    let message: rmcp::model::ServerJsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"result":{"tools":[]}})).unwrap();
    // No await until the queue is full: the writer has not been scheduled yet.
    for _ in 0..4 {
        drop(transport.send(message.clone()));
    }
    assert_eq!(
        transport.send(message).await.unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        transport.close().await.unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
}

#[test]
fn every_registry_command_has_an_explicit_grant_and_effect() {
    use engine::Effect;
    // Deliberately independent of the registry: additions and effect changes need review.
    let cases = [
        ("project.create", Effect::SessionControl, false, false),
        ("scene.create", Effect::Edit, true, true),
        ("scene.delete", Effect::Edit, true, true),
        ("scene.switch", Effect::SessionControl, true, true),
        ("scene.rename", Effect::Edit, true, true),
        ("project.rename", Effect::Edit, true, true),
        ("project.duplicate", Effect::SessionControl, false, false),
        ("project.delete", Effect::SessionControl, false, false),
        ("project.thumbnail", Effect::SessionControl, false, false),
        ("project.save", Effect::SessionControl, true, true),
        ("audio.set_master", Effect::Edit, true, true),
        ("project.set_view_state", Effect::SessionControl, true, true),
        ("export_subtitles", Effect::FilePublish, true, false),
        ("burn_subtitles", Effect::FilePublish, true, false),
        ("import.add_asset", Effect::Edit, false, false),
        ("media.remove", Effect::Edit, true, true),
        ("timeline.add_track", Effect::Edit, true, true),
        ("track.remove", Effect::Edit, true, true),
        ("track.set_muted", Effect::Edit, true, true),
        ("track.set_hidden", Effect::Edit, true, true),
        ("track.set_magnet", Effect::Edit, true, true),
        ("track.set_flags", Effect::Edit, true, true),
        ("track.rename", Effect::Edit, true, true),
        ("track.reorder", Effect::Edit, true, true),
        ("track.set_locked", Effect::Edit, true, true),
        ("timeline.insert", Effect::Edit, true, true),
        ("sounds.add_to_timeline", Effect::Edit, false, false),
        ("captions.import", Effect::Edit, false, false),
        ("transcript.delete_range", Effect::Edit, true, true),
        ("move_item", Effect::Edit, true, true),
        ("move_items", Effect::Edit, true, true),
        ("timeline.move_items", Effect::Edit, true, true),
        ("timeline.trim", Effect::Edit, true, true),
        ("timeline.pause_candidates", Effect::JobControl, true, true),
        ("timeline.remove_pauses", Effect::Edit, true, true),
        ("split_at", Effect::Edit, true, true),
        ("trim_start", Effect::Edit, true, true),
        ("trim_end", Effect::Edit, true, true),
        ("delete", Effect::Edit, true, true),
        ("ripple_delete", Effect::Edit, true, true),
        ("timeline.split_left", Effect::Edit, true, true),
        ("timeline.split_right", Effect::Edit, true, true),
        ("timeline.duplicate", Effect::Edit, true, true),
        ("timeline.set_hidden", Effect::Edit, true, true),
        ("timeline.set_muted", Effect::Edit, true, true),
        ("timeline.extract_audio", Effect::Edit, true, true),
        ("timeline.recover_audio", Effect::Edit, true, true),
        ("set_item_param", Effect::Edit, true, true),
        ("timeline.set_item_param", Effect::Edit, true, true),
        ("timeline.set_speed", Effect::Edit, true, true),
        ("project.set_canvas", Effect::Edit, true, true),
        ("project.set_fps", Effect::Edit, true, true),
        ("project.set_background", Effect::Edit, true, true),
        ("timeline.set_item_param_many", Effect::Edit, true, true),
        ("audio.set_fade", Effect::Edit, true, true),
        ("audio.set_loudness", Effect::Edit, true, true),
        ("audio.clear_loudness", Effect::Edit, true, true),
        ("audio.apply_loudness", Effect::Edit, true, true),
        ("transition.add", Effect::Edit, true, true),
        ("transition.move", Effect::Edit, true, true),
        ("transition.set_duration", Effect::Edit, true, true),
        ("transition.remove", Effect::Edit, true, true),
        ("masks.add", Effect::Edit, true, true),
        ("masks.remove", Effect::Edit, true, true),
        ("masks.update", Effect::Edit, true, true),
        ("masks.reorder", Effect::Edit, true, true),
        ("effects.add", Effect::Edit, true, true),
        ("effects.remove", Effect::Edit, true, true),
        ("effects.update_param", Effect::Edit, true, true),
        ("effects.reorder", Effect::Edit, true, true),
        ("effects.toggle", Effect::Edit, true, true),
        ("keyframes.set_param", Effect::Edit, true, true),
        ("keyframes.add", Effect::Edit, true, true),
        ("keyframes.remove", Effect::Edit, true, true),
        ("keyframes.retime", Effect::Edit, true, true),
        ("keyframes.set_handle", Effect::Edit, true, true),
        ("keyframes.scale_time", Effect::Edit, true, true),
        ("keyframes.toggle_at", Effect::Edit, true, true),
        ("keyframes.delete_selection", Effect::Edit, true, true),
        ("keyframes.set_extrapolation", Effect::Edit, true, true),
        ("retranslate", Effect::Edit, true, true),
        ("bookmark.add", Effect::Edit, true, true),
        ("bookmark.move", Effect::Edit, true, true),
        ("bookmark.toggle", Effect::Edit, true, true),
        ("bookmark.update", Effect::Edit, true, true),
        ("bookmark.remove", Effect::Edit, true, true),
        ("batch", Effect::Edit, true, true),
        ("timeline.edit", Effect::Edit, true, true),
        ("timeline.set_ripple", Effect::SessionControl, true, true),
        ("begin_gesture", Effect::SessionControl, false, false),
        ("timeline.preview", Effect::SessionControl, false, false),
        (
            "timeline.commit_preview",
            Effect::SessionControl,
            false,
            false,
        ),
        (
            "timeline.discard_preview",
            Effect::SessionControl,
            false,
            false,
        ),
        ("commit_gesture", Effect::SessionControl, false, false),
        ("cancel_gesture", Effect::SessionControl, false, false),
        ("undo", Effect::SessionControl, true, true),
        ("redo", Effect::SessionControl, true, true),
        ("inspect", Effect::Read, true, true),
        ("preview", Effect::JobControl, true, true),
        ("preview.snapshot", Effect::FilePublish, true, false),
        ("export", Effect::FilePublish, true, false),
        ("job_status", Effect::Read, true, true),
        ("job_cancel", Effect::JobControl, true, true),
        ("catalog", Effect::Read, true, true),
        ("provider_catalog", Effect::Read, true, true),
        ("fonts.list", Effect::Read, true, true),
        ("fonts.load", Effect::Read, true, true),
        ("fonts.download", Effect::JobControl, false, false),
    ];
    let root = tempfile::tempdir().unwrap();
    let grant = Grants::output(root.path()).unwrap();
    let project = project::Project::empty(project::ProjectId(1), "policy".into()).unwrap();
    let registry = engine::registry();
    let encoded_root = serde_json::to_string(root.path()).unwrap();
    assert_eq!(cases.len(), registry.len());
    assert_eq!(
        cases
            .iter()
            .map(|row| row.0)
            .collect::<std::collections::BTreeSet<_>>(),
        registry.iter().map(|spec| spec.id).collect()
    );
    for (id, effect, with_output, without_output) in cases {
        let spec = registry.iter().find(|spec| spec.id == id).unwrap();
        assert_eq!(spec.effect, effect, "{id}");
        // The declared class must agree with the reviewed decision in this table.
        assert_eq!(
            spec.grant,
            Some(match (with_output, without_output) {
                (true, true) => engine::GrantClass::Agent,
                (true, false) => engine::GrantClass::Output,
                (false, _) => engine::GrantClass::Human,
            }),
            "{id}"
        );
        let mut params: Value = serde_json::from_str(
            &serde_json::to_string(&spec.example.params)
                .unwrap()
                .replace("$output", encoded_root.trim_matches('"')),
        )
        .unwrap();
        if effect == Effect::FilePublish {
            params["path"] = json!(root.path().join("output"));
        }
        let command: Command =
            serde_json::from_value(json!({"command":id,"params":params})).unwrap();
        for (grants, expected) in [(&grant, with_output), (&Grants::default(), without_output)] {
            let result = grants.authorize(&command, &project);
            if expected {
                assert!(result.is_ok(), "{id}: {result:?}");
            } else {
                assert!(
                    matches!(result, Err(EditError::Permission)),
                    "{id}: {result:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn desktop_project_routing_selects_explicit_or_focused_session_and_refuses_invalid_targets() {
    let (handler, mut inbox) =
        Inbox::new(Grants::default(), ByteBudget::new(768 << 20).unwrap()).unwrap();
    let mut first = Session::new(
        project::Project::empty(project::ProjectId(1), "first".into()).unwrap(),
        100,
    )
    .unwrap();
    let mut second = Session::new(
        project::Project::empty(project::ProjectId(2), "second".into()).unwrap(),
        100,
    )
    .unwrap();
    for (index, target, expected) in [
        (0, Some(json!("2")), Some(2)),
        (1, None, Some(1)),
        (2, Some(json!(2)), None),
        (3, Some(json!("999")), None),
        (4, Some(Value::Null), None),
    ] {
        let revision = if target.as_ref().and_then(Value::as_str) == Some("2") {
            second.revision()
        } else {
            first.revision()
        };
        let mut value = json!({"command":"project.rename","request_id":format!("route-{index}"),
            "params":{"name":format!("edit-{index}")},"base_rev":revision,"idempotency_key":format!("route-{index}")});
        if let Some(target) = target {
            value["project_id"] = target;
        }
        let (reply, receive) = oneshot::channel();
        handler
            .send
            .send(Pending {
                value,
                expires: Instant::now() + DEADLINE,
                reply,
            })
            .await
            .unwrap();
        inbox
            .poll_projects(|target, inbox, value| {
                let session = match target.unwrap_or(project::ProjectId(1)) {
                    project::ProjectId(1) => &mut first,
                    project::ProjectId(2) => &mut second,
                    _ => return Err(EditError::SessionGone),
                };
                inbox.execute_project(session, value, |_, _| Err(EditError::Permission))
            })
            .unwrap();
        let result = receive.await.unwrap();
        if let Some(id) = expected {
            let response = result.unwrap();
            assert_eq!(
                response.pointer("/value/status"),
                Some(&json!("result")),
                "{response}"
            );
            assert_eq!(
                if id == 1 {
                    first.project()
                } else {
                    second.project()
                }
                .data()
                .name
                .as_str(),
                format!("edit-{index}")
            );
        } else {
            assert!(result.is_err());
        }
    }
    assert_eq!(first.project().data().name.as_str(), "edit-1");
    assert_eq!(second.project().data().name.as_str(), "edit-0");
    inbox.retain_projects(|id| id == project::ProjectId(2));
    assert!(!inbox.projects.contains_key(&project::ProjectId(1)));
    assert!(inbox.projects.contains_key(&project::ProjectId(2)));
}
