//! Verifies the owning module interface.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use engine::providers::TextAccess;
use engine::{
    ByteBudget, Command, Session,
    control::Error as EngineError,
    control::{self, Policy},
};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::sync::Arc;
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
        Err(EngineError::Control(
            engine::control::ControlError::Permission
        ))
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
        Err(EngineError::Control(
            engine::control::ControlError::Permission
        ))
    ));
    assert!(matches!(
        Grants::default().authorize(&snapshot(output.join("frame.png")), &project),
        Err(EngineError::Control(
            engine::control::ControlError::Permission
        ))
    ));
    let document: ai::chapters::ChapterDocument = serde_json::from_value(
        engine::registry()
            .iter()
            .find(|entry| entry.id == "chapters.export_sidecars")
            .unwrap()
            .example
            .params["document"]
            .clone(),
    )
    .unwrap();
    let sidecars = |dir| Command::ChaptersExportSidecars {
        dir,
        document: document.clone(),
    };
    assert!(grant.authorize(&sidecars(output.clone()), &project).is_ok());
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut refusals = vec![
        (
            "file command targeting granted directory",
            grant.authorize(&export(output.clone()), &project).is_err(),
        ),
        (
            "sidecars targeting a file",
            grant
                .authorize(&sidecars(output.join("out.mp4")), &project)
                .is_err(),
        ),
    ];
    #[cfg(unix)]
    {
        let alias = root.path().join("unselected-alias");
        std::os::unix::fs::symlink(&output, &alias).unwrap();
        refusals.push((
            "unselected directory alias",
            grant.authorize(&sidecars(alias.clone()), &project).is_err(),
        ));
        refusals.push((
            "file command targeting alias directory",
            grant.authorize(&export(alias.clone()), &project).is_err(),
        ));
        // A symlink spelling explicitly selected by the human is a legitimate grant.
        let selected = root.path().join("selected-alias");
        std::os::unix::fs::symlink(&output, &selected).unwrap();
        let selected_grant = Grants::output(&selected).unwrap();
        for directory in [selected.clone(), std::fs::canonicalize(&output).unwrap()] {
            assert!(
                selected_grant
                    .authorize(&sidecars(directory.clone()), &project)
                    .is_ok()
            );
            assert!(
                selected_grant
                    .authorize(&export(directory.join("selected.mp4")), &project)
                    .is_ok()
            );
        }
        assert!(matches!(
            grant.authorize(&export(alias.join("out.mp4")), &project),
            Err(EngineError::Control(
                engine::control::ControlError::Permission
            ))
        ));
    }
    assert!(
        refusals.iter().all(|(_, refused)| *refused),
        "grant refusal cases: {refusals:?}"
    );
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

#[tokio::test]
async fn lut_source_import_is_denied_on_real_mcp_wire_before_job_or_io() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("private.cube");
    std::fs::write(&source, b"must not be parsed").unwrap();
    let budget = ByteBudget::new(128 << 20).unwrap();
    let (handler, mut inbox) = Inbox::new(Grants::default(), budget.clone()).unwrap();
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
    send(&mut write,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"lut-policy","version":"1"}}})).await;
    assert!(response(&mut read).await.get("result").is_some());
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let mut data = project::Project::empty(project::ProjectId(1), "LUT policy".into())
        .unwrap()
        .into_data();
    let id: project::LutId = serde_json::from_value(json!("01".repeat(32))).unwrap();
    data.luts.push(project::LutRecord {
        id,
        name: "Registered look".into(),
        licence: None,
        source_name: "look.cube".into(),
        source_hash: serde_json::from_value(json!(vec![0u8; 32])).unwrap(),
        directory: root.path().join("already-registered"),
    });
    data.scene_mut().timeline.main.items.push(project::Item {
        id: project::ItemId(1),
        source_asset: None,
        source_range: time::SourceRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND))
            .unwrap(),
        start: time::Tick::ZERO,
        content: project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([80, 120, 200, 255]),
        }),
        effects: vec![
            project::effect_definition("color_grade")
                .unwrap()
                .instance("grade"),
        ],
        masks: vec![],
        animations: Default::default(),
        retime: Default::default(),
        blend_mode: Default::default(),
        hidden: false,
    });
    let mut session = Session::new(project::Project::new(data).unwrap(), 10).unwrap();
    let revision = session.revision();
    let before = session.project().clone();
    send(&mut write,json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_run","arguments":{"command":"color.import_lut","params":{"path":source},"base_rev":revision,"idempotency_key":"lut-source","request_id":"lut-source"}}})).await;
    while inbox.receive.is_empty() {
        tokio::task::yield_now().await;
    }
    inbox
        .poll_projects(|_, inbox, value| {
            inbox.execute_project(&mut session, value, |_, _| {
                panic!("import cannot save project")
            })
        })
        .unwrap();
    let denied = response(&mut read).await;
    assert_eq!(
        denied.pointer("/result/structuredContent/value/value"),
        Some(&json!("permission")),
        "{denied}"
    );
    assert_eq!(session.project(), &before);
    assert_eq!(session.revision(), revision);
    assert!(
        inbox
            .projects
            .values()
            .all(|runner| runner.snapshots().is_empty())
    );
    assert_eq!(std::fs::read(source).unwrap(), b"must not be parsed");
    // Agent edits may name an existing resource without reading any path. An
    // absent bundle here also proves selection itself does no filesystem work.
    send(&mut write,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"command_run","arguments":{"command":"effects.update_param","params":{"item":1,"effect":"grade","key":"lut","value":{"lut":id}},"base_rev":revision,"idempotency_key":"lut-select","request_id":"lut-select"}}})).await;
    while inbox.receive.is_empty() {
        tokio::task::yield_now().await;
    }
    let mut saved = false;
    inbox
        .poll_projects(|_, inbox, value| {
            inbox.execute_project(&mut session, value, |_, _| {
                saved = true;
                Ok(())
            })
        })
        .unwrap();
    let selected = response(&mut read).await;
    assert_eq!(
        selected.pointer("/result/isError"),
        Some(&json!(false)),
        "{selected}"
    );
    assert!(!saved);
    assert_eq!(session.history_len(), (1, 0));
    assert_ne!(session.revision(), revision);
    assert_eq!(
        session.project().data().scene().timeline.main.items[0].effects[0].value("lut"),
        Some(&project::EffectValue::Lut(Some(id)))
    );
    assert!(!root.path().join("already-registered").exists());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
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
    let path = project::BuiltinPath::Opacity.into();
    let mut animations = project::Animations::default();
    animations
        .set(
            path,
            project::Param::Value(
                project::Channel::new(
                    project::ParamValue::Scalar(0.7),
                    vec![
                        project::Keyframe::new(time::Tick(0), project::ParamValue::Scalar(0.25)),
                        project::Keyframe::new(time::Tick(12), project::ParamValue::Scalar(0.75)),
                    ],
                    project::Extrapolation::Hold,
                    project::Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let inserted = session
        .apply(engine::Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                animations,
                source_asset: None,
                source_range: time::SourceRange::new(
                    time::Tick::ZERO,
                    time::Tick(time::TICKS_PER_SECOND),
                )
                .unwrap(),
                start: time::Tick::ZERO,
                retime: Default::default(),
                content: project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([255; 4]),
                }),
            },
        })
        .unwrap();
    let item = inserted
        .created
        .iter()
        .find_map(|i| {
            if let engine::CreatedId::Item(i) = i {
                Some(*i)
            } else {
                None
            }
        })
        .unwrap();
    session
        .apply(engine::Command::CopyKeyframes {
            keys: vec![engine::KeyframeRef {
                item,
                path: project::BuiltinPath::Opacity.into(),
                component: None,
                time: time::Tick::ZERO,
            }],
        })
        .unwrap();
    let before = session.project().clone();
    let history = session.history_len();
    let initial = session.revision();
    let request = json!({"command":"clipboard.copy_keyframes","params":{"keys":[{"item":item,"path":"opacity","time":12}]},"base_rev":initial,"idempotency_key":"queued","request_id":"q"});
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

    let request = json!({"command":"clipboard.paste_keyframes","params":{"item":item,"at":40,"source":{"kind":"stored"}},"base_rev":initial,"idempotency_key":"cancelled","request_id":"c"});
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
    assert_eq!(session.project(), &before);
    assert_eq!(session.history_len(), history);
    session
        .apply(engine::Command::CopyKeyframes {
            keys: vec![engine::KeyframeRef {
                item,
                path: project::BuiltinPath::Opacity.into(),
                component: None,
                time: time::Tick::ZERO,
            }],
        })
        .unwrap();
    assert_eq!(session.revision(), initial);
    task.abort();
}

#[tokio::test]
async fn partial_frames_survive_receive_cancellation_and_hostile_framing_closes() {
    use rmcp::transport::Transport as _;
    for hostile in [
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"id\":2,\"method\":\"ping\"}\n".to_vec(),
        vec![b'x'; (16 << 20) + 1],
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
        ("project.restore", Effect::SessionControl, false, false),
        ("project.purge", Effect::SessionControl, false, false),
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
        ("sounds.save", Effect::Read, false, false),
        ("sounds.search", Effect::Read, false, false),
        ("sounds.unsave", Effect::Read, false, false),
        ("music.add_to_timeline", Effect::Edit, false, false),
        ("music.save", Effect::Read, false, false),
        ("music.search", Effect::Read, false, false),
        ("music.unsave", Effect::Read, false, false),
        ("captions.import", Effect::Edit, false, false),
        ("captions.apply_style", Effect::Edit, true, true),
        ("captions.save_preset", Effect::Edit, true, true),
        ("captions.rename_preset", Effect::Edit, true, true),
        ("captions.delete_preset", Effect::Edit, true, true),
        (
            "clipboard.copy_keyframes",
            Effect::SessionControl,
            true,
            true,
        ),
        ("clipboard.paste_keyframes", Effect::Edit, true, true),
        ("clipboard.read", Effect::Read, true, true),
        ("text.apply_animation", Effect::Edit, true, true),
        ("text.apply_preset", Effect::Edit, true, true),
        ("text.save_preset", Effect::Edit, true, true),
        ("transcript.delete_range", Effect::Edit, true, true),
        ("transcript.merge_cues", Effect::Edit, true, true),
        ("timeline.filler_candidates", Effect::Read, true, true),
        ("timeline.remove_fillers", Effect::Edit, true, true),
        ("chapters.generate", Effect::JobControl, true, true),
        ("chapters.export_sidecars", Effect::FilePublish, true, false),
        ("fillers.classify", Effect::JobControl, true, true),
        ("move_item", Effect::Edit, true, true),
        ("move_items", Effect::Edit, true, true),
        ("timeline.move_items", Effect::Edit, true, true),
        ("timeline.trim", Effect::Edit, true, true),
        ("timeline.pause_candidates", Effect::JobControl, true, true),
        ("media.generate_waveform", Effect::JobControl, true, true),
        ("media.detect_scenes", Effect::JobControl, true, true),
        (
            "media.generate_clip_thumbnails",
            Effect::JobControl,
            true,
            true,
        ),
        ("timeline.remove_pauses", Effect::Edit, true, true),
        ("split_at", Effect::Edit, true, true),
        ("trim_start", Effect::Edit, true, true),
        ("trim_end", Effect::Edit, true, true),
        ("delete", Effect::Edit, true, true),
        ("ripple_delete", Effect::Edit, true, true),
        ("timeline.split_left", Effect::Edit, true, true),
        ("timeline.split_right", Effect::Edit, true, true),
        ("timeline.duplicate", Effect::Edit, true, true),
        ("timeline.set_flipped", Effect::Edit, true, true),
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
        ("audio.measure_loudness", Effect::JobControl, true, true),
        ("audio.set_loudness", Effect::Edit, true, true),
        ("audio.clear_loudness", Effect::Edit, true, true),
        ("audio.apply_loudness", Effect::Edit, true, true),
        ("transition.add", Effect::Edit, true, true),
        ("transition.move", Effect::Edit, true, true),
        ("transition.set_duration", Effect::Edit, true, true),
        ("transition.set_param", Effect::Edit, true, true),
        ("transition.remove", Effect::Edit, true, true),
        ("masks.add", Effect::Edit, true, true),
        ("masks.add_point", Effect::Edit, true, true),
        ("masks.remove", Effect::Edit, true, true),
        ("masks.remove_point", Effect::Edit, true, true),
        ("masks.set_path", Effect::Edit, true, true),
        ("masks.update", Effect::Edit, true, true),
        ("masks.reorder", Effect::Edit, true, true),
        ("effects.add", Effect::Edit, true, true),
        ("effects.remove", Effect::Edit, true, true),
        ("effects.update_param", Effect::Edit, true, true),
        ("color.import_lut", Effect::JobControl, false, false),
        ("color.register_lut", Effect::Edit, false, false),
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
        ("reframe.apply", Effect::Edit, true, true),
        ("reframe.clear", Effect::Edit, true, true),
        ("reframe.generate", Effect::JobControl, true, true),
        ("reframe.set_manual", Effect::Edit, true, true),
        ("captions.apply", Effect::Edit, true, true),
        ("captions.set_word_highlight", Effect::Edit, true, true),
        ("captions.generate", Effect::JobControl, true, true),
        ("clips.suggest", Effect::JobControl, true, true),
        ("clips.export", Effect::FilePublish, true, false),
        ("captions.translate", Effect::JobControl, true, true),
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
        ("fonts.import", Effect::JobControl, false, false),
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
        if effect == Effect::FilePublish && params.get("path").is_some() {
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
                    matches!(
                        result,
                        Err(EngineError::Control(
                            engine::control::ControlError::Permission
                        ))
                    ),
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
                    _ => {
                        return Err(EngineError::Control(
                            engine::control::ControlError::SessionGone,
                        ));
                    }
                };
                inbox.execute_project(session, value, |_, _| {
                    Err(EngineError::Control(
                        engine::control::ControlError::Permission,
                    ))
                })
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
    // A desktop typed Copy is visible through the same routed Session's inbox.
    let mut animations = project::Animations::default();
    let mut opacity = project::Param::new(project::ParamValue::Scalar(0.7)).unwrap();
    opacity
        .channel_mut(None)
        .unwrap()
        .put(project::Keyframe::new(
            time::Tick(-3),
            project::ParamValue::Scalar(0.25),
        ))
        .unwrap();
    animations
        .set(project::BuiltinPath::Opacity.into(), opacity)
        .unwrap();
    let added = first
        .apply(engine::Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                animations,
                source_asset: None,
                source_range: time::SourceRange::new(
                    time::Tick::ZERO,
                    time::Tick(time::TICKS_PER_SECOND),
                )
                .unwrap(),
                start: time::Tick::ZERO,
                retime: Default::default(),
                content: project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([255; 4]),
                }),
            },
        })
        .unwrap();
    let item = added
        .created
        .iter()
        .find_map(|id| {
            if let engine::CreatedId::Item(id) = id {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    first
        .apply(engine::Command::CopyKeyframes {
            keys: vec![engine::KeyframeRef {
                item,
                path: project::BuiltinPath::Opacity.into(),
                component: None,
                time: time::Tick(-3),
            }],
        })
        .unwrap();
    for (project, nonempty) in [(Some("1"), true), (Some("2"), false), (None, true)] {
        let mut value =
            json!({"command":"clipboard.read","request_id":"clipboard-route","params":{}});
        if let Some(project) = project {
            value["project_id"] = json!(project);
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
                let session = if target.unwrap_or(project::ProjectId(1)) == project::ProjectId(1) {
                    &mut first
                } else {
                    &mut second
                };
                inbox.execute_project(session, value, |_, _| {
                    Err(EngineError::Control(
                        engine::control::ControlError::Permission,
                    ))
                })
            })
            .unwrap();
        let response = receive.await.unwrap().unwrap();
        assert_eq!(
            response.pointer("/value/value/kind"),
            Some(&json!("clipboard"))
        );
        if nonempty {
            assert_eq!(
                response.pointer("/value/value/payload/origin"),
                Some(&json!(-3))
            );
        } else {
            assert_eq!(response.pointer("/value/value/payload"), Some(&Value::Null));
        }
    }
    inbox.retain_projects(|id| id == project::ProjectId(2));
    assert!(!inbox.projects.contains_key(&project::ProjectId(1)));
    assert!(inbox.projects.contains_key(&project::ProjectId(2)));
}

struct NoChapters {
    config: ai::ProviderConfig,
}
impl ai::chapters::ChapterGenerator for NoChapters {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::chapters::MODE.into(),
            operation: ai::Operation::GenerateChapters,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::chapters::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "chapters-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        _: &ai::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::chapters::ChapterProposal, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}
struct NoFillers {
    config: ai::ProviderConfig,
}
impl ai::fillers::FillerClassifier for NoFillers {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::fillers::MODE.into(),
            operation: ai::Operation::ClassifyFillers,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::fillers::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "fillers-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        _: &ai::fillers::FillerInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::fillers::FillerProposal, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}
#[test]
fn chapters_generate_needs_the_lent_consent_gate_before_any_upload() {
    let (_handler, mut inbox) =
        Inbox::new(Grants::default(), ByteBudget::new(64 << 20).unwrap()).unwrap();
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(1), "chapters".into()).unwrap(),
        10,
    )
    .unwrap();
    let config = ai::chapters::OpenAiChapterGenerator::provider_config(ai::defaults::model(
        "openai",
        ai::Operation::GenerateChapters,
    ))
    .unwrap();
    let request = |revision, key: &str| {
        json!({
            "command": "chapters.generate",
            "params": {"language": "vi", "max_chapters": 4},
            "base_rev": revision,
            "idempotency_key": key,
            "request_id": key,
        })
    };
    let revision = session.revision();
    let response = inbox
        .execute_project(
            &mut session,
            request(json!(revision), "chapters-denied"),
            |_, _| Ok(()),
        )
        .unwrap();
    assert_eq!(response.pointer("/value/status"), Some(&json!("error")));
    assert_eq!(response.pointer("/value/value"), Some(&json!("permission")));

    inbox.attach_text(Arc::new(TextAccess::new(
        Arc::new(NoChapters {
            config: config.clone(),
        }),
        Arc::new(NoFillers {
            config: config.clone(),
        }),
        Box::new(engine::providers::StoredConsent::grant([(
            config,
            ai::Money::new("USD", 0).unwrap(),
        )])),
    )));
    let revision = session.revision();
    let response = inbox
        .execute_project(
            &mut session,
            request(json!(revision), "chapters-granted"),
            |_, _| Ok(()),
        )
        .unwrap();
    assert_eq!(response.pointer("/value/status"), Some(&json!("error")));
    assert_eq!(response.pointer("/value/value"), Some(&json!("refused")));
}

#[tokio::test]
async fn delegated_job_failures_keep_codes_on_the_mcp_wire() {
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
    send(&mut write, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"failures","version":"1"}}})).await;
    assert!(response(&mut read).await.get("result").is_some());
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    for (index, (inner, code)) in [
        (
            EngineError::Lut(engine::luts::Error::Render(render::Error::Unavailable(
                Vec::new(),
            ))),
            "unavailable",
        ),
        (
            EngineError::Lut(engine::luts::Error::Render(render::Error::Cancelled)),
            "render",
        ),
        (
            EngineError::Library(engine::library::Error::Edit(engine::EditError::Project(
                project::Error::Busy,
            ))),
            "busy",
        ),
        (
            EngineError::Library(engine::library::Error::Runtime(
                engine::RuntimeError::Cancelled,
            )),
            "cancelled",
        ),
        (engine::RuntimeError::CacheCorrupt.into(), "job"),
        (
            engine::RuntimeError::Failed(Arc::new(engine::RuntimeError::CacheCorrupt)).into(),
            "job",
        ),
        (engine::RuntimeError::QueueFull.into(), "queue_full"),
        (
            engine::RuntimeError::WorkerPanicked.into(),
            "worker_failure",
        ),
        (
            EngineError::Control(engine::control::ControlError::Limit),
            "limit",
        ),
        (
            EngineError::Session(engine::SessionError::StaleResult),
            "stale_result",
        ),
        (engine::RuntimeError::BudgetFull.into(), "budget"),
        (engine::RuntimeError::Poisoned.into(), "job"),
        (
            engine::RuntimeError::Io(std::io::Error::other("eviction")).into(),
            "io",
        ),
        (EngineError::Io(std::io::Error::other("disk")), "io"),
        (
            engine::recovery::Error::PersistenceFailed(Arc::new(EngineError::Io(
                std::io::Error::other("disk"),
            )))
            .into(),
            "io",
        ),
        (
            EngineError::Render(render::Error::Unavailable(Vec::new())),
            "unavailable",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        send(&mut write, json!({"jsonrpc":"2.0","id":index+2,"method":"tools/call","params":{"name":"command_run","arguments":{"command":"inspect","params":{}}}})).await;
        while inbox.receive.is_empty() {
            tokio::task::yield_now().await;
        }
        inbox
            .poll_projects(|_, _, _| {
                Err(EngineError::Runtime(engine::RuntimeError::Failed(
                    Arc::new(EngineError::Runtime(engine::RuntimeError::Failed(
                        Arc::new(inner),
                    ))),
                )))
            })
            .unwrap();
        let value = response(&mut read).await;
        assert_eq!(
            value.pointer("/result/structuredContent/error/code"),
            Some(&json!(code))
        );
        assert_eq!(value.pointer("/result/isError"), Some(&json!(true)));
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}
