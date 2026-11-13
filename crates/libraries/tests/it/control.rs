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
            stickers: None,
            brand: None,
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
            stickers: None,
            brand: None,
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
            stickers: None,
            brand: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
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

// Sticker catalogs, search and recents: the bundled flag assets, the flag and shape search rows, and the recents list with its file round trip.
fn flag_asset_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/stickers/flags")
}

#[test]
fn flag_assets_are_all_well_formed_svg_and_counted() {
    let mut count = 0;
    for entry in std::fs::read_dir(flag_asset_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("svg") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let document = roxmltree::Document::parse(&text)
            .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
        assert_eq!(document.root_element().tag_name().name(), "svg");
        count += 1;
    }
    assert_eq!(count, 271, "every catalogued flag has one asset");
}

#[test]
fn flag_search_matches_names_codes_regions_and_continents() {
    use libraries::stickers::search_flags;
    let codes = |query: &str| {
        search_flags(query, 500)
            .items
            .iter()
            .map(|item| item.code)
            .collect::<Vec<_>>()
    };

    let everything = search_flags("", 100);
    assert_eq!(
        (
            everything.total,
            everything.items.len(),
            everything.has_more
        ),
        (271, 100, true),
        "empty query returns every row, limited, with a total"
    );
    assert_eq!(codes(""), codes("   "), "whitespace is an empty query");
    assert_eq!(codes("catalonia"), ["ES-CT"]);
    assert_eq!(codes("  Saint Helena "), ["SH-HL"]);
    assert_eq!(codes("ascension"), ["SH-AC"]);
    assert_eq!(
        codes("Organisations"),
        ["ARAB", "ASEAN", "CEFTA", "EAC", "UN"],
        "a region id matches case-insensitively, in catalog order"
    );
    assert_eq!(codes("subdivisions").len(), 12);
    let middle_east = search_flags("west asia", 500);
    assert!(!middle_east.items.is_empty());
    assert!(
        middle_east
            .items
            .iter()
            .all(|item| item.region == Some("Middle East")),
        "a region alias filters by region"
    );
    let europe = search_flags("europe", 500);
    assert!(!europe.items.is_empty());
    assert!(europe.items.iter().all(|item| {
        matches!(
            item.region,
            Some("Western Europe" | "Eastern Europe" | "Northern Europe" | "Southern Europe")
        )
    }));
    assert!(codes("zzzz-no-match").is_empty());
}

#[test]
fn shape_search_keeps_catalog_order_and_matches_names() {
    use libraries::stickers::search_shapes;
    let values = |query: &str| {
        search_shapes(query)
            .iter()
            .map(|shape| shape.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        values(""),
        ["square", "circle", "triangle", "hexagon", "diamond", "star"]
    );
    assert_eq!(values("  TRI "), ["triangle"]);
    assert!(values("zzz").is_empty());
}

#[test]
fn recent_stickers_cap_order_dedupe_and_clear() {
    use libraries::stickers::{FLAGS_PROVIDER, Recents, build_sticker_id, search_flags};
    let ids: Vec<String> = search_flags("", 500)
        .items
        .iter()
        .map(|item| build_sticker_id(FLAGS_PROVIDER, item.code))
        .collect();
    let mut recents = Recents::default();
    for id in &ids {
        assert!(recents.record(id));
    }
    assert_eq!(recents.ids().len(), 50, "the list keeps the newest 50");
    assert_eq!(recents.ids()[0], ids[270], "newest first");
    assert_eq!(recents.ids()[49], ids[221], "the oldest kept entry is last");

    assert!(recents.record(&ids[221]));
    assert_eq!(
        recents.ids().len(),
        50,
        "re-recording does not grow the list"
    );
    assert_eq!(
        recents.ids()[0],
        ids[221],
        "re-recording moves the id to the front"
    );
    assert_eq!(recents.ids()[1], ids[270]);

    assert!(
        recents.record(" flags:us "),
        "ids are trimmed and canonicalised"
    );
    assert_eq!(recents.ids()[0], "flags:US");
    assert!(
        !recents.record("flags:ZZ"),
        "unknown flag codes are refused"
    );
    assert!(!recents.record("nope:x"), "unknown providers are refused");
    assert!(
        !recents.record("shapes:pentagon"),
        "unknown shapes are refused"
    );
    assert_eq!(recents.ids().len(), 50);

    let loaded = Recents::from_ids(
        ["flags:ad", "flags:AD", "bad", "shapes:star"]
            .map(str::to_owned)
            .into_iter(),
    );
    assert_eq!(
        loaded.ids(),
        ["flags:AD", "shapes:star"],
        "dedupe keeps the first"
    );

    recents.clear();
    assert!(recents.ids().is_empty());
}

// Sticker commands through the real catalog: browse and search pages, `stickers.insert` with
// undo, redo and a project round trip, and the refusals for an unknown id or a missing catalog.
fn control_stickers(
    s: &mut Session,
    r: &mut JobRunner,
    stickers: Option<&engine::control::StickerAccess<'_>>,
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
            music: None,
            stickers,
            brand: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        },
        r,
    )
    .unwrap()
}

#[test]
fn sticker_commands_search_insert_undo_redo_and_round_trip() {
    use engine::control::{StickerCategory, StickerEntry, StickerPage, StickerQuery};
    use libraries::stickers::{resolve_sticker, search_catalog};
    let page = search_catalog("", StickerCategory::All, Some(8));
    assert_eq!(page.total, 277, "six shapes and 271 flags");
    let ids: Vec<&str> = page.results.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "shapes:square",
            "shapes:circle",
            "shapes:triangle",
            "shapes:hexagon",
            "shapes:diamond",
            "shapes:star",
            "flags:AD",
            "flags:AE"
        ]
    );
    assert_eq!((page.results[0].width, page.results[0].height), (512, 512));
    assert_eq!((page.results[6].width, page.results[6].height), (640, 480));
    let flags = search_catalog("", StickerCategory::Flags, Some(1000));
    assert_eq!((flags.total, flags.results.len()), (271, 271));
    let shapes = search_catalog("", StickerCategory::Shapes, None);
    assert_eq!((shapes.total, shapes.results.len()), (6, 6));
    assert!(search_catalog("asia", StickerCategory::Flags, None).total > 0);
    assert_eq!(
        search_catalog("asia", StickerCategory::Shapes, None).total,
        0
    );
    assert_eq!(resolve_sticker(" flags:vn ").unwrap().id, "flags:VN");
    assert_eq!(resolve_sticker("shapes:star").unwrap().name, "Star");
    assert!(resolve_sticker("flags:zz").is_none());
    assert!(resolve_sticker("emoji:smile").is_none());

    let search = |query: &StickerQuery| -> control::Result<StickerPage> {
        Ok(search_catalog(&query.query, query.category, query.limit))
    };
    let resolve = |id: &str| -> control::Result<StickerEntry> {
        resolve_sticker(id).ok_or(control::Error::Edit(engine::EditError::Invalid(
            "unknown sticker id",
        )))
    };
    let access = engine::control::StickerAccess::new(&search, &resolve);
    let mut s = session();
    let mut r = runner();
    let initial = s.project().clone();

    let ControlResult::StickerSearch { page } = ok(control_stickers(
        &mut s,
        &mut r,
        Some(&access),
        "stickers.search",
        json!({"query": "vietnam", "category": "flags"}),
        "search",
    )) else {
        panic!("stickers.search result");
    };
    assert_eq!(
        page.results
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["flags:VN"]
    );
    assert!(Arc::ptr_eq(&initial, s.project()), "search is not an edit");

    error(
        control_stickers(&mut s, &mut r, None, "stickers.search", json!({}), "bare"),
        ErrorCode::Refused,
    );
    for (key, params) in [
        (
            "unknown",
            json!({"sticker": "flags:zz", "start": 0, "duration": 1000}),
        ),
        (
            "negative",
            json!({"sticker": "flags:vn", "start": 0, "duration": -1}),
        ),
    ] {
        let response = control_stickers(
            &mut s,
            &mut r,
            Some(&access),
            "stickers.insert",
            params,
            key,
        );
        assert!(
            matches!(response.value, ResponseValue::Error(_)),
            "{key}: {response:?}"
        );
        assert!(
            Arc::ptr_eq(&initial, s.project()),
            "{key} leaves the project unchanged"
        );
    }

    let ControlResult::Edit { outcome } = ok(control_stickers(
        &mut s,
        &mut r,
        Some(&access),
        "stickers.insert",
        json!({"sticker": "flags:vn", "start": 0, "duration": 254_016_000_000_i64}),
        "insert",
    )) else {
        panic!("stickers.insert result");
    };
    assert!(outcome.changed);
    let placement = outcome.placements[0];
    let inserted = s.project().clone();
    let track = inserted
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.id == placement.track)
        .unwrap();
    assert_eq!(track.kind, project::TrackKind::Sticker);
    let item = track
        .items
        .iter()
        .find(|item| item.id == placement.item)
        .unwrap();
    assert_eq!(
        item.content,
        project::ItemParams::Sticker(project::StickerParams::new("flags:VN", 640, 480))
    );
    assert!(item.masks.is_empty());

    s.undo().unwrap();
    assert!(
        s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .all(|track| track.items.is_empty())
    );
    s.redo().unwrap();
    assert_eq!(s.project(), &inserted);

    let bytes = project::encode(&inserted, false).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("\"sticker\":\"flags:VN\"") && !text.contains("recents"));
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(&loaded.project, inserted.as_ref());
}

#[test]
fn flag_catalog_matches_the_bundled_flag_files() {
    // `assets/stickers/flags` owns the id list; render's byte table is checked against it too.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/stickers/flags");
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().ok()?;
            name.strip_suffix(".svg").map(str::to_owned)
        })
        .collect();
    files.sort();
    let mut codes: Vec<String> = libraries::stickers::search_flags("", usize::MAX)
        .items
        .iter()
        .map(|item| item.code.to_ascii_lowercase())
        .collect();
    codes.sort();
    assert_eq!(codes, files);
}
