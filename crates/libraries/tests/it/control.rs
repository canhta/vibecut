#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
//! The `fonts.*` control commands through the real store and the shared runner.

use engine::control::{
    self, Caller, ControlResult, ErrorCode, FontAccess, Policy, Request, Response, ResponseValue,
};
use engine::jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock};
use engine::{ByteBudget, Command, Session};
use project::{Project, ProjectId};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use crate::example_store;

struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &Project) -> engine::Result<()> {
        Ok(())
    }
}

fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 16,
            retained: 16,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}
fn session() -> Session {
    Session::new(
        Project::empty(ProjectId(u64::MAX), "Test".into()).unwrap(),
        10,
    )
    .unwrap()
}
fn request(s: &Session, id: &str, params: Value, key: &str) -> Request {
    Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: s.session_id().into(),
        request_id: "request".into(),
        idempotency_key: Some(key.into()),
        base_rev: Some(s.revision()),
        command: id.into(),
        params,
    }
}
fn control<'a>(
    s: &mut Session,
    r: &mut JobRunner,
    fonts: Option<&'a FontAccess<'a>>,
    id: &str,
    params: Value,
    key: &str,
) -> Response {
    let req = request(s, id, params, key);
    s.control_request(
        req,
        Caller {
            principal: "test",
            external: true,
            policy: &Allow,
            fonts,
        },
        r,
    )
    .unwrap()
}
fn ok(response: Response) -> ControlResult {
    match response.value {
        ResponseValue::Result(value) => value,
        ResponseValue::Error(error) => panic!("unexpected error {error:?}"),
    }
}
fn error(response: Response, code: ErrorCode) {
    assert!(
        matches!(response.value, ResponseValue::Error(found) if found == code),
        "{response:?}"
    );
}
fn finish(r: &mut JobRunner, id: JobId) {
    for _ in 0..20_000 {
        r.tick().unwrap();
        let snapshot = r.snapshot(id).unwrap();
        if matches!(
            snapshot.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            assert_eq!(snapshot.state, JobState::Completed, "{:?}", snapshot.error);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("job deadline");
}

#[test]
fn font_commands_return_typed_results_and_refuse_without_a_store_or_acceptance() {
    let store = example_store();
    let list = |query: Option<&str>| store.list(query);
    let load = |family: &str| store.load(family);
    let download = |runner: &mut JobRunner, family: &str, accepted: bool| {
        store
            .submit_download(runner, family, accepted)
            .map(|(job, _)| job)
    };
    let fonts = FontAccess::new(&list, &load, &download);
    let mut s = session();
    let mut r = runner();
    let revision = s.revision();
    let ControlResult::Fonts { entries } = ok(control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.list",
        json!({"query":"lob"}),
        "a",
    )) else {
        panic!("fonts.list result");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].family, "Lobster");
    assert_eq!(entries[0].state, engine::control::FontState::NotDownloaded);
    // A null or omitted query lists every family.
    for params in [json!({}), json!({"query":null})] {
        let ControlResult::Fonts { entries } = ok(control(
            &mut s,
            &mut r,
            Some(&fonts),
            "fonts.list",
            params,
            "b",
        )) else {
            panic!("fonts.list result");
        };
        assert!(entries.iter().any(|entry| entry.family == "Inter"));
    }
    let ControlResult::Font { font } = ok(control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.load",
        json!({"family":"Lobster"}),
        "c",
    )) else {
        panic!("fonts.load result");
    };
    assert_eq!(font.state, engine::control::FontLoadState::NotDownloaded);
    assert_eq!(font.effective_family, "Inter");

    // Download needs the explicit acknowledgement and a catalog family.
    error(
        control(
            &mut s,
            &mut r,
            Some(&fonts),
            "fonts.download",
            json!({"family":"Lobster","accept_license":false}),
            "d",
        ),
        ErrorCode::Permission,
    );
    error(
        control(
            &mut s,
            &mut r,
            Some(&fonts),
            "fonts.download",
            json!({"family":"Inter","accept_license":true}),
            "e",
        ),
        ErrorCode::Refused,
    );
    let ControlResult::JobAccepted { job, paths, .. } = ok(control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.download",
        json!({"family":"Lobster","accept_license":true}),
        "f",
    )) else {
        panic!("fonts.download result");
    };
    assert!(paths.is_empty());
    finish(&mut r, job);
    let ControlResult::Font { font } = ok(control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.load",
        json!({"family":"lobster"}),
        "g",
    )) else {
        panic!("fonts.load result");
    };
    assert_eq!(font.state, engine::control::FontLoadState::Loaded);

    // A session without a store refuses every font command; none is a project edit.
    let mut bare = session();
    for (id, params) in [
        ("fonts.list", json!({})),
        ("fonts.load", json!({"family":"Inter"})),
        (
            "fonts.download",
            json!({"family":"Lobster","accept_license":true}),
        ),
    ] {
        error(
            control(&mut bare, &mut r, None, id, params, "h"),
            ErrorCode::Refused,
        );
    }
    assert_eq!(
        s.revision(),
        revision,
        "font commands are not project edits"
    );
    assert!(!s.can_undo());
}
