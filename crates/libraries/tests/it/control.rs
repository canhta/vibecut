#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
//! The `fonts.*` control commands through the real store and the shared runner.

use engine::control::{
    self, Caller, ControlResult, ErrorCode, FontAccess, MusicAccess, Policy, Request, Response,
    ResponseValue, SoundAccess,
};
use engine::jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock};
use engine::{ByteBudget, Command, Session};
use project::{Project, ProjectId};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::example_store;

struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &Project) -> engine::control::Result<()> {
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
            sounds: None,
            music: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
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
fn control_sounds(
    s: &mut Session,
    r: &mut JobRunner,
    sounds: Option<&SoundAccess<'_>>,
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
            fonts: None,
            sounds,
            music: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        },
        r,
    )
    .unwrap()
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
    let import = |runner: &mut JobRunner, path: &Path, accepted: bool| {
        store
            .submit_import(runner, path.to_owned(), accepted)
            .map(|(id, _)| id)
    };
    let fonts = FontAccess::new(&list, &load, &download, &import);
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

    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("font.ttf");
    std::fs::write(&path, crate::font_catalog::FIXTURE).unwrap();
    error(
        control(
            &mut s,
            &mut r,
            Some(&fonts),
            "fonts.import",
            json!({"path":path,"accept_license":false}),
            "import-denied",
        ),
        ErrorCode::Permission,
    );
    let params = json!({"path":path,"accept_license":true});
    let accepted = control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.import",
        params.clone(),
        "import-once",
    );
    let replay = control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.import",
        params,
        "import-once",
    );
    assert_eq!(
        serde_json::to_value(&accepted).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    let ControlResult::JobAccepted { job, paths, .. } = ok(accepted) else {
        panic!("import job");
    };
    assert!(paths.is_empty());
    finish(&mut r, job);
    let ControlResult::Font { font } = ok(control(
        &mut s,
        &mut r,
        Some(&fonts),
        "fonts.load",
        json!({"family":"VibeCut Fixture Sans"}),
        "imported-load",
    )) else {
        panic!("loaded import");
    };
    assert_eq!(font.origin, Some(engine::control::FontOrigin::Custom));
    assert_eq!(font.state, engine::control::FontLoadState::Loaded);

    // A session without a store refuses every font command; none is a project edit.
    let mut bare = session();
    for (id, params) in [
        ("fonts.list", json!({})),
        ("fonts.load", json!({"family":"Inter"})),
        ("fonts.import", json!({"path":path,"accept_license":true})),
        (
            "fonts.download",
            json!({"family":"Lobster","accept_license":true}),
        ),
    ] {
        error(
            control(&mut bare, &mut r, None, id, params, id),
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

fn sound(id: u64, downloads: u64) -> libraries::sounds::SoundResult {
    libraries::sounds::SoundResult {
        id,
        name: "Whoosh".into(),
        author: "Freesound user".into(),
        licence: "Creative Commons 0".into(),
        attribution: "Whoosh by Freesound user (Creative Commons 0)".into(),
        source_url: format!("https://freesound.org/s/{id}/"),
        audio_url: format!("https://cdn.example/{id}.mp3"),
        duration_ms: 100,
        downloads,
    }
}

#[test]
fn sound_commands_return_typed_results_and_never_edit_the_project() {
    let root = tempfile::tempdir().unwrap();
    let library = libraries::sounds::SoundLibrary::open(root.path()).unwrap();
    library.save(&sound(5, 10)).unwrap();
    let search = |query: &engine::control::SoundQuery| {
        let request = libraries::sounds::SearchRequest {
            query: query.query.clone(),
            page: query.page,
            page_size: query.page_size,
            commercial_only: query.commercial_only,
        };
        Ok(library
            .search_local(&request)?
            .into_control(query.commercial_only))
    };
    let save =
        |entry: &engine::control::SoundEntry| -> engine::control::Result<engine::control::SavedSoundEntry> {
            Ok(library
                .save(&libraries::sounds::SoundResult::from(entry))?
                .into())
        };
    let unsave = |id: u64| -> engine::control::Result<bool> { Ok(library.unsave(id)?) };
    let sounds = SoundAccess::new(&search, &save, &unsave);
    let mut s = session();
    let mut r = runner();
    let revision = s.revision();

    let ControlResult::SoundSearch { page } = ok(control_sounds(
        &mut s,
        &mut r,
        Some(&sounds),
        "sounds.search",
        json!({"query":"whoosh","page":1,"page_size":15,"commercial_only":true}),
        "a",
    )) else {
        panic!("sounds.search result");
    };
    assert_eq!(page.source, engine::control::SoundSource::Local);
    assert_eq!(page.results.len(), 1);
    assert_eq!(page.results[0].id, 5);
    assert!(page.commercial_only);

    let entry = engine::control::SoundEntry::from(sound(6, 20));
    let ControlResult::SoundSaved { saved } = ok(control_sounds(
        &mut s,
        &mut r,
        Some(&sounds),
        "sounds.save",
        json!({"sound": entry.clone()}),
        "b",
    )) else {
        panic!("sounds.save result");
    };
    assert_eq!(saved.id, 6);
    assert_eq!(saved.licence, "Creative Commons 0");

    let ControlResult::SoundUnsaved { id, removed } = ok(control_sounds(
        &mut s,
        &mut r,
        Some(&sounds),
        "sounds.unsave",
        json!({"id":6}),
        "c",
    )) else {
        panic!("sounds.unsave result");
    };
    assert_eq!((id, removed), (6, true));
    let ControlResult::SoundUnsaved { removed, .. } = ok(control_sounds(
        &mut s,
        &mut r,
        Some(&sounds),
        "sounds.unsave",
        json!({"id":6}),
        "d",
    )) else {
        panic!("sounds.unsave result");
    };
    assert!(!removed);

    // A session without the lent library refuses every sound command; none edits.
    let mut bare = session();
    for (id, params) in [
        (
            "sounds.search",
            json!({"query":"","page":1,"page_size":15,"commercial_only":true}),
        ),
        ("sounds.save", json!({"sound": entry})),
        ("sounds.unsave", json!({"id":5})),
    ] {
        error(
            control_sounds(&mut bare, &mut r, None, id, params, "e"),
            ErrorCode::Refused,
        );
    }
    assert_eq!(
        s.revision(),
        revision,
        "sound commands are not project edits"
    );
    assert!(!s.can_undo());
}

fn control_music(
    s: &mut Session,
    r: &mut JobRunner,
    music: Option<&MusicAccess<'_>>,
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
            fonts: None,
            sounds: None,
            music,
            media: None,
            chapters: None,
            captions: None,
        },
        r,
    )
    .unwrap()
}

fn music_track(id: &str) -> libraries::music::MusicResult {
    libraries::music::MusicResult {
        id: id.into(),
        title: "Night Walk".into(),
        author: "Example Artist".into(),
        licence: "CC BY 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
        attribution: "\"Night Walk\" by Example Artist is licensed under CC BY 4.0.".into(),
        source_url: format!("https://source.example/{id}"),
        audio_url: format!("https://cdn.example/{id}.mp3"),
        duration_ms: 100,
    }
}

#[test]
fn music_commands_return_typed_results_and_never_edit_the_project() {
    let root = tempfile::tempdir().unwrap();
    let library = libraries::music::MusicLibrary::open(root.path()).unwrap();
    library.save(&music_track("track-5")).unwrap();
    let mut s = session();
    let mut r = runner();
    let revision = s.revision();

    let ControlResult::MusicSearch { page } = library.with_control(None, |music| {
        ok(control_music(
            &mut s,
            &mut r,
            Some(music),
            "music.search",
            json!({"query":"night","tag":null,"page":1,"page_size":15,"saved":true}),
            "a",
        ))
    }) else {
        panic!("music.search result");
    };
    assert_eq!(page.source, engine::control::MusicSource::Local);
    assert_eq!(page.results.len(), 1);
    assert_eq!(page.results[0].id, "track-5");
    assert_eq!(
        page.results[0].licence_url,
        "https://creativecommons.org/licenses/by/4.0/"
    );

    // A remote search is refused before any credential or request without the
    // composition-root approval, while Saved still works.
    library.with_control(None, |music| {
        error(
            control_music(
                &mut s,
                &mut r,
                Some(music),
                "music.search",
                json!({"query":"night","tag":null,"page":1,"page_size":15,"saved":false}),
                "b",
            ),
            ErrorCode::Sounds,
        );
    });

    let entry = engine::control::MusicEntry::from(music_track("track-6"));
    let ControlResult::MusicSaved { saved } = library.with_control(None, |music| {
        ok(control_music(
            &mut s,
            &mut r,
            Some(music),
            "music.save",
            json!({"music": entry.clone()}),
            "c",
        ))
    }) else {
        panic!("music.save result");
    };
    assert_eq!(saved.music.id, "track-6");
    assert_eq!(saved.music.licence, "CC BY 4.0");

    let ControlResult::MusicUnsaved { removed, .. } = library.with_control(None, |music| {
        ok(control_music(
            &mut s,
            &mut r,
            Some(music),
            "music.unsave",
            json!({"id":"track-6"}),
            "d",
        ))
    }) else {
        panic!("music.unsave result");
    };
    assert!(removed);

    // A session without the lent library refuses every music command; none edits.
    let mut bare = session();
    for (id, params) in [
        (
            "music.search",
            json!({"query":"","tag":null,"page":1,"page_size":15,"saved":true}),
        ),
        ("music.save", json!({"music": entry})),
        ("music.unsave", json!({"id":"track-5"})),
    ] {
        error(
            control_music(&mut bare, &mut r, None, id, params, "e"),
            ErrorCode::Refused,
        );
    }
    assert_eq!(
        s.revision(),
        revision,
        "music commands are not project edits"
    );
    assert!(!s.can_undo());
}
