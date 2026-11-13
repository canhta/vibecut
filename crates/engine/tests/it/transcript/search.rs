#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "integration fixtures"
)]
use crate::support::control::{Allow, media_access, ok, request, runner, seed};
use engine::Session;
use engine::control::{Caller, ControlResult, MediaAccess};
use engine::{
    ByteBudget,
    cache::ArtifactCache,
    transcript::search::{Answer, Segment, Source, fold, query},
};
use media::AssetId;
use project::{ItemParams, Project};
use serde_json::json;
use std::sync::Mutex;
use time::Tick;

fn cache(dir: &tempfile::TempDir) -> ArtifactCache {
    let budget = ByteBudget::new(64 << 20).unwrap();
    ArtifactCache::open(dir.path(), budget, 64).unwrap().0
}

fn segment(start: i64, end: i64, text: &str) -> Segment {
    Segment {
        start: Tick(start),
        end: Tick(end),
        text: text.to_owned(),
    }
}

fn source(content: u8, duration: Option<i64>, segments: Vec<Segment>) -> Source {
    Source {
        asset: AssetId(7),
        content: [content; 32],
        duration: duration.map(Tick),
        segments,
    }
}

fn vietnamese() -> Source {
    source(
        1,
        Some(10_000),
        vec![
            segment(0, 2_000, "Xin chào các bạn"),
            segment(2_000, 4_000, "Hôm nay trời đẹp"),
            segment(4_000, 10_000, "你好世界"),
        ],
    )
}

#[test]
fn folding_unifies_diacritics_compatibility_forms_and_case() {
    assert_eq!(fold("Tiếng Việt"), "tieng viet");
    assert_eq!(fold("Đà Nẵng"), "da nang");
    assert_eq!(fold("ＡＢＣ"), "abc");
    assert_eq!(fold("Cha\u{300}o"), fold("Chào"));
    assert_eq!(fold("Chào"), "chao");
    assert_eq!(fold("你好"), "你好");
}

#[test]
fn query_matches_folded_text_and_returns_asset_time_and_snippet() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    let sources = [vietnamese()];

    let answer = query(&mut cache, &sources, "chao", 10).unwrap();
    assert_eq!(answer.hits.len(), 1);
    let hit = &answer.hits[0];
    assert_eq!(hit.asset, AssetId(7));
    assert_eq!((hit.start, hit.end), (Tick(0), Tick(2_000)));
    assert_eq!(hit.snippet, "Xin chào các bạn");

    let accented = query(&mut cache, &sources, "HÔM NAY", 10).unwrap();
    assert_eq!(accented.hits.len(), 1);
    assert_eq!(accented.hits[0].start, Tick(2_000));
    assert_eq!(
        query(&mut cache, &sources, "dep", 10).unwrap().hits.len(),
        1
    );
    assert_eq!(
        query(&mut cache, &sources, "đẹp", 10).unwrap().hits.len(),
        1
    );
    assert_eq!(
        query(&mut cache, &sources, "好世", 10).unwrap().hits[0].start,
        Tick(4_000)
    );
    assert!(
        query(&mut cache, &sources, "không", 10)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn index_round_trips_through_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    let sources = [vietnamese()];

    let built = query(&mut cache, &sources, "chao", 10).unwrap();
    assert_eq!(built.rebuilt, 1);
    let reused = query(&mut cache, &sources, "chao", 10).unwrap();
    assert_eq!(reused.rebuilt, 0);
    assert_eq!(reused.hits, built.hits);
    assert!(!reused.stale);
    assert!(!reused.partial);
}

#[test]
fn stale_index_is_rebuilt_when_media_or_transcript_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    let first = query(&mut cache, &[vietnamese()], "chao", 10).unwrap();
    assert_eq!((first.rebuilt, first.stale), (1, false));

    let replaced_media = query(
        &mut cache,
        &[source(2, Some(10_000), vietnamese().segments)],
        "chao",
        10,
    )
    .unwrap();
    assert_eq!((replaced_media.rebuilt, replaced_media.stale), (1, true));
    assert_eq!(replaced_media.hits, first.hits);

    let settled = query(
        &mut cache,
        &[source(2, Some(10_000), vietnamese().segments)],
        "chao",
        10,
    )
    .unwrap();
    assert_eq!((settled.rebuilt, settled.stale), (0, false));

    let edited = {
        let mut segments = vietnamese().segments;
        segments[0] = segment(0, 2_000, "Xin chào mọi người");
        source(2, Some(10_000), segments)
    };
    let after_edit = query(&mut cache, &[edited], "moi nguoi", 10).unwrap();
    assert_eq!((after_edit.rebuilt, after_edit.stale), (1, true));
    assert_eq!(after_edit.hits.len(), 1);
    let reverted = query(&mut cache, &[vietnamese()], "moi nguoi", 10).unwrap();
    assert_eq!((reverted.rebuilt, reverted.stale), (1, true));
    assert!(reverted.hits.is_empty());
}

#[test]
fn transcript_covering_less_than_media_is_partial() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    let short: Answer = query(
        &mut cache,
        &[source(1, Some(10_000), vec![segment(0, 4_000, "chao")])],
        "chao",
        10,
    )
    .unwrap();
    assert!(short.partial);

    let full = query(
        &mut cache,
        &[source(1, Some(4_000), vec![segment(0, 4_000, "chao")])],
        "chao",
        10,
    )
    .unwrap();
    assert!(!full.partial);

    let unknown = query(
        &mut cache,
        &[source(1, None, vec![segment(0, 4_000, "chao")])],
        "chao",
        10,
    )
    .unwrap();
    assert!(!unknown.partial);
}

#[test]
fn limit_caps_hits_and_invalid_queries_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    let sources = [vietnamese()];

    assert_eq!(query(&mut cache, &sources, "a", 1).unwrap().hits.len(), 1);
    assert!(query(&mut cache, &sources, "   ", 10).is_err());
    assert!(query(&mut cache, &sources, "chao", 0).is_err());
    assert!(query(&mut cache, &sources, "chao", 201).is_err());
    assert!(query(&mut cache, &sources, &"x".repeat(257), 10).is_err());
}

/// The seed session with its caption cue text replaced; the caption item keeps its source asset.
fn with_cue_text(base: &Session, text: &str) -> Session {
    let mut data = base.project().data().clone();
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            for cue in &mut params.cues.cues {
                for cue_text in cue.texts.values_mut() {
                    cue_text.text = text.into();
                }
            }
        }
    }
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

/// One `transcript.search` request through the control dispatch, as an agent sends it.
fn search(s: &mut Session, media: &Mutex<MediaAccess>, key: &str, query: &str) -> ControlResult {
    let mut r = runner();
    let allow = Allow;
    let req = request(s, "transcript.search", json!({ "query": query }), key);
    ok(s.control_request(
        req,
        Caller {
            principal: "local-mcp",
            external: true,
            policy: &allow,
            fonts: None,
            sounds: None,
            music: None,
            stickers: None,
            brand: None,
            media: Some(media),
            chapters: None,
            captions: None,
            vision: None,
        },
        &mut r,
    )
    .unwrap())
}

#[test]
fn transcript_search_command_reports_hits_and_index_staleness() {
    let dir = tempfile::tempdir().unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut s = with_cue_text(&seed(), "Xin chào các bạn");

    // The seed's caption item carries asset 0 over one second; its cue covers all of it.
    let ControlResult::TranscriptSearch {
        hits,
        partial,
        stale,
        rebuilt,
    } = search(&mut s, &media, "search-first", "chao")
    else {
        panic!("transcript search result");
    };
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].asset, AssetId(0));
    assert_eq!(hits[0].start, Tick::ZERO);
    assert_eq!(hits[0].end, Tick(time::TICKS_PER_SECOND));
    assert_eq!(hits[0].snippet, "Xin chào các bạn");
    assert_eq!((partial, stale, rebuilt), (false, false, 1));

    // Same media and transcript: the cached index answers without a rebuild.
    let ControlResult::TranscriptSearch { stale, rebuilt, .. } =
        search(&mut s, &media, "search-repeat", "chao")
    else {
        panic!("transcript search result");
    };
    assert_eq!((stale, rebuilt), (false, 0));

    // Editing the cue changes the transcript; the next query rebuilds and reports stale.
    let mut edited = with_cue_text(&s, "Xin chào mọi người");
    let ControlResult::TranscriptSearch {
        hits,
        stale,
        rebuilt,
        ..
    } = search(&mut edited, &media, "search-edited", "moi nguoi")
    else {
        panic!("transcript search result");
    };
    assert_eq!((stale, rebuilt, hits.len()), (true, 1, 1));
    assert_eq!(hits[0].snippet, "Xin chào mọi người");

    // A word only the old wording had no longer matches.
    let ControlResult::TranscriptSearch { hits, .. } =
        search(&mut edited, &media, "search-old", "ban")
    else {
        panic!("transcript search result");
    };
    assert!(hits.is_empty());
}

#[test]
fn search_module_has_no_network_or_provider_imports() {
    let sources = [
        include_str!("../../../src/transcript/search.rs"),
        include_str!("../../../src/transcript/search/folding.rs"),
        include_str!("../../../src/transcript/search/index.rs"),
    ];
    let forbidden = [
        "transport",
        "ai::",
        "providers",
        "std::net",
        "TcpStream",
        "reqwest",
        "ureq",
        "http",
    ];
    for text in sources {
        for token in forbidden {
            assert!(!text.contains(token), "search must not use `{token}`");
        }
    }
}
