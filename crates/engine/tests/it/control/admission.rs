#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use crate::support::control::{
    Allow, Deny, add, caller, error, ok, request, run, runner, seed, session,
};
use engine::control::{self, Caller, ControlResult, ErrorCode, Payload, ResponseValue};
use engine::jobs::JobRunner;
use engine::{Command, Error as EngineError, Session, registry};
use project::{Color, Item, ItemId, ItemParams, OverlayParams, Project, Track, TrackId, TrackKind};
use serde_json::{Value, json};
use time::{SourceRange, Tick};

#[test]
fn invalid_shape_version_unknown_and_nested_extra_refuse_before_effects() {
    let mut s = seed();
    let mut r = runner();
    let initial = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let mut cases = vec![
        ("absent", json!({}), ErrorCode::UnknownCommand),
        (
            "timeline.add_track",
            json!({"name":"x","kind":"wrong"}),
            ErrorCode::InvalidParams,
        ),
        (
            "timeline.add_track",
            json!({"name":1,"kind":"overlay"}),
            ErrorCode::InvalidParams,
        ),
        (
            "timeline.add_track",
            json!({"name":"x","kind":"video","extra":"secret"}),
            ErrorCode::InvalidParams,
        ),
        ("undo", json!({"extra":true}), ErrorCode::InvalidParams),
    ];
    let mut insert = registry()
        .iter()
        .find(|s| s.id == "timeline.insert")
        .unwrap()
        .example
        .params
        .clone();
    insert["item"]["source_range"]["extra"] = true.into();
    cases.push(("timeline.insert", insert, ErrorCode::InvalidParams));
    for (id, params, code) in cases {
        error(run(&mut s, &mut r, id, params, "invalid"), code);
        assert_eq!(s.project(), &initial);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
    }
    for version in [control::API_VERSION - 1, control::API_VERSION + 1] {
        let mut bad = request(
            &s,
            "timeline.add_track",
            json!({"name":"x","kind":"video"}),
            "bad-version",
        );
        bad.api_version = version;
        error(
            s.control_request(bad, caller(&Allow), &mut r).unwrap(),
            ErrorCode::SchemaMismatch,
        );
    }
    let mut bad = request(
        &s,
        "timeline.add_track",
        json!({"name":"x","kind":"video"}),
        "bad-hash",
    );
    bad.registry_hash = Some("old".into());
    error(
        s.control_request(bad, caller(&Allow), &mut r).unwrap(),
        ErrorCode::SchemaMismatch,
    );
    assert_eq!(s.project(), &initial);
    assert_eq!(s.revision(), rev);
    assert_eq!(r.counts().0, 0);
}
#[test]
fn loss_replay_conflict_principal_and_current_authorization() {
    let mut s = session();
    let mut r = runner();
    let req = request(
        &s,
        "timeline.add_track",
        json!({"name":"once","kind":"video"}),
        "once",
    );
    let first = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    add(&mut s, &mut r, "second");
    let replay = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    assert_eq!(s.history_len(), (2, 0));
    error(
        s.control_request(req.clone(), caller(&Deny), &mut r)
            .unwrap(),
        ErrorCode::Permission,
    );
    let mut conflict = req.clone();
    conflict.params["name"] = "different".into();
    error(
        s.control_request(conflict, caller(&Allow), &mut r).unwrap(),
        ErrorCode::IdempotencyConflict,
    );
    error(
        s.control_request(
            req,
            Caller {
                principal: "other",
                external: true,
                policy: &Allow,
                fonts: None,
                sounds: None,
                media: None,
                chapters: None,
                captions: None,
            },
            &mut r,
        )
        .unwrap(),
        ErrorCode::StaleRevision,
    );
    assert_eq!(s.history_len(), (2, 0));
}
#[test]
fn ledger_exhaustion_has_no_eviction_and_no_effect() {
    let mut s = session();
    let mut r = runner();
    let req = request(&s, "project.rename", json!({"name":"Test"}), "first");
    let first = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    for index in 1..control::MAX_RECEIPTS {
        ok(run(
            &mut s,
            &mut r,
            "project.rename",
            json!({"name":"Test"}),
            &index.to_string(),
        ));
    }
    let before = s.revision();
    error(
        run(
            &mut s,
            &mut r,
            "timeline.add_track",
            json!({"name":"never","kind":"video"}),
            "full",
        ),
        ErrorCode::ReceiptCapacity,
    );
    assert!(s.project().data().scene().timeline.overlays.is_empty());
    assert_eq!(s.revision(), before);
    assert_eq!(s.history_len(), (0, 0));
    let replay = s.control_request(req, caller(&Allow), &mut r).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
}
#[test]
fn stale_batch_and_external_gestures_cannot_bypass_admission() {
    let mut s = session();
    let mut r = runner();
    let old = request(
        &s,
        "batch",
        json!({"commands":[{"command":"timeline.add_track","params":{"name":"stale","kind":"video"}}]}),
        "batch",
    );
    add(&mut s, &mut r, "ui");
    let before = s.project().clone();
    error(
        s.control_request(old, caller(&Allow), &mut r).unwrap(),
        ErrorCode::StaleRevision,
    );
    assert_eq!(s.project(), &before);
    for id in [
        "begin_gesture",
        "commit_gesture",
        "cancel_gesture",
        "timeline.preview",
        "timeline.commit_preview",
        "timeline.discard_preview",
    ] {
        error(
            run(
                &mut s,
                &mut r,
                id,
                if id == "timeline.preview" {
                    json!({"commands":[]})
                } else {
                    json!({})
                },
                id,
            ),
            ErrorCode::Permission,
        );
    }
    s.begin_gesture().unwrap();
    error(
        run(&mut s, &mut r, "inspect", json!({}), "inspect-preview"),
        ErrorCode::TransactionActive,
    );
    error(
        run(
            &mut s,
            &mut r,
            "timeline.add_track",
            json!({"name":"blocked","kind":"video"}),
            "gesture-edit",
        ),
        ErrorCode::TransactionActive,
    );
    s.cancel_gesture().unwrap();
    let malformed = json!({"commands":[{"command":"timeline.add_track","params":{"name":"early","kind":"video"}},{"command":"export","params":{"settings":render::OutputOptions::default(),"path":"/never","overwrite":false}}]});
    assert!(matches!(
        run(&mut s, &mut r, "batch", malformed, "mixed").value,
        ResponseValue::Error(_)
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(r.counts().0, 0);
}
#[test]
fn strict_frame_parser_refuses_duplicate_truncated_hostile_and_unsafe_numbers() {
    for bytes in [
        b"{\"x\":1,\"x\":2}".as_slice(),
        b"{\"x\":{\"a\":1,\"a\":2}}",
        b"{",
        b"[",
        b"\xff",
        b"9007199254740992",
        b"-9007199254740992",
        b"18446744073709551615",
        b"-9223372036854775808",
        b"1e999",
        b"0 trailing",
    ] {
        assert!(control::parse_frame(bytes).is_err(), "{bytes:?}");
    }
    for n in [-control::MAX_SAFE_INTEGER, control::MAX_SAFE_INTEGER] {
        assert_eq!(
            control::parse_frame(n.to_string().as_bytes())
                .unwrap()
                .as_i64(),
            Some(n)
        );
    }
    assert!(control::parse_frame(&vec![b' '; control::MAX_FRAME_BYTES + 1]).is_err());
    assert!(
        control::parse_frame(format!("{}0{}", "[".repeat(34), "]".repeat(34)).as_bytes()).is_err()
    );
    assert!(control::parse_frame(format!("\"{}\"", "a".repeat((4 << 20) + 1)).as_bytes()).is_err());
    let mut s = session();
    let rev = s.revision();
    assert!(
        crate::wire::run(
            &mut s,
            "bookmark.add",
            json!({"start":control::MAX_SAFE_INTEGER+1,"duration":0,"note":"x","color":[0,0,0,0]})
        )
        .is_err()
    );
    assert_eq!(s.revision(), rev);
}
#[test]
fn policy_refusal_and_nested_policy_are_safe_before_io() {
    let mut s = session();
    let mut r = runner();
    let mut requests = vec![request(
        &s,
        "export",
        json!({"settings":render::OutputOptions::default(),"path":"/secret/private","overwrite":true}),
        "denied",
    )];
    requests.push(request(&s,"batch",json!({"commands":[{"command":"timeline.add_track","params":{"name":"private text","kind":"video"}}]}),"batch-denied"));
    let before = s.project().clone();
    for req in requests {
        let result = s.control_request(req, caller(&Deny), &mut r).unwrap();
        error(result.clone(), ErrorCode::Permission);
        let diagnostic = serde_json::to_string(&result).unwrap();
        assert!(!diagnostic.contains("secret") && !diagnostic.contains("private text"));
    }
    assert_eq!(s.project(), &before);
    assert_eq!(r.counts().0, 0);
}
#[test]
fn stale_loudness_export_refuses_before_jobs_and_replays_without_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("export.mp4");
    for measured in [
        None,
        Some(project::LoudnessGains {
            key: [0; 32],
            clip: vec![],
            master_db: 0.0,
        }),
    ] {
        let mut data = seed().project().as_ref().clone().into_data();
        data.settings.loudness.master = Some(project::LoudnessTarget::DEFAULT);
        data.settings.loudness.measured = measured;
        let mut s = Session::new(Project::new(data).unwrap(), 1).unwrap();
        let mut r = runner();
        let before = s.project().clone();
        let revision = s.revision();
        let history = s.history_len();
        let mut cursor = s.snapshot_and_subscribe(&r).unwrap().cursor;
        let req = request(
            &s,
            "export",
            json!({
                "settings": render::OutputOptions::default(), "path": output, "overwrite": false
            }),
            "loudness-stale",
        );
        for _ in 0..2 {
            error(
                s.control_request(req.clone(), caller(&Allow), &mut r)
                    .unwrap(),
                ErrorCode::Refused,
            );
        }
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), history);
        assert!(r.snapshots().is_empty());
        assert!(r.drain_events().is_empty());
        assert!(matches!(
            s.events(&mut cursor).unwrap().as_slice(),
            [control::Event {
                payload: Payload::CommandError {
                    code: ErrorCode::Refused,
                    ..
                },
                ..
            }]
        ));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn concurrent_duplicate_requests_execute_once_under_the_session_boundary() {
    let s = session();
    let req = request(
        &s,
        "timeline.add_track",
        json!({"name":"Concurrent","kind":"video"}),
        "concurrent",
    );
    let state = std::sync::Mutex::new((s, runner()));
    let values = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let mut guard = state.lock().unwrap();
                    let (session, runner) = &mut *guard;
                    serde_json::to_value(
                        session
                            .control_request(req.clone(), caller(&Allow), runner)
                            .unwrap(),
                    )
                    .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(values.windows(2).all(|pair| pair[0] == pair[1]));
    let guard = state.lock().unwrap();
    assert_eq!(guard.0.history_len(), (1, 0));
    assert_eq!(guard.0.project().data().scene().timeline.overlays.len(), 1);
}
#[test]
fn every_child_failure_restores_content_history_revision_and_ids() {
    for failed in 0..4 {
        let mut s = session();
        let mut r = runner();
        let before = s.project().clone();
        let revision = s.revision();
        let mut listener = s.snapshot_and_subscribe(&r).unwrap();
        let children: Vec<_> = (0..4)
            .map(|index| {
                if index == failed {
                    json!({"command":"track.remove","params":{"track":1000}})
                } else {
                    json!({"command":"timeline.add_track","params":{"name":"Candidate","kind":"video"}})
                }
            })
            .collect();
        assert!(matches!(
            run(
                &mut s,
                &mut r,
                "batch",
                json!({"commands":children}),
                "failed"
            )
            .value,
            ResponseValue::Error(_)
        ));
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), (0, 0));
        assert!(
            s.events(&mut listener.cursor)
                .unwrap()
                .iter()
                .all(|e| !matches!(e.payload, Payload::ProjectChanged { .. }))
        );
        let result = ok(run(
            &mut s,
            &mut r,
            "timeline.add_track",
            json!({"name":"After","kind":"video"}),
            "after",
        ));
        assert!(
            matches!(result,ControlResult::Edit{outcome} if outcome.created==vec![engine::CreatedId::Track(TrackId(1))])
        );
    }
}
#[test]
fn external_gesture_ownership_is_also_enforced_inside_generic_batch() {
    let mut s = session();
    let mut r = runner();
    let before = s.revision();
    for id in [
        "begin_gesture",
        "commit_gesture",
        "cancel_gesture",
        "timeline.preview",
        "timeline.commit_preview",
        "timeline.discard_preview",
    ] {
        assert!(matches!(
            run(
                &mut s,
                &mut r,
                "batch",
                json!({"commands":[{"command":id,"params":if id == "timeline.preview" {json!({"commands":[]})} else {json!({})}}]}),
                id
            )
            .value,
            ResponseValue::Error(_)
        ));
    }
    assert_eq!(s.revision(), before);
    assert_eq!(s.history_len(), (0, 0));
    let mut snapshot = s.snapshot_and_subscribe(&r).unwrap();
    s.begin_gesture().unwrap();
    s.apply(Command::RenameProject {
        name: "preview".into(),
    })
    .unwrap();
    s.cancel_gesture().unwrap();
    assert!(
        s.events(&mut snapshot.cursor)
            .unwrap()
            .iter()
            .all(|e| !matches!(e.payload, Payload::ProjectChanged { .. }))
    );
}
#[test]
fn parser_exact_collection_node_depth_and_text_limits() {
    let array = json!(vec![0; 100_000]);
    assert!(control::parse_frame(&serde_json::to_vec(&array).unwrap()).is_ok());
    let array = json!(vec![0; 100_001]);
    assert!(control::parse_frame(&serde_json::to_vec(&array).unwrap()).is_err());
    let value = json!([vec![0; 99_998], vec![0; 99_999]]);
    assert!(control::parse_frame(&serde_json::to_vec(&value).unwrap()).is_ok());
    let value = json!([vec![0; 99_999], vec![0; 99_999]]);
    assert!(control::parse_frame(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(
        control::parse_frame(format!("{}0{}", "[".repeat(32), "]".repeat(32)).as_bytes()).is_ok()
    );
    assert!(
        control::parse_frame(format!("{}0{}", "[".repeat(33), "]".repeat(33)).as_bytes()).is_err()
    );
    assert!(control::parse_frame(format!("\"{}\"", "a".repeat(4 << 20)).as_bytes()).is_ok());
}
#[test]
fn malformed_envelope_and_batch_depth_have_no_effect() {
    let mut s = session();
    let mut r = runner();
    let initial = s.project().clone();
    let revision = s.revision();
    let req = request(
        &s,
        "timeline.add_track",
        json!({"name":"x","kind":"video"}),
        "shape",
    );
    let mut value = serde_json::to_value(req).unwrap();
    value["caller"] = "self-authorized".into();
    assert!(
        s.control_json(&serde_json::to_vec(&value).unwrap(), caller(&Allow), &mut r)
            .is_err()
    );
    let command = json!({"command":"timeline.add_track","params":{"name":"x","kind":"video"}});
    for count in [1024, 1025] {
        let batch = json!({"commands":vec![command.clone();count]});
        let result = run(&mut s, &mut r, "batch", batch, &format!("count-{count}"));
        // Root batch counts as one command: 1024 children exceed the total budget.
        error(result, ErrorCode::Limit);
    }
    let mut nested = command;
    for _ in 0..17 {
        nested = json!({"command":"batch","params":{"commands":[nested]}});
    }
    let req = request(&s, "batch", json!({"commands":[nested]}), "deep");
    assert!(s.control_request(req, caller(&Allow), &mut r).is_err());
    assert_eq!(s.project(), &initial);
    assert_eq!(s.revision(), revision);
    assert_eq!(r.counts().0, 0);
}
#[test]
fn diagnostic_debug_redacts_request_content_output_paths_and_frame_bytes() {
    let s = session();
    let request = request(
        &s,
        "export",
        json!({"path":"/private-secret-path","text":"private-secret-text"}),
        "private-secret-key",
    );
    assert!(!format!("{request:?}").contains("private-secret"));
    let result = ControlResult::JobAccepted {
        job: engine::jobs::JobId(0),
        revision: s.revision(),
        paths: vec!["/private-secret-path".into()],
    };
    assert!(!format!("{result:?}").contains("private-secret"));
    let frame = control::PreviewFrame {
        frame: 0,
        output_tick: Tick::ZERO,
        clips: vec![],
        width: 1,
        height: 1,
        jpeg: b"private-secret-frame".to_vec(),
    };
    assert!(!format!("{frame:?}").contains("private-secret"));
}
fn assert_integer_candidate_refusal(s: &mut Session, r: &mut JobRunner, id: &str, params: Value) {
    let before = s.project().clone();
    let history = s.history_len();
    let revision = s.revision();
    let metadata = s.metadata().clone();
    let mut listener = s.snapshot_and_subscribe(r).unwrap();
    let req = request(s, id, params, "integer-candidate");
    let response = s.control_request(req.clone(), caller(&Allow), r).unwrap();
    error(response.clone(), ErrorCode::InvalidParams);
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.revision(), revision);
    assert_eq!(s.metadata(), &metadata);
    assert!(
        s.events(&mut listener.cursor)
            .unwrap()
            .iter()
            .all(|e| !matches!(
                e.payload,
                Payload::ProjectChanged { .. } | Payload::RevisionChanged
            ))
    );
    assert!(s.take_change().is_none());
    ok(run(s, r, "inspect", json!({}), "inspect-after-refusal"));
    assert!(
        jsonschema::options()
            .offline()
            .build(&control::contract_schema()["response"])
            .unwrap()
            .is_valid(&serde_json::to_value(&response).unwrap())
    );
    s.apply(Command::RenameProject {
        name: "Later revision".into(),
    })
    .unwrap();
    let after = s.project().clone();
    let replay = s.control_request(req, caller(&Allow), r).unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(response).unwrap()
    );
    assert_eq!(s.project(), &after);
}
#[test]
fn integer_candidates_at_the_safe_limit_refuse_atomically_through_every_entry() {
    // A generated id at the safe limit refuses the next allocation.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(control::MAX_SAFE_INTEGER as u64),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![],
    });
    let mut generated = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    ok(run(&mut generated, &mut r, "inspect", json!({}), "before"));
    assert_integer_candidate_refusal(
        &mut generated,
        &mut r,
        "timeline.add_track",
        json!({"name":"Next","kind":"video"}),
    );

    // Derived source ticks refuse through one command and through a batch.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(1),
            animations: Default::default(),
            source_asset: None,
            source_range: SourceRange::new(
                Tick(control::MAX_SAFE_INTEGER - 10),
                Tick(control::MAX_SAFE_INTEGER),
            )
            .unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([0, 0, 0, 255]),
            }),
        }],
    });
    for batch in [false, true] {
        let mut s = Session::new(Project::new(data.clone()).unwrap(), 10).unwrap();
        let mut r = runner();
        ok(run(&mut s, &mut r, "inspect", json!({}), "before"));
        if batch {
            assert_integer_candidate_refusal(
                &mut s,
                &mut r,
                "batch",
                json!({"commands":[{"command":"project.rename","params":{"name":"Must roll back"}},{"command":"timeline.add_track","params":{"name":"Must not consume ID","kind":"video"}},{"command":"trim_end","params":{"item":1,"at":20}}]}),
            );
            let result = ok(run(
                &mut s,
                &mut r,
                "timeline.add_track",
                json!({"name":"After refusal","kind":"video"}),
                "after-batch",
            ));
            assert!(
                matches!(result,ControlResult::Edit{outcome} if outcome.created==vec![engine::CreatedId::Track(TrackId(2))])
            );
        } else {
            assert_integer_candidate_refusal(&mut s, &mut r, "trim_end", json!({"item":1,"at":20}));
        }
    }

    // The last safe created id still matches the output schema; the next id refuses.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(control::MAX_SAFE_INTEGER as u64 - 1),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![],
    });
    let mut last_safe = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let result = ok(run(
        &mut last_safe,
        &mut r,
        "timeline.add_track",
        json!({"name":"Last safe","kind":"video"}),
        "last-safe",
    ));
    assert!(
        matches!(&result,ControlResult::Edit{outcome} if outcome.created==vec![engine::CreatedId::Track(TrackId(control::MAX_SAFE_INTEGER as u64))])
    );
    let schema = &registry()
        .iter()
        .find(|spec| spec.id == "timeline.add_track")
        .unwrap()
        .output_schema;
    assert!(
        jsonschema::options()
            .offline()
            .build(schema)
            .unwrap()
            .is_valid(&serde_json::to_value(result).unwrap())
    );
    last_safe.take_change();
    assert_integer_candidate_refusal(
        &mut last_safe,
        &mut r,
        "timeline.add_track",
        json!({"name":"Unsafe","kind":"video"}),
    );

    // Derived keyframe ticks refuse before commit.
    let mut data = seed().project().data().clone();
    let path = project::AnimationPath::new("opacity").unwrap();
    data.scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find(|item| item.id == ItemId(1))
        .unwrap()
        .animations
        .get_mut(&path)
        .unwrap()
        .channel_mut(None)
        .unwrap()
        .put(project::Keyframe::new(
            Tick(control::MAX_SAFE_INTEGER / 2 + 1),
            project::ParamValue::Scalar(0.5),
        ))
        .unwrap();
    let mut keyframes = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    ok(run(&mut keyframes, &mut r, "inspect", json!({}), "before"));
    assert_integer_candidate_refusal(
        &mut keyframes,
        &mut r,
        "keyframes.scale_time",
        json!({"item":1,"numerator":2,"denominator":1}),
    );

    // History destinations and legacy JSON share the same candidate validation.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(1),
            animations: Default::default(),
            source_asset: None,
            source_range: SourceRange::new(
                Tick(control::MAX_SAFE_INTEGER - 10),
                Tick(control::MAX_SAFE_INTEGER),
            )
            .unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([0, 0, 0, 255]),
            }),
        }],
    });
    let mut history = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let before = history.project().clone();
    let revision = history.revision();
    assert!(matches!(
        crate::wire::run(&mut history, "trim_end", json!({"item":1,"at":20})),
        Err(EngineError::Control(
            engine::control::ControlError::InvalidParams
        ))
    ));
    assert_eq!(history.project(), &before);
    assert_eq!(history.revision(), revision);
    history
        .apply(Command::TrimEnd {
            item: ItemId(1),
            at: Tick(20),
        })
        .unwrap();
    history.undo().unwrap();
    history.take_change();
    assert_eq!(history.project(), &before);
    assert_eq!(history.history_len(), (0, 1));
    assert_integer_candidate_refusal(&mut history, &mut r, "redo", json!({}));
}

#[test]
fn owner_failures_classify_publication_receipts_and_events() {
    use ErrorCode as C;
    use engine::recovery::Error as R;
    use engine::{Error as E, RuntimeError as B};
    use engine::{
        Failure, FontError as F, SoundError as S, providers::Error as P, transcript::Error as T,
    };
    use std::{io, sync::Arc};
    macro_rules! row {
        ($e:expr, $c:ident, $t:expr) => {
            ($e, C::$c, $t)
        };
    }
    let mut cases = vec![
        row!(E::Transcript(T::Runtime(B::NetworkTransient)), Job, true),
        row!(
            E::Transcript(T::Runtime(B::Failed(Arc::new(B::Cancelled)))),
            Cancelled,
            false
        ),
        row!(
            E::Transcript(T::Providers(P::ConsentRequired)),
            Permission,
            false
        ),
        row!(E::Transcript(T::Ai(ai::Error::ConsentRequired)), Ai, false),
        row!(
            E::Transcript(T::Ai(ai::Error::RateLimit {
                retry_after_seconds: Some(3)
            })),
            Ai,
            true
        ),
        row!(
            E::Transcript(T::Media(media::MediaError::Cancelled)),
            Media,
            false
        ),
        row!(E::Providers(P::Runtime(B::NetworkTransient)), Job, true),
        row!(
            E::Providers(P::Runtime(B::Failed(Arc::new(B::Cancelled)))),
            Cancelled,
            false
        ),
        row!(
            E::Providers(P::Ai(ai::Error::RateLimit {
                retry_after_seconds: Some(3)
            })),
            Ai,
            true
        ),
        row!(E::Providers(P::Ai(ai::Error::ConsentRequired)), Ai, false),
        row!(
            E::Models(engine::models::Error::Runtime(B::NetworkTransient)),
            Job,
            true
        ),
        row!(
            E::Models(engine::models::Error::Runtime(B::Failed(Arc::new(
                B::Cancelled
            )))),
            Cancelled,
            false
        ),
        row!(
            E::Models(engine::models::Error::Ai(ai::Error::ConsentRequired)),
            Ai,
            false
        ),
        row!(
            E::Models(engine::models::Error::Ai(ai::Error::Chapters(
                ai::chapters::ChapterRefusal::EmptyTranscript
            ))),
            Refused,
            false
        ),
        row!(
            E::Models(engine::models::Error::Ai(ai::Error::RateLimit {
                retry_after_seconds: Some(3)
            })),
            Ai,
            true
        ),
        row!(
            E::Models(engine::models::Error::Io(io::Error::other("model IO"))),
            Io,
            false
        ),
        row!(
            E::Runtime(B::Render(render::Error::Unavailable(Vec::new()))),
            Unavailable,
            false
        ),
        row!(
            E::Runtime(B::Render(render::Error::Cancelled)),
            Render,
            false
        ),
        row!(
            E::Runtime(B::Media(media::MediaError::Cancelled)),
            Media,
            false
        ),
        row!(
            E::Lut(engine::luts::Error::Render(render::Error::Unavailable(
                Vec::new()
            ))),
            Unavailable,
            false
        ),
        row!(
            E::Lut(engine::luts::Error::Render(render::Error::Cancelled)),
            Render,
            false
        ),
        row!(
            E::Lut(engine::luts::Error::Runtime(B::NetworkTransient)),
            Job,
            true
        ),
        row!(
            E::Lut(engine::luts::Error::Runtime(B::Failed(Arc::new(
                B::Cancelled
            )))),
            Cancelled,
            false
        ),
        row!(
            E::Library(engine::library::Error::Edit(engine::EditError::Project(
                project::Error::Busy
            ))),
            Busy,
            false
        ),
        row!(
            E::Library(engine::library::Error::Edit(engine::EditError::Session(
                engine::SessionError::NoHistory
            ))),
            Refused,
            false
        ),
        row!(
            E::Library(engine::library::Error::Runtime(B::NetworkTransient)),
            Job,
            true
        ),
        row!(
            E::Library(engine::library::Error::Runtime(B::Failed(Arc::new(
                B::Cancelled
            )))),
            Cancelled,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::SchemaMismatch),
            SchemaMismatch,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::SessionGone),
            SessionGone,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::StaleRevision),
            StaleRevision,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::Permission),
            Permission,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::IdempotencyConflict),
            IdempotencyConflict,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::ReceiptCapacity),
            ReceiptCapacity,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::ResyncRequired),
            ResyncRequired,
            false
        ),
        row!(
            E::Runtime(engine::RuntimeError::JobNotRetained),
            JobNotRetained,
            false
        ),
        row!(
            E::Session(engine::SessionError::ControlIdentity),
            Refused,
            false
        ),
        row!(E::Runtime(B::CacheBusy), Refused, false),
        row!(
            E::Runtime(engine::RuntimeError::ModelResponse),
            Refused,
            false
        ),
        row!(
            E::Runtime(engine::RuntimeError::StorageFull),
            Refused,
            false
        ),
        row!(E::Recovery(R::Runtime(B::NetworkTransient)), Job, true),
        row!(E::Recovery(R::Runtime(B::Cancelled)), Cancelled, false),
        row!(
            E::Recovery(R::Edit(engine::EditError::Session(
                engine::SessionError::StaleResult
            ))),
            StaleResult,
            false
        ),
        row!(
            E::Recovery(R::Edit(engine::EditError::Control(
                engine::control::ControlError::InvalidParams
            ))),
            InvalidParams,
            false
        ),
        row!(E::Recovery(R::Corrupt), Refused, false),
        row!(E::Recovery(R::Busy), Refused, false),
        row!(E::Recovery(R::SaveAsRequired), Refused, false),
        row!(
            E::Recovery(R::PersistenceFailed(Arc::new(E::Io(io::Error::other(
                "disk"
            ))))),
            Io,
            false
        ),
        row!(E::Recovery(R::Io(io::Error::other("disk"))), Io, false),
        row!(E::Recovery(R::Limit), Limit, false),
        row!(E::Recovery(R::Invalid("clock")), Refused, false),
        row!(E::Recovery(R::Unsupported("locking")), Refused, false),
        row!(E::Recovery(R::Project(project::Error::Busy)), Busy, false),
        row!(
            E::Recovery(R::Project(project::Error::Conflict)),
            Conflict,
            false
        ),
        row!(
            E::Recovery(R::Project(project::Error::Io(io::Error::other("disk")))),
            Io,
            false
        ),
        row!(
            E::Recovery(R::Project(project::Error::Media(
                media::MediaError::Cancelled
            ))),
            Media,
            false
        ),
        row!(
            E::Recovery(R::Project(project::Error::Invalid("journal"))),
            Refused,
            false
        ),
        row!(
            E::Runtime(engine::RuntimeError::QueueFull),
            QueueFull,
            false
        ),
        row!(E::Runtime(B::BudgetFull), Budget, false),
        row!(E::Runtime(B::ArtifactTooLarge), Budget, false),
        row!(
            E::Runtime(engine::RuntimeError::Cancelled),
            Cancelled,
            false
        ),
        row!(
            E::Runtime(engine::RuntimeError::WorkerPanicked),
            WorkerFailure,
            false
        ),
        row!(E::Runtime(B::Poisoned), Job, false),
        row!(E::Runtime(B::Io(io::Error::other("eviction"))), Io, false),
        row!(E::Runtime(B::Limit), Limit, false),
        row!(
            E::Runtime(B::Invalid("model accounting owner")),
            Refused,
            false
        ),
        row!(
            E::Runtime(engine::RuntimeError::NetworkTransient),
            Job,
            true
        ),
        row!(
            E::Session(engine::SessionError::StaleResult),
            StaleResult,
            false
        ),
        row!(E::Runtime(B::LoudnessStale), Refused, false),
        row!(
            E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Control(
                engine::control::ControlError::Limit
            )))),
            Limit,
            false
        ),
        row!(E::Runtime(engine::RuntimeError::CacheCorrupt), Job, false),
        row!(E::Render(render::Error::Cancelled), Render, false),
        row!(E::Media(media::MediaError::Cancelled), Media, false),
        row!(E::Ai(ai::Error::Auth), Ai, false),
        row!(E::Sounds(S::NoCredentials), Setup, false),
        row!(E::Font(F::Unavailable), Refused, false),
        row!(E::Providers(P::ConsentRequired), Permission, false),
        row!(E::Transcript(T::NoSpeech), Refused, false),
        row!(E::Io(io::Error::other("io")), Io, false),
        row!(
            E::Control(engine::control::ControlError::UnknownCommand),
            UnknownCommand,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::InvalidParams),
            InvalidParams,
            false
        ),
        row!(
            E::Control(engine::control::ControlError::Limit),
            Limit,
            false
        ),
        row!(
            E::Edit(engine::EditError::NoTrack(TrackId(1))),
            Refused,
            false
        ),
        row!(
            E::Edit(engine::EditError::NoAsset(media::AssetId(1))),
            Refused,
            false
        ),
        row!(
            E::Edit(engine::EditError::AssetsInUse(Vec::new())),
            Refused,
            false
        ),
        row!(
            E::Edit(engine::EditError::NoItem(ItemId(1))),
            Refused,
            false
        ),
        row!(
            E::Edit(engine::EditError::Locked(TrackId(1))),
            Refused,
            false
        ),
        row!(E::Edit(engine::EditError::SceneBoundary), Refused, false),
        row!(E::Edit(engine::EditError::Protected), Refused, false),
        row!(E::Edit(engine::EditError::CuesNotAdjacent), Refused, false),
        row!(E::Edit(engine::EditError::CuesAcrossTracks), Refused, false),
        row!(E::Edit(engine::EditError::CuesAcrossItems), Refused, false),
        row!(
            E::Edit(engine::EditError::Overlap(TrackId(1))),
            Refused,
            false
        ),
        row!(
            E::Artifacts(engine::ArtifactError::EmptyProject),
            EmptyProject,
            false
        ),
        row!(E::Edit(engine::EditError::Invalid("input")), Refused, false),
        row!(
            E::Edit(engine::EditError::Unsupported("capability")),
            Refused,
            false
        ),
        row!(E::Session(engine::SessionError::Panicked), Failed, false),
        row!(
            E::Session(engine::SessionError::TransactionActive),
            TransactionActive,
            false
        ),
        row!(
            E::Session(engine::SessionError::NoGesture),
            TransactionActive,
            false
        ),
        row!(E::Session(engine::SessionError::NoHistory), Refused, false),
        row!(
            E::Session(engine::SessionError::IdExhausted),
            Refused,
            false
        ),
        row!(E::Runtime(B::IdExhausted), Refused, false),
        row!(
            E::Edit(engine::EditError::Project(project::Error::Busy)),
            Busy,
            false
        ),
        row!(
            E::Edit(engine::EditError::Time(time::TimeError::Overflow)),
            Refused,
            false
        ),
        row!(
            E::Edit(engine::EditError::Captions(captions::Error::TooLarge)),
            Refused,
            false
        ),
        row!(E::Providers(P::ModelMissing("test")), Setup, false),
        row!(E::Providers(P::RunnerMissing), Setup, false),
        row!(E::Providers(P::CredentialMissing("test")), Setup, false),
        row!(E::Providers(P::AutoLanguageUnsupported), Refused, false),
        row!(E::Providers(P::ConsentRequired), Permission, false),
        row!(E::Transcript(T::NoSpeech), Refused, false),
        row!(E::Transcript(T::LowConfidence), Refused, false),
        row!(E::Font(F::Unavailable), Refused, false),
        row!(E::Font(F::NotDownloadable), Refused, false),
        row!(E::Font(F::DownloadActive), Refused, false),
        row!(E::Font(F::Integrity), Refused, false),
        row!(E::Font(F::Response), Refused, false),
        row!(
            E::Font(F::Face(text::FontError::InvalidName)),
            Refused,
            false
        ),
        row!(E::Font(F::Catalog), Refused, false),
        row!(E::Sounds(S::NoCredentials), Setup, false),
        row!(E::Sounds(S::Auth), Auth, false),
        row!(E::Sounds(S::Quota), Quota, false),
        row!(
            E::Sounds(S::RateLimit {
                retry_after_seconds: Some(3)
            }),
            RateLimit,
            false
        ),
        row!(E::Sounds(S::Network), Network, false),
        row!(E::Sounds(S::Server), Network, false),
        row!(E::Sounds(S::InvalidResponse), Sounds, false),
        row!(E::Sounds(S::Limit), SoundLimit, false),
        row!(E::Sounds(S::NoAudioDevice), AudioDevice, false),
        row!(E::Sounds(S::NotCached), Sounds, false),
        row!(E::Sounds(S::LibraryCorrupt), Sounds, false),
        row!(E::Sounds(S::Cancelled), SoundCancelled, false),
        row!(E::Sounds(S::Io(io::Error::other("io"))), SoundIo, false),
        row!(
            E::Sounds(S::Media(media::MediaError::Cancelled)),
            Media,
            false
        ),
        row!(E::Sounds(S::Project(project::Error::Busy)), SoundIo, false),
        row!(E::Sounds(S::Invalid("input")), Sounds, false),
        row!(
            E::Render(render::Error::Unavailable(Vec::new())),
            Unavailable,
            false
        ),
        row!(
            E::Render(render::Error::Io(io::Error::other("render"))),
            Render,
            false
        ),
        row!(
            E::Edit(engine::EditError::Project(project::Error::Conflict)),
            Conflict,
            false
        ),
        row!(
            E::Edit(engine::EditError::Project(project::Error::Io(
                io::Error::other("project")
            ))),
            Io,
            false
        ),
        row!(
            E::Edit(engine::EditError::Project(project::Error::Media(
                media::MediaError::Cancelled
            ))),
            Media,
            false
        ),
        row!(
            E::Edit(engine::EditError::Project(project::Error::Invalid(
                "project"
            ))),
            Refused,
            false
        ),
        row!(
            E::Ai(ai::Error::Chapters(
                ai::chapters::ChapterRefusal::EmptyTranscript
            )),
            Refused,
            false
        ),
        row!(E::Ai(ai::Error::Network), Ai, true),
        row!(E::Ai(ai::Error::Timeout), Ai, true),
        row!(E::Ai(ai::Error::Server), Ai, true),
        row!(E::Ai(ai::Error::Schema), Ai, true),
        row!(
            E::Ai(ai::Error::RateLimit {
                retry_after_seconds: Some(3)
            }),
            Ai,
            true
        ),
        row!(E::Ai(ai::Error::Incomplete), Ai, false),
    ];
    for (inner, code) in [
        (E::Runtime(B::CacheBusy), C::Refused),
        (E::Runtime(B::ModelResponse), C::Refused),
        (E::Runtime(B::StorageFull), C::Refused),
        (E::Runtime(B::LoudnessStale), C::Refused),
        (E::Runtime(B::JobNotRetained), C::JobNotRetained),
        (E::Runtime(B::IdExhausted), C::Refused),
        (E::Runtime(B::Cancelled), C::Cancelled),
        (E::Runtime(B::NetworkTransient), C::Job),
        (E::Runtime(B::CacheCorrupt), C::Job),
        (E::Runtime(B::QueueFull), C::QueueFull),
        (E::Runtime(B::WorkerPanicked), C::WorkerFailure),
        (E::Control(engine::control::ControlError::Limit), C::Limit),
        (
            E::Session(engine::SessionError::StaleResult),
            C::StaleResult,
        ),
        (E::Io(io::Error::other("disk")), C::Io),
        (
            E::Render(render::Error::Unavailable(Vec::new())),
            C::Unavailable,
        ),
    ] {
        cases.push((
            E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Runtime(
                engine::RuntimeError::Failed(Arc::new(inner)),
            )))),
            code,
            false,
        ));
    }
    for (cause, code) in [
        (E::Runtime(B::CacheBusy), C::Refused),
        (E::Runtime(B::ModelResponse), C::Refused),
        (E::Runtime(B::StorageFull), C::Refused),
        (E::Runtime(B::LoudnessStale), C::Refused),
        (E::Runtime(B::JobNotRetained), C::JobNotRetained),
        (E::Runtime(B::IdExhausted), C::Refused),
        (E::Runtime(B::Cancelled), C::Cancelled),
        (E::Runtime(B::CacheCorrupt), C::Job),
        (E::Control(engine::control::ControlError::Limit), C::Limit),
        (E::Runtime(B::BudgetFull), C::Budget),
        (
            E::Runtime(engine::RuntimeError::WorkerPanicked),
            C::WorkerFailure,
        ),
        (E::Runtime(B::Poisoned), C::Job),
        (E::Runtime(engine::RuntimeError::Cancelled), C::Cancelled),
        (
            E::Session(engine::SessionError::StaleResult),
            C::StaleResult,
        ),
        (E::Runtime(engine::RuntimeError::NetworkTransient), C::Job),
    ] {
        let failure = R::PersistenceFailed(Arc::new(cause));
        assert_eq!(failure.code(), code);
        assert!(!failure.transient());
        cases.push((E::Recovery(failure), code, false));
    }
    cases.push((
        E::Runtime(B::Failed(Arc::new(B::NetworkTransient))),
        C::Job,
        false,
    ));
    for cause in [
        B::CacheBusy,
        B::ModelResponse,
        B::StorageFull,
        B::LoudnessStale,
    ] {
        cases.push((E::Runtime(B::Failed(Arc::new(cause))), C::Refused, false));
    }
    cases.push((
        E::Runtime(B::Failed(Arc::new(B::Cancelled))),
        C::Cancelled,
        false,
    ));
    cases.push((
        E::Runtime(B::Failed(Arc::new(engine::SessionError::StaleResult))),
        C::StaleResult,
        false,
    ));
    let message = E::Providers(P::ModelMissing("test-model")).to_string();
    let cache = B::CacheCorrupt;
    assert_eq!(cache.code(), C::Job);
    assert!(!cache.transient());
    assert!(std::error::Error::source(&cache).is_none());
    cases.push((E::Runtime(B::Failed(Arc::new(cache))), C::Job, false));
    assert!(message.contains("test-model"));
    assert!(!message.contains("vibecut-cli"));
    for (failure, code, transient) in cases {
        assert_eq!(failure.transient(), transient, "{failure:?}");
        let mut s = seed();
        let r = runner();
        let before = s.project().clone();
        let rev = s.revision();
        let history = s.history_len();
        let mut cursor = s.snapshot_and_subscribe(&r).unwrap().cursor;
        let req = request(&s, "project.save", json!({}), "failure");
        error(
            s.control_lifecycle_save(req.clone(), caller(&Allow), |_, _| Err(failure))
                .unwrap(),
            code,
        );
        error(
            s.control_lifecycle_save(req, caller(&Allow), |_, _| panic!("replayed publication"))
                .unwrap(),
            code,
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
        assert!(
            matches!(s.events(&mut cursor).unwrap().as_slice(), [control::Event { payload: Payload::CommandError { code: observed, .. }, .. }] if *observed == code)
        );
    }
}

#[test]
fn clipboard_copy_receipts_preserve_newer_snapshot_and_stale_paste_is_atomic() {
    let mut s = seed();
    let mut r = runner();
    let before = s.project().clone();
    let history = s.history_len();
    let rev = s.revision();
    let first = request(
        &s,
        "clipboard.copy_keyframes",
        json!({"keys":[{"item":1,"path":"opacity","time":0}]}),
        "copy-first",
    );
    let stale = request(
        &s,
        "clipboard.paste_keyframes",
        json!({"item":1,"at":2,"source":{"kind":"stored"}}),
        "stale-paste",
    );
    let first_response = s
        .control_request(first.clone(), caller(&Allow), &mut r)
        .unwrap();
    ok(first_response.clone());
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    assert_ne!(s.revision(), rev);
    error(
        s.control_request(stale, caller(&Allow), &mut r).unwrap(),
        ErrorCode::StaleRevision,
    );
    s.execute("keyframes.add",json!({"item":1,"path":"opacity","component":null,"key":{"time":12,"value":{"kind":"scalar","value":0.25},"segment":"linear","tangent":"broken","incoming":null,"outgoing":{"dt":4,"dv":0.2}}})).unwrap();
    ok(run(
        &mut s,
        &mut r,
        "clipboard.copy_keyframes",
        json!({"keys":[{"item":1,"path":"opacity","time":12}]}),
        "copy-second",
    ));
    let rev = s.revision();
    let replay = s.control_request(first, caller(&Allow), &mut r).unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(first_response).unwrap()
    );
    assert_eq!(s.revision(), rev);
    let result = ok(run(&mut s, &mut r, "clipboard.read", json!({}), "read"));
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(encoded["kind"], "clipboard");
    assert_eq!(encoded["payload"]["origin"], 12);
    assert_eq!(
        encoded["payload"]["params"][0]["param"]["channels"]["keys"][0]["outgoing"],
        json!({"dt":4,"dv":0.2})
    );
    let paste = request(
        &s,
        "clipboard.paste_keyframes",
        json!({"item":1,"at":40,"source":{"kind":"stored"}}),
        "paste",
    );
    let response = s
        .control_request(paste.clone(), caller(&Allow), &mut r)
        .unwrap();
    ok(response.clone());
    let pasted = s.project().clone();
    let history = s.history_len();
    ok(run(
        &mut s,
        &mut r,
        "clipboard.copy_keyframes",
        json!({"keys":[{"item":1,"path":"opacity","time":0}]}),
        "copy-third",
    ));
    assert_eq!(
        serde_json::to_value(s.control_request(paste, caller(&Allow), &mut r).unwrap()).unwrap(),
        serde_json::to_value(response).unwrap()
    );
    assert_eq!(s.project(), &pasted);
    assert_eq!(s.history_len(), history);
    let keys = &s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(1))
        .unwrap()
        .animations
        .get(&project::BuiltinPath::Opacity.into())
        .unwrap()
        .channel(None)
        .unwrap()
        .keys();
    assert!(keys.iter().any(|k| k.time == Tick(40)
        && k.value == project::ParamValue::Scalar(0.25)
        && k.tangent == project::TangentMode::Broken));
    // Undo and redo keep the Session snapshot; successful identity change clears it.
    s.undo().unwrap();
    s.redo().unwrap();
    s.apply(Command::CreateScene {
        name: "Other".into(),
    })
    .unwrap();
    let scene = s.project().data().scenes.iter().last().unwrap().id;
    s.apply(Command::SwitchScene { scene }).unwrap();
    let result = serde_json::to_value(ok(run(
        &mut s,
        &mut r,
        "clipboard.read",
        json!({}),
        "cleared",
    )))
    .unwrap();
    assert_eq!(result["payload"], Value::Null);
}
