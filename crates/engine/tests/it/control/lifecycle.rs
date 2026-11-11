#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use crate::support::control::{Allow, add, caller, error, ok, request, run, runner, session};
use engine::control::{self, ControlResult, ErrorCode, Payload, Request};
use engine::{Command, EditError, Session};
use project::ProjectId;
use serde_json::{Value, json};

fn request_at(
    s: &Session,
    request_id: &str,
    id: &str,
    params: Value,
    key: &str,
    base: &control::Revision,
) -> Request {
    Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: s.session_id().into(),
        request_id: request_id.into(),
        idempotency_key: Some(key.into()),
        base_rev: Some(base.clone()),
        command: id.into(),
        params,
    }
}
#[test]
fn revision_and_job_target_share_ui_save_preview_undo_and_rollback_boundaries() {
    let mut s = session();
    let mut r = runner();
    let first = s.revision();
    let old = s.job_target();
    add(&mut s, &mut r, "edit");
    assert_ne!(s.revision(), first);
    assert!(matches!(
        s.apply_job_result(
            &old,
            Command::RenameProject {
                name: "stale".into()
            }
        ),
        Err(EditError::Session(engine::SessionError::StaleResult))
    ));
    let edit = s.revision();
    s.undo().unwrap();
    assert_ne!(s.revision(), first);
    assert_ne!(s.revision(), edit);
    s.redo().unwrap();
    let before = s.revision();
    s.saved().unwrap();
    assert_ne!(s.revision(), before);
    let before = s.revision();
    s.transaction(|_| Ok(())).unwrap();
    assert_ne!(s.revision(), before);
    let before = s.revision();
    let result: engine::Result<()> = s.transaction(|s| {
        s.apply(Command::RenameProject {
            name: "preview".into(),
        })?;
        Err(EditError::Control(
            engine::control::ControlError::InvalidParams,
        ))
    });
    assert!(result.is_err());
    assert_ne!(s.revision(), before);
    let before = s.revision();
    s.begin_gesture().unwrap();
    s.cancel_gesture().unwrap();
    assert_ne!(s.revision(), before);
    let req = request(
        &s,
        "project.rename",
        json!({"name":"restart"}),
        "old-session",
    );
    let mut next = session();
    error(
        next.control_request(req, caller(&Allow), &mut r).unwrap(),
        ErrorCode::SessionGone,
    );
}
#[test]
fn snapshot_barrier_slow_listener_and_preview_never_claim_committed_change() {
    let mut s = session();
    let mut r = runner();
    let mut snapshot = s.snapshot_and_subscribe(&r).unwrap();
    add(&mut s, &mut r, "first");
    let events = s.events(&mut snapshot.cursor).unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e.payload, Payload::ProjectChanged { .. }))
    );
    assert!(s.events(&mut snapshot.cursor).unwrap().is_empty());
    let mut slow = s.snapshot_and_subscribe(&r).unwrap();
    for index in 0..control::MAX_EVENTS {
        add(&mut s, &mut r, &format!("pressure-{index}"));
    }
    assert!(matches!(
        s.events(&mut slow.cursor),
        Err(engine::control::ControlError::ResyncRequired)
    ));
    let mut fresh = s.snapshot_and_subscribe(&r).unwrap();
    s.begin_gesture().unwrap();
    s.apply(Command::RenameProject {
        name: "preview".into(),
    })
    .unwrap();
    assert!(s.snapshot_and_subscribe(&r).is_err());
    assert!(
        s.events(&mut fresh.cursor)
            .unwrap()
            .iter()
            .all(|e| !matches!(e.payload, Payload::ProjectChanged { .. }))
    );
    s.cancel_gesture().unwrap();
}
#[test]
fn decimal_project_identity_is_one_encoding_and_old_files_refuse() {
    for id in [
        0,
        control::MAX_SAFE_INTEGER as u64,
        (control::MAX_SAFE_INTEGER as u64) + 1,
        u64::MAX,
    ] {
        let id = ProjectId(id);
        let wire = serde_json::to_string(&id).unwrap();
        assert_eq!(wire, format!("\"{}\"", id.0));
        assert_eq!(serde_json::from_str::<ProjectId>(&wire).unwrap(), id);
    }
    for bad in ["0", "\"01\"", "\"+1\"", "\"18446744073709551616\""] {
        assert!(serde_json::from_str::<ProjectId>(bad).is_err());
    }
    let bytes = project::encode(session().project(), false).unwrap();
    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["project"]["id"], u64::MAX.to_string());
    value["schema_version"] = json!(2);
    assert!(matches!(
        project::decode(&serde_json::to_vec(&value).unwrap()),
        Err(project::Error::TooOld { .. })
    ));
}
#[test]
fn lifecycle_save_uses_receipts_and_retries_after_publication_failure() {
    let mut s = session();
    let initial = s.revision();
    let mut published = 0;
    let response = s
        .control_lifecycle_save(
            request_at(&s, "save-1", "project.save", json!({}), "save", &initial),
            caller(&Allow),
            |project, _metadata| {
                assert_eq!(project.data().name.as_str(), "Test");
                published += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(published, 1);
    let ControlResult::Edit { outcome } = ok(response) else {
        panic!("save result");
    };
    assert!(!outcome.changed);
    let saved = s.revision();
    assert_ne!(saved, initial);

    let mut replayed = 0;
    let replay = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-replay",
                "project.save",
                json!({}),
                "save",
                &initial,
            ),
            caller(&Allow),
            |_, _| {
                replayed += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(replayed, 0);
    let ControlResult::Edit { outcome } = ok(replay.clone()) else {
        panic!("replay result");
    };
    assert!(!outcome.changed);
    assert_eq!(replay.rev_after, saved);

    let conflict = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-conflict",
                "project.save",
                json!({}),
                "save",
                &s.revision(),
            ),
            caller(&Allow),
            |_, _| panic!("conflicting save must not publish"),
        )
        .unwrap();
    error(conflict, ErrorCode::IdempotencyConflict);

    let before_failure = s.revision();
    let mut failure_publishes = 0;
    let failure = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-fail",
                "project.save",
                json!({}),
                "fail",
                &before_failure,
            ),
            caller(&Allow),
            |_, _| {
                failure_publishes += 1;
                Err(EditError::Io(std::io::Error::other("disk")))
            },
        )
        .unwrap();
    assert_eq!(failure_publishes, 1);
    error(failure, ErrorCode::Io);
    assert_eq!(s.revision(), before_failure);

    let mut replay_publishes = 0;
    let replay_failure = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-fail-replay",
                "project.save",
                json!({}),
                "fail",
                &before_failure,
            ),
            caller(&Allow),
            |_, _| {
                replay_publishes += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(replay_publishes, 0);
    error(replay_failure, ErrorCode::Io);
    assert_eq!(s.revision(), before_failure);

    let conflict = run(
        &mut s,
        &mut runner(),
        "timeline.add_track",
        json!({"name":"Conflict","kind":"video"}),
        "fail",
    );
    error(conflict, ErrorCode::IdempotencyConflict);

    let retry = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-retry",
                "project.save",
                json!({}),
                "fresh",
                &before_failure,
            ),
            caller(&Allow),
            |_, _| Ok(()),
        )
        .unwrap();
    assert!(matches!(ok(retry), ControlResult::Edit { .. }));
    assert_ne!(s.revision(), before_failure);

    let invalid = s.control_lifecycle_save(
        request_at(&s, "", "project.save", json!({}), "fresh", &s.revision()),
        caller(&Allow),
        |_, _| panic!("invalid request must not publish"),
    );
    assert!(matches!(
        invalid,
        Err(EditError::Control(engine::control::ControlError::Limit))
    ));
}
