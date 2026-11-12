#![allow(clippy::unwrap_used, clippy::panic)]
use engine::jobs::{Clock, JobId, JobRunner, JobState, RunnerConfig};
use engine::{Command, Placement, Session};
use libraries::music::{
    MusicClient, MusicDownloads, MusicLibrary, MusicPreview, MusicResult, MusicToken, PreviewClip,
    PreviewSink, SearchRequest, add_saved,
};
use media::ContentHash;
use project::{AudioSource, ItemParams, Project, ProjectId};
use std::{
    io::{Cursor, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use time::Tick;
use transport::{DownloadTransport, HttpRequest, HttpResponse, HttpStreamResponse, HttpTransport};

const AMBIENT: &str = include_str!("../fixtures/music/ambient.json");
const CHILL: &str = include_str!("../fixtures/music/chill.json");

struct JsonTransport {
    response: HttpResponse,
    requests: Mutex<Vec<HttpRequest>>,
}
impl HttpTransport for JsonTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, transport::Error> {
        self.send_bounded(request, 1 << 20)
    }
    fn send_bounded(
        &self,
        request: HttpRequest,
        max_bytes: usize,
    ) -> Result<HttpResponse, transport::Error> {
        self.requests.lock().unwrap().push(request);
        if self.response.body.len() > max_bytes {
            return Err(transport::Error::Payload);
        }
        Ok(HttpResponse {
            status: self.response.status,
            headers: self.response.headers.clone(),
            body: self.response.body.clone(),
        })
    }
}
fn client(body: &str) -> (MusicClient, Arc<JsonTransport>) {
    let transport = Arc::new(JsonTransport {
        response: HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        },
        requests: Mutex::new(Vec::new()),
    });
    (MusicClient::new(None, transport.clone()), transport)
}
fn request() -> SearchRequest {
    SearchRequest {
        query: "ambient".into(),
        tag: None,
        page: 1,
        page_size: 5,
    }
}
fn track(id: &str, licence: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": format!("Track {id}"),
        "creator": format!("artist-{id}"),
        "url": format!("https://cdn.example/{id}.mp3"),
        "foreign_landing_url": format!("https://source.example/{id}"),
        "license": licence,
        "license_version": if licence == "cc0" { "1.0" } else { "4.0" },
        "license_url": if licence == "cc0" { "https://creativecommons.org/publicdomain/zero/1.0/" } else { "https://creativecommons.org/licenses/by/4.0/" },
        "attribution": format!("Track {id} by artist-{id}"),
        "duration": 31_000,
    })
}
fn page(results: Vec<serde_json::Value>, pages: u64) -> String {
    serde_json::json!({
        "result_count": 73,
        "page_count": pages,
        "page_size": 5,
        "page": 1,
        "results": results,
    })
    .to_string()
}
fn result(id: &str) -> MusicResult {
    MusicResult {
        id: id.into(),
        title: format!("Track {id}"),
        author: "Author".into(),
        licence: "CC BY 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
        attribution: format!("Track {id} by Author"),
        source_url: format!("https://source.example/{id}"),
        audio_url: format!("https://cdn.example/{id}.mp3"),
        duration_ms: 250,
    }
}

#[test]
fn search_requests_the_recorded_contract_and_admits_only_cc0_and_by() {
    let (client, transport) = client(AMBIENT);
    let page = client.search(&request()).unwrap();
    assert_eq!(page.count, 73);
    assert_eq!(page.page, 1);
    assert_eq!(page.page_size, 5);
    assert_eq!(page.results.len(), 5);
    assert!(page.has_next);
    assert_eq!(page.results[0].id, "7d573956-f2f0-457a-9ed6-3b2de0280c4e");
    assert_eq!(page.results[0].title, "Ambient Dance");
    assert_eq!(page.results[0].author, "Zeropage");
    assert_eq!(page.results[0].licence, "CC BY 3.0");
    assert_eq!(
        page.results[0].licence_url,
        "https://creativecommons.org/licenses/by/3.0/"
    );
    assert!(page.results[0].attribution.contains("Zeropage"));
    assert!(page.results[0].source_url.starts_with("https://"));
    assert!(page.results[0].audio_url.starts_with("https://"));
    assert_eq!(page.results[0].duration_ms, 228_000);
    assert!(
        page.results
            .iter()
            .all(|track| track.licence.starts_with("CC BY") || track.licence.starts_with("CC0"))
    );
    let requests = transport.requests.lock().unwrap();
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert!(
        request
            .url
            .starts_with("https://api.openverse.org/v1/audio/?"),
        "{}",
        request.url
    );
    assert!(request.url.contains("category=music"));
    assert!(request.url.contains("license=cc0%2Cby"));
    assert!(request.url.contains("page=1"));
    assert!(request.url.contains("page_size=5"));
    assert!(request.url.contains("filter_dead=true"));
    assert!(request.url.contains("mature=false"));
    assert!(request.url.contains("q=ambient"));
    assert!(!request.url.contains("tags="));
    assert!(!request.url.contains("license_type="));
    // Anonymous queries carry no credential at all.
    assert!(request.headers.is_empty());
}
#[test]
fn tag_search_uses_the_tag_filter_and_never_combines_it_with_free_text() {
    let (client, transport) = client(CHILL);
    let page = client
        .search(&SearchRequest {
            query: String::new(),
            tag: Some("chill".into()),
            page: 1,
            page_size: 4,
        })
        .unwrap();
    assert_eq!(page.results.len(), 3);
    assert!(!page.has_next);
    let requests = transport.requests.lock().unwrap();
    assert!(requests[0].url.contains("tags=chill"));
    assert!(!requests[0].url.contains("q="));
    drop(requests);
    assert!(matches!(
        client.search(&SearchRequest {
            query: "ambient".into(),
            tag: Some("chill".into()),
            page: 1,
            page_size: 4,
        }),
        Err(engine::MusicError::Invalid("search request"))
    ));
}
#[test]
fn a_user_key_travels_as_a_bearer_header_and_never_in_the_url() {
    let transport = Arc::new(JsonTransport {
        response: HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: AMBIENT.as_bytes().to_vec(),
        },
        requests: Mutex::new(Vec::new()),
    });
    let client = MusicClient::new(
        Some(MusicToken::new("user-key-value").unwrap()),
        transport.clone(),
    );
    client.search(&request()).unwrap();
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests[0].headers[0].0, "Authorization");
    assert_eq!(requests[0].headers[0].1, "Bearer user-key-value");
    assert!(!requests[0].url.contains("user-key-value"));
    assert!(!format!("{:?}", MusicToken::new("s3cr3t").unwrap()).contains("s3cr3t"));
    assert!(matches!(MusicToken::new(""), Err(engine::MusicError::Auth)));
    assert!(matches!(
        MusicToken::new("space key"),
        Err(engine::MusicError::Auth)
    ));
}
#[test]
fn results_the_server_should_have_filtered_are_dropped_not_shown() {
    let body = page(
        vec![
            track("keep-cc0", "cc0"),
            track("keep-by", "by"),
            track("nc", "by-nc"),
            track("sa", "by-sa"),
            track("nd", "by-nd"),
            track("sampling", "sampling+"),
            serde_json::json!({
                "id": "no-author",
                "title": "Anonymous",
                "url": "https://cdn.example/a.mp3",
                "foreign_landing_url": "https://source.example/a",
                "license": "by",
                "license_version": "4.0",
                "license_url": "https://creativecommons.org/licenses/by/4.0/",
                "duration": 1000,
            }),
            serde_json::json!({
                "id": "insecure",
                "title": "Insecure",
                "creator": "Author",
                "url": "http://cdn.example/a.mp3",
                "foreign_landing_url": "https://source.example/a",
                "license": "by",
                "license_version": "4.0",
                "license_url": "https://creativecommons.org/licenses/by/4.0/",
                "duration": 1000,
            }),
            serde_json::json!({
                "id": "hostile-title",
                "title": "x".repeat(5000),
                "creator": "Author",
                "url": "https://cdn.example/a.mp3",
                "foreign_landing_url": "https://source.example/a",
                "license": "by",
                "license_version": "4.0",
                "license_url": "https://creativecommons.org/licenses/by/4.0/",
                "duration": 1000,
            }),
            serde_json::json!({
                "id": "zero-duration",
                "title": "Zero",
                "creator": "Author",
                "url": "https://cdn.example/a.mp3",
                "foreign_landing_url": "https://source.example/a",
                "license": "by",
                "license_version": "4.0",
                "license_url": "https://creativecommons.org/licenses/by/4.0/",
                "duration": 0,
            }),
        ],
        1,
    );
    let (client, _) = client(&body);
    let page = client
        .search(&SearchRequest {
            page_size: 20,
            ..request()
        })
        .unwrap();
    let ids: Vec<_> = page.results.iter().map(|track| track.id.as_str()).collect();
    assert_eq!(ids, ["keep-cc0", "keep-by"]);
}
#[test]
fn hostile_search_responses_and_provider_errors_are_typed() {
    let (oversized, _) = client(&"x".repeat((1 << 20) + 1));
    assert!(matches!(
        oversized.search(&request()),
        Err(engine::MusicError::Limit)
    ));
    let (malformed, _) = client("{\"results\":[");
    assert!(matches!(
        malformed.search(&request()),
        Err(engine::MusicError::InvalidResponse)
    ));
    let too_many = page(
        (0..6)
            .map(|index| track(&format!("t{index}"), "cc0"))
            .collect(),
        1,
    );
    let (crowded, _) = client(&too_many);
    assert!(matches!(
        crowded.search(&request()),
        Err(engine::MusicError::InvalidResponse)
    ));
    for (status, headers, expected) in [
        (401, Vec::new(), "Auth"),
        (402, Vec::new(), "Quota"),
        (403, Vec::new(), "Auth"),
        (404, Vec::new(), "Invalid"),
        (408, Vec::new(), "Network"),
        (429, vec![("retry-after".into(), "7".into())], "RateLimit"),
        (502, Vec::new(), "Network"),
        (503, Vec::new(), "Network"),
        (500, Vec::new(), "Server"),
        (418, Vec::new(), "Invalid"),
    ] {
        let transport = Arc::new(JsonTransport {
            response: HttpResponse {
                status,
                headers,
                body: b"{\"detail\":\"hostile detail\"}".to_vec(),
            },
            requests: Mutex::new(Vec::new()),
        });
        let client = MusicClient::new(None, transport);
        let error = client.search(&request()).unwrap_err();
        match expected {
            "Auth" => assert!(matches!(error, engine::MusicError::Auth)),
            "Quota" => assert!(matches!(error, engine::MusicError::Quota)),
            "Network" => assert!(matches!(error, engine::MusicError::Network)),
            "RateLimit" => assert!(matches!(
                error,
                engine::MusicError::RateLimit {
                    retry_after_seconds: Some(7)
                }
            )),
            "Server" => assert!(matches!(error, engine::MusicError::Server)),
            "Invalid" => assert!(matches!(error, engine::MusicError::InvalidResponse)),
            _ => panic!("unexpected error variant"),
        }
    }
}
#[test]
fn bounded_request_shapes_are_refused_before_any_request() {
    let (client, transport) = client(AMBIENT);
    for bad in [
        SearchRequest {
            page_size: 21,
            ..request()
        },
        SearchRequest {
            page_size: 0,
            ..request()
        },
        SearchRequest {
            page: 0,
            ..request()
        },
        SearchRequest {
            page: 1001,
            ..request()
        },
        SearchRequest {
            query: "x".repeat(201),
            ..request()
        },
        SearchRequest {
            tag: Some("x".repeat(201)),
            ..request()
        },
    ] {
        assert!(matches!(
            client.search(&bad),
            Err(engine::MusicError::Invalid("search request"))
        ));
    }
    assert!(transport.requests.lock().unwrap().is_empty());
    for (query, tag) in [
        ("x".repeat(200), None),
        (String::new(), Some("x".repeat(200))),
    ] {
        client
            .search(&SearchRequest {
                query,
                tag,
                page: 1000,
                page_size: 20,
            })
            .unwrap();
    }
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[test]
fn saved_library_round_trips_offline_and_clear_all_removes_cache() {
    let root = tempfile::tempdir().unwrap();
    let music = result("7d573956-f2f0-457a-9ed6-3b2de0280c4e");
    {
        let library = MusicLibrary::open(root.path()).unwrap();
        assert!(matches!(
            MusicLibrary::open(root.path()),
            Err(engine::MusicError::Project(project::Error::Busy))
        ));
        library.save(&music).unwrap();
        assert!(library.is_saved(&music.id).unwrap());
        library.store_cached(&music.audio_url, &wav()).unwrap();
        assert!(library.cached(&music.audio_url).unwrap().is_some());
        let page = library
            .search_local(&SearchRequest {
                query: "track".into(),
                ..SearchRequest::default()
            })
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.source, libraries::music::SearchSource::Local);
        let stored = std::fs::read_to_string(root.path().join("library.json")).unwrap();
        assert!(!stored.contains("token"));
        assert!(!stored.contains(root.path().to_str().unwrap()));
    }
    let library = MusicLibrary::open(root.path()).unwrap();
    assert!(library.is_saved(&music.id).unwrap());
    assert!(library.cached(&music.audio_url).unwrap().is_some());
    library.unsave(&music.id).unwrap();
    assert!(!library.is_saved(&music.id).unwrap());
    assert!(library.cached(&music.audio_url).unwrap().is_none());
    library.save(&music).unwrap();
    library.store_cached(&music.audio_url, &wav()).unwrap();
    assert_eq!(library.clear_all().unwrap(), 1);
    assert!(library.saved().unwrap().is_empty());
    assert!(library.cached(&music.audio_url).unwrap().is_none());
    library.save(&music).unwrap();
    library.store_cached(&music.audio_url, &wav()).unwrap();
    let saved = library.saved().unwrap();
    let path = root.path().join("library.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    for operation in ["insert", "update", "unsave", "clear"] {
        let mut changed = music.clone();
        changed.audio_url.push_str("?updated=1");
        let failed = match operation {
            "insert" => {
                changed.id.push_str("-new");
                library.save(&changed).is_err()
            }
            "update" => library.save(&changed).is_err(),
            "unsave" => library.unsave(&music.id).is_err(),
            _ => library.clear_all().is_err(),
        };
        assert!(failed, "{operation} must report failed publication");
        assert_eq!(library.saved().unwrap(), saved, "{operation}");
        assert!(library.cached(&music.audio_url).unwrap().is_some());
    }
}
#[test]
fn changed_source_url_invalidates_the_cached_file() {
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let mut music = result("8");
    library.save(&music).unwrap();
    library.store_cached(&music.audio_url, &wav()).unwrap();
    assert!(library.cached(&music.audio_url).unwrap().is_some());
    music.audio_url = "https://cdn.example/8-v2.mp3".into();
    library.save(&music).unwrap();
    assert!(
        library
            .cached("https://cdn.example/8.mp3")
            .unwrap()
            .is_none()
    );
    assert!(library.cached(&music.audio_url).unwrap().is_none());
}
#[test]
fn local_search_honours_query_and_pagination() {
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    for (id, author, duration) in [
        ("1", "Alice", 1000),
        ("2", "Bob", 3000),
        ("3", "Alice", 2000),
    ] {
        let mut music = result(id);
        music.author = author.into();
        music.duration_ms = duration;
        library.save(&music).unwrap();
    }
    let page = library
        .search_local(&SearchRequest {
            query: "alice".into(),
            page: 1,
            page_size: 1,
            ..SearchRequest::default()
        })
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.results[0].id, "3");
    assert!(page.has_next);
    let page = library
        .search_local(&SearchRequest {
            query: String::new(),
            page: 1,
            page_size: 15,
            ..SearchRequest::default()
        })
        .unwrap();
    assert_eq!(page.count, 3);
    assert_eq!(page.results[0].id, "2");
    assert!(!page.has_next);
}
#[test]
fn library_refuses_oversized_library_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("library.json");
    std::fs::write(&path, vec![b'x'; (1 << 20) + 1]).unwrap();
    assert!(matches!(
        MusicLibrary::open(root.path()),
        Err(engine::MusicError::LibraryCorrupt)
    ));
}

#[test]
fn ingress_remote_keeps_canonical_siblings_and_drops_malformed_rows() {
    let identities = [
        ("cc0", "1.0", "CC0 1.0", "publicdomain/zero/1.0"),
        ("by", "1.0", "CC BY 1.0", "licenses/by/1.0"),
        ("by", "2.0", "CC BY 2.0", "licenses/by/2.0"),
        ("by", "2.5", "CC BY 2.5", "licenses/by/2.5"),
        ("by", "3.0", "CC BY 3.0", "licenses/by/3.0"),
        ("by", "4.0", "CC BY 4.0", "licenses/by/4.0"),
    ];
    for (family, version, display, deed) in identities {
        for slash in ["", "/"] {
            let mut good = track("good", family);
            good["license_version"] = version.into();
            good["license_url"] = format!("https://creativecommons.org/{deed}{slash}").into();
            good["title"] = "a\u{0301}".into();
            let mut contradictory = good.clone();
            contradictory["license_url"] = "https://creativecommons.org/licenses/by-nc/4.0/".into();
            let mut malformed = good.clone();
            malformed["title"] = 12.into();
            let mut missing = good.clone();
            missing["attribution"] = serde_json::Value::Null;
            let body = page(vec![good, contradictory, malformed, missing], 1);
            let (client, _) = client(&body);
            let found = client.search(&request()).unwrap();
            assert_eq!(found.results.len(), 1, "{display}{slash}");
            assert_eq!(found.results[0].licence, display);
            assert_eq!(found.results[0].title, "á");
            assert_eq!(
                found.results[0].licence_url,
                format!("https://creativecommons.org/{deed}{slash}")
            );
            let root = tempfile::tempdir().unwrap();
            let library = MusicLibrary::open(root.path()).unwrap();
            let saved = library.save(&found.results[0]).unwrap();
            assert_eq!(saved.music, found.results[0]);
            drop(library);
            assert_eq!(
                MusicLibrary::open(root.path()).unwrap().saved().unwrap(),
                [saved]
            );
        }
    }
    for (family, version, deed) in [
        ("by", "", "licenses/by/4.0/"),
        ("by", "9.0", "licenses/by/9.0/"),
        ("by", "3.0", "licenses/by/3.0/us/"),
        ("by", "4.0", "publicdomain/zero/1.0/"),
        ("cc0", "1.0", "licenses/by/1.0/"),
        ("pdm", "1.0", "publicdomain/mark/1.0/"),
        ("by-sa", "4.0", "licenses/by-sa/4.0/"),
        ("by-nd", "4.0", "licenses/by-nd/4.0/"),
        ("by-nc", "4.0", "licenses/by-nc/4.0/"),
        ("unknown", "4.0", "licenses/by/4.0/"),
    ] {
        let mut bad = track("bad", family);
        bad["license_version"] = version.into();
        bad["license_url"] = format!("https://creativecommons.org/{deed}").into();
        let (client, _) = client(&page(vec![bad, track("good", "by")], 1));
        let found = client.search(&request()).unwrap();
        assert_eq!(
            found
                .results
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["good"],
            "{family}/{version}/{deed}"
        );
    }
}

#[test]
fn ingress_public_save_reopen_and_add_refuse_before_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let good = result("guard");
    library.save(&good).unwrap();
    let cached = library.store_cached(&good.audio_url, &wav()).unwrap();
    let saved = library.saved().unwrap();
    let path = root.path().join("library.json");
    let before = std::fs::read(&path).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(1), "music".into()).unwrap(), 8).unwrap();
    let project = project::encode(session.project(), false).unwrap();
    let revision = session.revision();
    let absent_media = root.path().join("must-not-create");
    let absent_cache = libraries::music::CachedMusic {
        path: root.path().join("must-not-read"),
        ..cached.clone()
    };
    let mut rows = Vec::new();
    for (field, values) in [
        ("id", vec![String::new(), "é".into(), "x".repeat(129)]),
        (
            "title",
            vec![
                String::new(),
                " ".into(),
                "a\u{0085}b".into(),
                "x".repeat(4097),
            ],
        ),
        (
            "author",
            vec![String::new(), "a\nb".into(), "x".repeat(4097)],
        ),
        (
            "attribution",
            vec![String::new(), "a\u{0000}b".into(), "x".repeat(4097)],
        ),
        (
            "licence",
            vec![
                "CC BY-NC 4.0".into(),
                "CC BY-SA 4.0".into(),
                "CC BY-ND 4.0".into(),
                "PDM 1.0".into(),
                "CC BY".into(),
                "unknown".into(),
            ],
        ),
        (
            "licence_url",
            vec![
                "https://creativecommons.org/licenses/by-nc/4.0/".into(),
                "https://creativecommons.org/publicdomain/zero/1.0/".into(),
                "https://creativecommons.org/licenses/by/3.0/".into(),
                "https://creativecommons.org/licenses/by/4.0/us/".into(),
                "https://creativecommons.org/licenses/by/4.0/?q=1".into(),
            ],
        ),
        (
            "source_url",
            vec![
                "https://".into(),
                "https:///empty".into(),
                "https://user:secret@source.example/a".into(),
                "http://source.example/a".into(),
                format!("https://source.example/{}", "x".repeat(8192)),
            ],
        ),
        (
            "audio_url",
            vec![
                "https://:443/a".into(),
                "https://host:invalid/a".into(),
                "https://host/%zz".into(),
                "https://user@host/a".into(),
                format!("https://cdn.example/{}", "x".repeat(8192)),
            ],
        ),
    ] {
        for value in values {
            let mut json = serde_json::to_value(&good).unwrap();
            json[field] = value.into();
            rows.push(serde_json::from_value::<MusicResult>(json).unwrap());
        }
    }
    for duration_ms in [0, 86_400_001] {
        rows.push(MusicResult {
            duration_ms,
            ..good.clone()
        });
    }
    for bad in &rows {
        assert!(
            matches!(library.save(bad), Err(engine::MusicError::Project(_))),
            "save {bad:?}"
        );
        assert_eq!(library.saved().unwrap(), saved);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(library.cached(&good.audio_url).unwrap().is_some());
        assert!(
            matches!(
                libraries::music::add_to_timeline(
                    &mut session,
                    bad,
                    &absent_cache,
                    &absent_media,
                    Placement::Auto,
                    Tick::ZERO
                ),
                Err(engine::control::Error::Music(engine::MusicError::Project(
                    _
                )))
            ),
            "add {bad:?}"
        );
        assert!(!absent_media.exists());
        assert_eq!(session.revision(), revision);
        assert_eq!(session.history_len(), (0, 0));
        assert_eq!(project::encode(session.project(), false).unwrap(), project);
        if bad.licence == good.licence {
            let mut raw = track(&bad.id, "by");
            for (field, value) in [
                ("title", bad.title.as_str()),
                ("creator", bad.author.as_str()),
                ("attribution", bad.attribution.as_str()),
                ("license_url", bad.licence_url.as_str()),
                ("foreign_landing_url", bad.source_url.as_str()),
                ("url", bad.audio_url.as_str()),
            ] {
                raw[field] = value.into();
            }
            raw["duration"] = bad.duration_ms.into();
            let (client, _) = client(&page(vec![raw, track("sibling", "by")], 1));
            let found = client.search(&request()).unwrap();
            assert_eq!(
                found
                    .results
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                ["sibling"]
            );
        }
    }
    drop(library);
    let reopened_root = tempfile::tempdir().unwrap();
    let reopened_path = reopened_root.path().join("library.json");
    for bad in rows {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "format": "vibecut.music-library", "version": 1,
            "tracks": [{"music": bad, "saved_at_unix_ms": 1}]
        }))
        .unwrap();
        std::fs::write(&reopened_path, &bytes).unwrap();
        assert!(matches!(
            MusicLibrary::open(reopened_root.path()),
            Err(engine::MusicError::Project(_))
        ));
        assert_eq!(std::fs::read(&reopened_path).unwrap(), bytes);
        assert!(!reopened_root.path().join("audio").exists());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(cached.path.exists());
    }
}

#[test]
fn ingress_saved_normalizes_text_and_identical_save_preserves_file_and_time() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("library.json");
    let mut music = result("normal");
    music.title = "a\u{0301}".into();
    music.author = "e\u{0301}".into();
    music.attribution = "o\u{0301}".into();
    let stored = serde_json::json!({"format": "vibecut.music-library", "version": 1,
        "tracks": [{"music": music, "saved_at_unix_ms": 1}]});
    let bytes = serde_json::to_vec(&stored).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let saved = library.saved().unwrap().remove(0);
    assert_eq!(
        (
            &*saved.music.title,
            &*saved.music.author,
            &*saved.music.attribution
        ),
        ("á", "é", "ó")
    );
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(library.save(&music).unwrap(), saved);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    let mut changed = music.clone();
    changed.title = "o\u{0301}".into();
    let updated = library.save(&changed).unwrap();
    assert_eq!(updated.music.title, "ó");
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted["tracks"][0]["music"]["title"], "ó");
    drop(library);
    assert_eq!(
        MusicLibrary::open(root.path()).unwrap().saved().unwrap(),
        [updated]
    );
}

#[test]
fn ingress_saved_versions_drop_before_body_but_current_corruption_refuses() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("library.json");
    for version in [0, 2] {
        std::fs::write(&path, format!(r#"{{"format":"vibecut.music-library","version":{version},"tracks":"not a current body"}}"#)).unwrap();
        assert!(
            MusicLibrary::open(root.path())
                .unwrap()
                .saved()
                .unwrap()
                .is_empty()
        );
    }
    for text in [
        r#"{"format":"vibecut.music-library","version":1,"tracks":"bad"}"#,
        r#"{"format":"different","version":1,"tracks":[]}"#,
        r#"{"format":"vibecut.music-library","version":1,"tracks":[{}]}"#,
        r#"{"format":"vibecut.music-library","version":1,"tracks":[],"extra":true}"#,
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(matches!(
            MusicLibrary::open(root.path()),
            Err(engine::MusicError::LibraryCorrupt)
        ));
    }
}

#[test]
fn ingress_saved_accepts_exact_provenance_and_envelope_bounds() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("library.json");
    let mut music = result(&"x".repeat(128));
    music.title = "á".repeat(2048);
    music.author = "a".repeat(4096);
    music.attribution = "b".repeat(4096);
    music.source_url = format!(
        "https://source.example/{}",
        "s".repeat(8192 - "https://source.example/".len())
    );
    music.audio_url = format!(
        "https://cdn.example/{}",
        "a".repeat(8192 - "https://cdn.example/".len())
    );
    for duration_ms in [1, 86_400_000] {
        let library = MusicLibrary::open(root.path()).unwrap();
        music.duration_ms = duration_ms;
        assert_eq!(library.save(&music).unwrap().music, music);
        drop(library);
        assert_eq!(
            MusicLibrary::open(root.path()).unwrap().saved().unwrap()[0].music,
            music
        );
    }
    for (count, accepted) in [(512, true), (513, false)] {
        let tracks: Vec<_> = (0..count)
            .map(|id| libraries::music::SavedMusic {
                music: result(&format!("saved-{id}")),
                saved_at_unix_ms: 1,
            })
            .collect();
        let bytes = serde_json::to_vec(
            &serde_json::json!({"format": "vibecut.music-library", "version": 1, "tracks": tracks}),
        )
        .unwrap();
        std::fs::write(&path, bytes).unwrap();
        let opened = MusicLibrary::open(root.path());
        if accepted {
            let library = opened.unwrap();
            assert_eq!(library.saved().unwrap().len(), count);
            assert!(matches!(
                library.save(&result("overflow")),
                Err(engine::MusicError::Limit)
            ));
        } else {
            assert!(matches!(opened, Err(engine::MusicError::LibraryCorrupt)));
        }
    }
    let mut bytes = br#"{"format":"vibecut.music-library","version":1,"tracks":[]}"#.to_vec();
    bytes.resize(1 << 20, b' ');
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        MusicLibrary::open(root.path())
            .unwrap()
            .saved()
            .unwrap()
            .is_empty()
    );
    bytes.push(b' ');
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        MusicLibrary::open(root.path()),
        Err(engine::MusicError::LibraryCorrupt)
    ));
}

#[test]
fn ingress_failed_saved_publication_preserves_bytes_memory_and_cache() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("library.json");
    let mut large = result("large");
    large.title = "t".repeat(4096);
    large.author = "a".repeat(4096);
    large.attribution = "c".repeat(4096);
    large.source_url = format!("https://source.example/{}", "s".repeat(8000));
    large.audio_url = format!("https://cdn.example/{}", "a".repeat(8000));
    let mut entries = Vec::new();
    let bytes = loop {
        large.id = format!("large-{}", entries.len());
        entries.push(libraries::music::SavedMusic {
            music: large.clone(),
            saved_at_unix_ms: 1,
        });
        let bytes = serde_json::to_vec(&serde_json::json!({"format": "vibecut.music-library", "version": 1, "tracks": entries})).unwrap();
        if bytes.len() > 1 << 20 {
            entries.pop();
            break serde_json::to_vec(&serde_json::json!({"format": "vibecut.music-library", "version": 1, "tracks": entries})).unwrap();
        }
    };
    std::fs::write(&path, &bytes).unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let cached = library
        .store_cached(&entries[0].music.audio_url, &wav())
        .unwrap();
    let before = library.saved().unwrap();
    large.id = "over-envelope".into();
    assert!(matches!(
        library.save(&large),
        Err(engine::MusicError::Limit)
    ));
    assert_eq!(library.saved().unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        ContentHash::read(&cached.path).unwrap(),
        cached.content_hash
    );
}

#[cfg(windows)]
#[test]
fn ingress_locked_saved_publication_preserves_bytes_memory_and_cache() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("locked");
    library.save(&music).unwrap();
    let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
    let path = root.path().join("library.json");
    let before = std::fs::read(&path).unwrap();
    let saved = library.saved().unwrap();
    // FILE_SHARE_READ permits inspection but denies replacement while held.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let changed = MusicResult {
        audio_url: "https://cdn.example/changed.mp3".into(),
        ..music
    };
    assert!(matches!(
        library.save(&changed),
        Err(engine::MusicError::Project(project::Error::Io(_)))
    ));
    assert_eq!(library.saved().unwrap(), saved);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        ContentHash::read(&cached.path).unwrap(),
        cached.content_hash
    );
    drop(held);
    assert_eq!(library.save(&changed).unwrap().music, changed);
}

#[cfg(any(unix, windows))]
fn music_cache_alias(target: &std::path::Path, link: &std::path::Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {output:?}"
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn owned_audio_alias_dangling_and_file_refuse_without_touching_catalog_or_target() {
    for kind in ["alias", "dangling", "file"] {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("sentinel"), b"unrelated").unwrap();
        std::fs::write(target.path().join(".control.lock"), b"control").unwrap();
        let library = MusicLibrary::open(root.path()).unwrap();
        library.save(&result("saved")).unwrap();
        drop(library);
        let before = std::fs::read(root.path().join("library.json")).unwrap();
        let audio = root.path().join("audio");
        std::fs::remove_dir(&audio).unwrap();
        match kind {
            "file" => std::fs::write(&audio, b"not a directory").unwrap(),
            "dangling" => music_cache_alias(&target.path().join("absent"), &audio),
            _ => music_cache_alias(target.path(), &audio),
        }
        assert!(
            matches!(
                MusicLibrary::open(root.path()),
                Err(engine::MusicError::Project(
                    project::Error::Conflict | project::Error::Unsupported(_)
                ))
            ),
            "{kind}"
        );
        assert_eq!(
            std::fs::read(root.path().join("library.json")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(target.path().join("sentinel")).unwrap(),
            b"unrelated"
        );
        assert_eq!(
            std::fs::read(target.path().join(".control.lock")).unwrap(),
            b"control"
        );
        assert!(!target.path().join("absent").exists());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn owned_clear_removes_only_canonical_artifacts_and_preserves_unknown_entries() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("whitelist");
    library.save(&music).unwrap();
    let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
    let audio = cached.path.parent().unwrap();
    let orphan = audio.join(format!("{}.audio", "c".repeat(64)));
    std::fs::write(&orphan, b"owned orphan").unwrap();
    let names = [
        "notes".to_owned(),
        ".control.lock".into(),
        ".part-keep".into(),
        "short.audio".into(),
        format!("{}.audio", "D".repeat(64)),
        format!("{}.audio", "b".repeat(63)),
    ];
    for name in &names {
        std::fs::write(audio.join(name), b"keep").unwrap();
    }
    let directory = audio.join(format!("{}.audio", "e".repeat(64)));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(target.path().join("sentinel"), b"target").unwrap();
    let link = audio.join(format!("{}.audio", "f".repeat(64)));
    music_cache_alias(target.path(), &link);
    assert_eq!(library.clear_all().unwrap(), 1);
    assert!(!cached.path.exists());
    assert!(!orphan.exists());
    for name in &names {
        assert_eq!(std::fs::read(audio.join(name)).unwrap(), b"keep", "{name}");
    }
    assert!(directory.is_dir());
    assert!(std::fs::symlink_metadata(link).is_ok());
    assert_eq!(
        std::fs::read(target.path().join("sentinel")).unwrap(),
        b"target"
    );
    assert!(library.saved().unwrap().is_empty());
}

#[test]
fn owned_clear_entry_limit_refuses_before_catalog_or_cache_mutation_and_recovers() {
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("bounded-clear");
    library.save(&music).unwrap();
    let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
    let audio = cached.path.parent().unwrap();
    for index in 0..10_000 {
        std::fs::write(audio.join(format!("note-{index}")), b"keep").unwrap();
    }
    let before = std::fs::read(root.path().join("library.json")).unwrap();
    let saved = library.saved().unwrap();
    assert!(matches!(
        library.clear_all(),
        Err(engine::MusicError::Limit)
    ));
    assert_eq!(library.saved().unwrap(), saved);
    assert_eq!(
        std::fs::read(root.path().join("library.json")).unwrap(),
        before
    );
    assert_eq!(
        ContentHash::read(&cached.path).unwrap(),
        cached.content_hash
    );
    std::fs::remove_file(audio.join("note-0")).unwrap();
    assert_eq!(library.clear_all().unwrap(), 1); // Exactly 10,000 entries now.
    assert_eq!(std::fs::read_dir(audio).unwrap().count(), 9_999);
    assert!(library.saved().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn owned_cleanup_failure_reports_already_committed_catalog_and_releases_operation() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("locked-cache");
    library.save(&music).unwrap();
    let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&cached.path)
        .unwrap();
    assert!(matches!(
        library.clear_all(),
        Err(engine::MusicError::Io(_))
    ));
    assert!(library.saved().unwrap().is_empty());
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("library.json")).unwrap()).unwrap();
    assert_eq!(stored["tracks"], serde_json::json!([]));
    assert_eq!(
        ContentHash::read(&cached.path).unwrap(),
        cached.content_hash
    );
    drop(held);
    assert_eq!(library.clear_all().unwrap(), 0);
    assert!(!cached.path.exists());
    library.save(&music).unwrap();
}

#[test]
fn owned_transfer_refuses_reentrant_mutations_without_locking_saved_and_releases_token() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
    let music = result("publication");
    library.save(&music).unwrap();
    let saved = library.saved().unwrap();
    let entered = AtomicBool::new(false);
    let downloads = MusicDownloads::new(
        library.clone(),
        Arc::new(BytesDownload {
            bytes: wav(),
            status: 200,
            headers: Vec::new(),
        }),
    );
    let cancelled = || {
        assert!(matches!(
            library.save(&music),
            Err(engine::MusicError::Project(project::Error::Busy))
        ));
        entered.store(true, Ordering::Release);
        assert_eq!(library.saved().unwrap(), saved); // No state mutex across IO.
        for refused in [
            library.unsave(&music.id).map(|_| ()),
            library.clear_all().map(|_| ()),
            library.store_cached(&music.audio_url, &wav()).map(|_| ()),
            library.cached(&music.audio_url).map(|_| ()),
        ] {
            assert!(matches!(
                refused,
                Err(engine::MusicError::Project(project::Error::Busy))
            ));
        }
        true
    };
    assert!(matches!(
        downloads.download(&music.audio_url, &cancelled, &|_, _| {}),
        Err(engine::MusicError::Cancelled)
    ));
    assert!(entered.load(Ordering::Acquire));
    assert_eq!(library.saved().unwrap(), saved);
    assert!(library.cached(&music.audio_url).unwrap().is_none());
    assert!(
        library
            .store_cached(&music.audio_url, b"not audio")
            .is_err()
    );
    library.store_cached(&music.audio_url, &wav()).unwrap();
    library.save(&music).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn owned_root_alias_is_chosen_once_and_held_directories_bind_local_operations() {
    let fixture = tempfile::tempdir().unwrap();
    let real = fixture.path().join("real root");
    let other = fixture.path().join("other root");
    std::fs::create_dir(&real).unwrap();
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("sentinel"), b"other").unwrap();
    let alias = fixture.path().join("root alias");
    music_cache_alias(&real, &alias);
    let library = MusicLibrary::open(&alias).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(&alias).unwrap();
    #[cfg(unix)]
    std::fs::remove_file(&alias).unwrap();
    music_cache_alias(&other, &alias);
    let music = result("root-binding");
    library.save(&music).unwrap();
    library.store_cached(&music.audio_url, &wav()).unwrap();
    assert_eq!(library.root(), std::fs::canonicalize(&real).unwrap());
    assert_eq!(library.clear_all().unwrap(), 1);
    assert_eq!(std::fs::read(other.join("sentinel")).unwrap(), b"other");
    drop(library);
    for parent in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("library");
        let library = MusicLibrary::open(&path).unwrap();
        library.save(&music).unwrap();
        let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
        let saved = library.saved().unwrap();
        let before = std::fs::read(path.join("library.json")).unwrap();
        let named = if parent {
            path.clone()
        } else {
            path.join("audio")
        };
        let moved = root.path().join("moved");
        let attempt = std::fs::rename(&named, &moved);
        #[cfg(windows)]
        {
            assert!(matches!(attempt.unwrap_err().raw_os_error(), Some(5 | 32)));
            assert_eq!(library.saved().unwrap(), saved);
            assert_eq!(std::fs::read(path.join("library.json")).unwrap(), before);
            assert_eq!(
                ContentHash::read(&cached.path).unwrap(),
                cached.content_hash
            );
            assert_eq!(library.clear_all().unwrap(), 1);
            drop(library);
            std::fs::rename(&named, &moved).unwrap();
        }
        #[cfg(unix)]
        {
            attempt.unwrap();
            std::fs::create_dir(&named).unwrap();
            assert!(matches!(
                library.clear_all(),
                Err(engine::MusicError::Project(project::Error::Conflict))
            ));
            assert!(matches!(
                library.save(&music),
                Err(engine::MusicError::Project(project::Error::Conflict))
            ));
            assert_eq!(library.saved().unwrap(), saved);
            let catalog = if parent {
                moved.join("library.json")
            } else {
                path.join("library.json")
            };
            assert_eq!(std::fs::read(catalog).unwrap(), before);
            drop(library);
        }
    }
}

#[cfg(windows)]
#[test]
fn owned_catalog_publication_failure_preserves_saved_cache_and_project_media() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let media = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("catalog-failure");
    library.save(&music).unwrap();
    let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(1), "music".into()).unwrap(), 8).unwrap();
    add_saved(
        &mut session,
        &library,
        &music.id,
        media.path(),
        Placement::Auto,
        Tick::ZERO,
    )
    .unwrap();
    let project = project::encode(session.project(), false).unwrap();
    let owned = session.project().data().assets[0].path().to_owned();
    let path = root.path().join("library.json");
    let bytes = std::fs::read(&path).unwrap();
    let saved = library.saved().unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    for action in ["insert", "url change", "unsave", "clear"] {
        let mut changed = music.clone();
        changed.audio_url.push_str("?changed=1");
        let failed = match action {
            "insert" => {
                changed.id = "new".into();
                library.save(&changed).map(|_| ())
            }
            "url change" => library.save(&changed).map(|_| ()),
            "unsave" => library.unsave(&music.id).map(|_| ()),
            _ => library.clear_all().map(|_| ()),
        };
        assert!(
            matches!(
                failed,
                Err(engine::MusicError::Project(project::Error::Io(_)))
            ),
            "{action}"
        );
        assert_eq!(library.saved().unwrap(), saved);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            library
                .cached(&music.audio_url)
                .unwrap()
                .unwrap()
                .content_hash,
            cached.content_hash
        );
        assert_eq!(ContentHash::read(&owned).unwrap(), cached.content_hash);
        assert_eq!(project::encode(session.project(), false).unwrap(), project);
    }
    drop(held);
    library.unsave(&music.id).unwrap();
    assert_eq!(ContentHash::read(&owned).unwrap(), cached.content_hash);
}

#[cfg(any(unix, windows))]
#[test]
fn owned_cache_alias_refuses_reads_and_writes_and_survives_unreferenced_cleanup() {
    for change in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let victim = target.path().join("sentinel");
        std::fs::write(&victim, b"unrelated").unwrap();
        let library = MusicLibrary::open(root.path()).unwrap();
        let music = result("file-alias");
        library.save(&music).unwrap();
        let path = library.cache_path(&music.audio_url);
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&victim, &path).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        let before = std::fs::read(root.path().join("library.json")).unwrap();
        let saved = library.saved().unwrap();
        assert!(matches!(
            library.cached(&music.audio_url),
            Err(engine::MusicError::LibraryCorrupt)
        ));
        assert!(matches!(
            library.store_cached(&music.audio_url, &wav()),
            Err(engine::MusicError::LibraryCorrupt)
        ));
        assert_eq!(library.saved().unwrap(), saved);
        assert_eq!(
            std::fs::read(root.path().join("library.json")).unwrap(),
            before
        );
        if change {
            library
                .save(&MusicResult {
                    audio_url: "https://cdn.example/new.mp3".into(),
                    ..music
                })
                .unwrap();
        } else {
            library.unsave(&music.id).unwrap();
        }
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&victim).unwrap(), b"unrelated");
        library.clear_all().unwrap();
        assert!(std::fs::symlink_metadata(path).is_ok());
        assert_eq!(std::fs::read(victim).unwrap(), b"unrelated");
    }
}

fn music_runner(clock: Arc<dyn Clock>) -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 4,
            network_attempts: 2,
            backoff: std::time::Duration::from_millis(1),
            progress_interval: std::time::Duration::from_millis(10),
        },
        engine::ByteBudget::new(8 << 20).unwrap(),
        clock,
    )
    .unwrap()
}
fn music_job_until(runner: &mut JobRunner, id: JobId, wanted: JobState) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(id).unwrap().state;
        if state == wanted {
            return;
        }
        assert!(
            !matches!(
                state,
                JobState::Completed | JobState::Cancelled | JobState::Failed
            ),
            "unexpected {state:?}, wanted {wanted:?}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "job never reached {wanted:?}"
        );
        std::thread::yield_now();
    }
}
struct TransferRelease(std::sync::mpsc::Sender<()>);
impl Drop for TransferRelease {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
struct TransferRead {
    bytes: Cursor<Vec<u8>>,
    gate: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
}
impl Read for TransferRead {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if let Some((entered, release)) = self.gate.take() {
            entered.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }
        self.bytes.read(out)
    }
}
struct TransferDownload(Mutex<Option<TransferRead>>);
impl DownloadTransport for TransferDownload {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, transport::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: Vec::new(),
            body: Box::new(self.0.lock().unwrap().take().unwrap()),
        })
    }
}

#[test]
fn transfer_shared_admission_preserves_catalog_cache_and_project_while_read_is_gated() {
    for submitted in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
        let music = result("retained");
        library.save(&music).unwrap();
        let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
        let mut session =
            Session::new(Project::empty(ProjectId(1), "music".into()).unwrap(), 8).unwrap();
        add_saved(
            &mut session,
            &library,
            &music.id,
            media.path(),
            Placement::Auto,
            Tick::ZERO,
        )
        .unwrap();
        let owned = session.project().data().assets[0].path().to_owned();
        let project = project::encode(session.project(), false).unwrap();
        let saved = library.saved().unwrap();
        let bytes = std::fs::read(root.path().join("library.json")).unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = MusicDownloads::new(
            library.clone(),
            Arc::new(TransferDownload(Mutex::new(Some(TransferRead {
                bytes: Cursor::new(wav()),
                gate: Some((entered_tx, release_rx)),
            })))),
        );
        let transport = Arc::new(Download {
            status: 500,
            headers: Vec::new(),
            calls: AtomicUsize::new(0),
        });
        let second = MusicDownloads::new(library.clone(), transport.clone());
        std::thread::scope(|scope| {
            let mut runner = music_runner(Arc::new(engine::jobs::SystemClock::default()));
            let release = TransferRelease(release_tx);
            let id = submitted.then(|| {
                first
                    .submit(&mut runner, "https://cdn.example/transfer.mp3")
                    .unwrap()
                    .0
            });
            let direct = if submitted {
                runner.tick().unwrap();
                None
            } else {
                Some(scope.spawn(|| {
                    first.download("https://cdn.example/transfer.mp3", &|| false, &|_, _| {})
                }))
            };
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            for refused in [
                library.clear_all().map(|_| ()),
                library.unsave(&music.id).map(|_| ()),
                library.save(&music).map(|_| ()),
                library.store_cached(&music.audio_url, &wav()).map(|_| ()),
                library.cached(&music.audio_url).map(|_| ()),
                second
                    .download("https://cdn.example/contender.mp3", &|| false, &|_, _| {})
                    .map(|_| ()),
            ] {
                assert!(matches!(
                    refused,
                    Err(engine::MusicError::Project(project::Error::Busy))
                ));
            }
            assert!(matches!(
                second.submit(&mut runner, "https://cdn.example/contender.mp3"),
                Err(engine::control::Error::Runtime(
                    engine::RuntimeError::CacheBusy
                ))
            ));
            assert_eq!(transport.calls.load(Ordering::Acquire), 0);
            assert_eq!(library.saved().unwrap(), saved);
            assert_eq!(
                library
                    .search_local(&SearchRequest::default())
                    .unwrap()
                    .count,
                1
            );
            assert_eq!(
                std::fs::read(root.path().join("library.json")).unwrap(),
                bytes
            );
            assert_eq!(
                ContentHash::read(&cached.path).unwrap(),
                cached.content_hash
            );
            assert_eq!(ContentHash::read(&owned).unwrap(), cached.content_hash);
            assert_eq!(project::encode(session.project(), false).unwrap(), project);
            drop(release);
            if let Some(job) = direct {
                job.join().unwrap().unwrap();
            }
            if let Some(id) = id {
                music_job_until(&mut runner, id, JobState::Completed);
            }
            library.save(&music).unwrap();
            assert_eq!(
                library
                    .cached("https://cdn.example/transfer.mp3")
                    .unwrap()
                    .unwrap()
                    .bytes()
                    .unwrap(),
                wav()
            );
        });
    }
}

#[test]
fn transfer_queue_cancel_failed_submission_and_runner_drop_release_admission() {
    let root = tempfile::tempdir().unwrap();
    let other_root = tempfile::tempdir().unwrap();
    let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
    let other = Arc::new(MusicLibrary::open(other_root.path()).unwrap());
    let transport = Arc::new(Download {
        status: 500,
        headers: Vec::new(),
        calls: AtomicUsize::new(0),
    });
    let first = MusicDownloads::new(library.clone(), transport.clone());
    let blocker = MusicDownloads::new(other.clone(), transport.clone());
    let mut runner = music_runner(Arc::new(engine::jobs::SystemClock::default()));
    let (blocked, cancel) = blocker
        .submit(&mut runner, "https://cdn.example/block.mp3")
        .unwrap();
    assert!(matches!(
        first.submit(&mut runner, "https://cdn.example/full.mp3"),
        Err(engine::control::Error::Runtime(
            engine::RuntimeError::QueueFull
        ))
    ));
    library.save(&result("released")).unwrap();
    cancel.cancel();
    runner.tick().unwrap();
    assert_eq!(runner.snapshot(blocked).unwrap().state, JobState::Cancelled);
    other.save(&result("released")).unwrap();
    let (id, cancel) = first
        .submit(&mut runner, "https://cdn.example/cancel.mp3")
        .unwrap();
    assert!(matches!(
        library.clear_all(),
        Err(engine::MusicError::Project(project::Error::Busy))
    ));
    cancel.cancel();
    runner.tick().unwrap();
    assert_eq!(runner.snapshot(id).unwrap().state, JobState::Cancelled);
    library.save(&result("released")).unwrap();
    first
        .submit(&mut runner, "https://cdn.example/drop.mp3")
        .unwrap();
    drop(runner);
    library.save(&result("released")).unwrap();
    assert_eq!(transport.calls.load(Ordering::Acquire), 0);
}

#[test]
fn transfer_part_collision_preserves_preexisting_object_in_a_fresh_process() {
    if let Ok(kind) = std::env::var("VIBECUT_MUSIC_COLLISION") {
        let root = tempfile::tempdir().unwrap();
        let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
        let music = result("collision");
        library.save(&music).unwrap();
        let path = root
            .path()
            .join("audio")
            .join(format!(".part-{}-0", std::process::id()));
        let sentinel = if kind == "directory" {
            std::fs::create_dir(&path).unwrap();
            path.join("sentinel")
        } else {
            path.clone()
        };
        std::fs::write(&sentinel, b"preexisting exact bytes").unwrap();
        let before = std::fs::read(root.path().join("library.json")).unwrap();
        let downloads = MusicDownloads::new(
            library.clone(),
            Arc::new(BytesDownload {
                bytes: wav(),
                status: 200,
                headers: Vec::new(),
            }),
        );
        assert!(matches!(
            downloads.download(&music.audio_url, &|| false, &|_, _| {}),
            Err(engine::MusicError::Io(_))
        ));
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"preexisting exact bytes"
        );
        assert_eq!(path.is_dir(), kind == "directory");
        assert_eq!(
            std::fs::read(root.path().join("library.json")).unwrap(),
            before
        );
        assert!(!library.cache_path(&music.audio_url).exists());
        downloads
            .download(&music.audio_url, &|| false, &|_, _| {})
            .unwrap();
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preexisting exact bytes");
        return;
    }
    for kind in ["file", "directory"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "music::transfer_part_collision_preserves_preexisting_object_in_a_fresh_process",
                "--nocapture",
            ])
            .env("VIBECUT_MUSIC_COLLISION", kind)
            .output()
            .unwrap();
        assert!(output.status.success(), "{kind}: {output:?}");
    }
}

struct TransferClock(std::sync::atomic::AtomicU64);
impl Clock for TransferClock {
    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.0.load(Ordering::Acquire))
    }
}
struct RetryDownload {
    calls: AtomicUsize,
    failures: usize,
}
impl DownloadTransport for RetryDownload {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, transport::Error> {
        let failed = self.calls.fetch_add(1, Ordering::AcqRel) < self.failures;
        Ok(HttpStreamResponse {
            status: if failed { 503 } else { 200 },
            headers: Vec::new(),
            body: Box::new(Cursor::new(wav())),
        })
    }
}
#[test]
fn transfer_retry_retains_admission_through_backoff_and_terminal_failure_or_success() {
    for failures in [1, 2] {
        let root = tempfile::tempdir().unwrap();
        let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
        let transport = Arc::new(RetryDownload {
            calls: AtomicUsize::new(0),
            failures,
        });
        let downloads = MusicDownloads::new(library.clone(), transport.clone());
        let clock = Arc::new(TransferClock(std::sync::atomic::AtomicU64::new(0)));
        let mut runner = music_runner(clock.clone());
        let (id, _) = downloads
            .submit(&mut runner, "https://cdn.example/retry.mp3")
            .unwrap();
        music_job_until(&mut runner, id, JobState::Backoff);
        assert!(matches!(
            library.clear_all(),
            Err(engine::MusicError::Project(project::Error::Busy))
        ));
        assert_eq!(transport.calls.load(Ordering::Acquire), 1);
        clock.0.store(1, Ordering::Release);
        music_job_until(
            &mut runner,
            id,
            if failures == 1 {
                JobState::Completed
            } else {
                JobState::Failed
            },
        );
        assert_eq!(runner.snapshot(id).unwrap().attempts, 2);
        assert_eq!(transport.calls.load(Ordering::Acquire), 2);
        assert_eq!(
            library
                .cached("https://cdn.example/retry.mp3")
                .unwrap()
                .is_some(),
            failures == 1
        );
        library.save(&result("released")).unwrap();
    }
}

struct FenceGate {
    token: Mutex<Option<engine::jobs::CancelToken>>,
    worker: Mutex<Option<std::thread::ThreadId>>,
    fired: AtomicBool,
    before: bool,
    effect: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl FenceGate {
    fn fire(&self) {
        if let Some(effect) = self.effect.lock().unwrap().take() {
            effect();
        } else {
            self.token.lock().unwrap().as_ref().unwrap().cancel();
        }
        self.fired.store(true, Ordering::Release);
    }
}
impl Clock for FenceGate {
    fn now(&self) -> std::time::Duration {
        if *self.worker.lock().unwrap() == Some(std::thread::current().id())
            && !self.fired.load(Ordering::Acquire)
        {
            self.fire();
        }
        std::time::Duration::ZERO
    }
}
struct FenceRead(Cursor<Vec<u8>>, Arc<FenceGate>);
impl Read for FenceRead {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.0.read(out)?;
        if n == 0 {
            if self.1.before {
                self.1.fire();
            } else {
                *self.1.worker.lock().unwrap() = Some(std::thread::current().id());
            }
        }
        Ok(n)
    }
}
struct FenceDownload(Arc<FenceGate>);
impl DownloadTransport for FenceDownload {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, transport::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: Vec::new(),
            body: Box::new(FenceRead(Cursor::new(wav()), self.0.clone())),
        })
    }
}
#[test]
fn transfer_real_fence_cancellation_and_typed_callback_failure_have_exact_outcomes() {
    for mode in ["before", "after", "conflict"]
        .into_iter()
        .chain(cfg!(windows).then_some("io"))
    {
        let root = tempfile::tempdir().unwrap();
        let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
        let music = result("fenced");
        library.save(&music).unwrap();
        let held = Arc::new(Mutex::new(None::<std::fs::File>));
        let effect: Option<Box<dyn FnOnce() + Send>> = match mode {
            "conflict" => {
                let path = root.path().join("music.vibecut");
                Some(Box::new(move || {
                    std::fs::write(path, b"foreign lease content").unwrap()
                }))
            }
            #[cfg(windows)]
            "io" => {
                use std::os::windows::fs::OpenOptionsExt;
                let path = library.cache_path(&music.audio_url);
                let held = held.clone();
                Some(Box::new(move || {
                    std::fs::write(&path, wav()).unwrap();
                    *held.lock().unwrap() = Some(
                        std::fs::OpenOptions::new()
                            .read(true)
                            .share_mode(3)
                            .open(path)
                            .unwrap(),
                    );
                }))
            }
            _ => None,
        };
        let gate = Arc::new(FenceGate {
            token: Mutex::new(None),
            worker: Mutex::new(None),
            fired: AtomicBool::new(false),
            before: mode == "before",
            effect: Mutex::new(effect),
        });
        let downloads = MusicDownloads::new(library.clone(), Arc::new(FenceDownload(gate.clone())));
        let mut runner = music_runner(gate.clone());
        let (id, cancel) = downloads.submit(&mut runner, &music.audio_url).unwrap();
        *gate.token.lock().unwrap() = Some(cancel);
        music_job_until(
            &mut runner,
            id,
            match mode {
                "before" => JobState::Cancelled,
                "after" => JobState::Completed,
                _ => JobState::Failed,
            },
        );
        assert!(gate.fired.load(Ordering::Acquire));
        assert_eq!(runner.output_bytes(id).is_some(), mode == "after");
        assert_eq!(
            library.cache_path(&music.audio_url).exists(),
            matches!(mode, "after" | "io")
        );
        if mode == "conflict" {
            assert!(matches!(
                (runner.snapshot(id).unwrap().error.unwrap().as_ref()
                    as &(dyn std::error::Error + 'static))
                    .downcast_ref::<engine::control::Error>(),
                Some(engine::control::Error::Music(engine::MusicError::Project(
                    project::Error::Conflict
                )))
            ));
            assert_eq!(
                std::fs::read(root.path().join("music.vibecut")).unwrap(),
                b"foreign lease content"
            );
            std::fs::remove_file(root.path().join("music.vibecut")).unwrap();
        } else if mode == "io" {
            assert!(matches!(
                (runner.snapshot(id).unwrap().error.unwrap().as_ref()
                    as &(dyn std::error::Error + 'static))
                    .downcast_ref::<engine::control::Error>(),
                Some(engine::control::Error::Music(engine::MusicError::Io(_)))
            ));
            assert_eq!(
                std::fs::read(library.cache_path(&music.audio_url)).unwrap(),
                wav()
            );
            held.lock().unwrap().take();
        } else {
            assert!(runner.snapshot(id).unwrap().error.is_none() || mode == "before");
            if mode == "after" {
                assert_eq!(
                    library
                        .cached(&music.audio_url)
                        .unwrap()
                        .unwrap()
                        .bytes()
                        .unwrap(),
                    wav()
                );
            }
            assert!(
                std::fs::read_dir(root.path().join("audio"))
                    .unwrap()
                    .all(|entry| !entry
                        .unwrap()
                        .file_name()
                        .as_encoded_bytes()
                        .starts_with(b".part-"))
            );
        }
        library.save(&music).unwrap();
    }
}

struct FailedRead(bool);
impl Read for FailedRead {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        assert!(!self.0, "synthetic reader panic");
        Err(std::io::Error::other("synthetic reader error"))
    }
}
struct FailedDownload(bool);
impl DownloadTransport for FailedDownload {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, transport::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: Vec::new(),
            body: Box::new(FailedRead(self.0)),
        })
    }
}
#[test]
fn transfer_owned_read_failure_and_panic_clean_parts_without_releasing_foreign_parts() {
    for (panic, submitted) in [(false, false), (true, false), (true, true)] {
        let root = tempfile::tempdir().unwrap();
        let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
        let music = result("failed");
        library.save(&music).unwrap();
        let audio = root.path().join("audio");
        std::fs::write(audio.join(".part-unrelated"), b"keep").unwrap();
        std::fs::write(audio.join(".control.lock"), b"control").unwrap();
        let before = std::fs::read(root.path().join("library.json")).unwrap();
        let downloads = MusicDownloads::new(library.clone(), Arc::new(FailedDownload(panic)));
        if submitted {
            let mut runner = music_runner(Arc::new(engine::jobs::SystemClock::default()));
            let (id, _) = downloads.submit(&mut runner, &music.audio_url).unwrap();
            music_job_until(&mut runner, id, JobState::Failed);
            assert!(matches!(
                (runner.snapshot(id).unwrap().error.unwrap().as_ref()
                    as &(dyn std::error::Error + 'static))
                    .downcast_ref::<engine::RuntimeError>(),
                Some(engine::RuntimeError::WorkerPanicked)
            ));
        } else if panic {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| downloads.download(
                    &music.audio_url,
                    &|| false,
                    &|_, _| {}
                )))
                .is_err()
            );
        } else {
            assert!(matches!(
                downloads.download(&music.audio_url, &|| false, &|_, _| {}),
                Err(engine::MusicError::Network)
            ));
        }
        assert_eq!(std::fs::read_dir(&audio).unwrap().count(), 2);
        assert_eq!(
            std::fs::read(audio.join(".part-unrelated")).unwrap(),
            b"keep"
        );
        assert_eq!(
            std::fs::read(audio.join(".control.lock")).unwrap(),
            b"control"
        );
        assert_eq!(
            std::fs::read(root.path().join("library.json")).unwrap(),
            before
        );
        library.save(&music).unwrap();
    }
}

#[cfg(windows)]
struct LockedPartRead {
    audio: std::path::PathBuf,
    held: Arc<Mutex<Option<std::fs::File>>>,
}
#[cfg(windows)]
impl Read for LockedPartRead {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        use std::os::windows::fs::OpenOptionsExt;
        let path = std::fs::read_dir(&self.audio)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .as_encoded_bytes()
                    .starts_with(format!(".part-{}-", std::process::id()).as_bytes())
            })
            .unwrap();
        *self.held.lock().unwrap() = Some(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(path)
                .unwrap(),
        );
        Err(std::io::Error::other("synthetic reader failure"))
    }
}
#[cfg(windows)]
struct LockedPartDownload {
    audio: std::path::PathBuf,
    held: Arc<Mutex<Option<std::fs::File>>>,
}
#[cfg(windows)]
impl DownloadTransport for LockedPartDownload {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, transport::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: Vec::new(),
            body: Box::new(LockedPartRead {
                audio: self.audio.clone(),
                held: self.held.clone(),
            }),
        })
    }
}
#[cfg(windows)]
#[test]
fn transfer_owned_cleanup_failure_is_typed_and_releases_admission() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
    let held = Arc::new(Mutex::new(None));
    let downloads = MusicDownloads::new(
        library.clone(),
        Arc::new(LockedPartDownload {
            audio: root.path().join("audio"),
            held: held.clone(),
        }),
    );
    let error = downloads
        .download("https://cdn.example/locked-part.mp3", &|| false, &|_, _| {})
        .unwrap_err();
    assert!(
        matches!(error, engine::MusicError::Io(ref cause) if matches!(cause.raw_os_error(), Some(5 | 32)))
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("audio"))
            .unwrap()
            .count(),
        1
    );
    held.lock().unwrap().take();
    library.save(&result("released")).unwrap();
    let remaining = std::fs::read_dir(root.path().join("audio"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::remove_file(remaining).unwrap();
}

struct Download {
    status: u16,
    headers: Vec<(String, String)>,
    calls: AtomicUsize,
}
impl DownloadTransport for Download {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, transport::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(HttpStreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::new(Cursor::new(Vec::new())),
        })
    }
}
struct BytesDownload {
    bytes: Vec<u8>,
    status: u16,
    headers: Vec<(String, String)>,
}
impl DownloadTransport for BytesDownload {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, transport::Error> {
        Ok(HttpStreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::new(Cursor::new(self.bytes.clone())),
        })
    }
}
struct CancelReader {
    bytes: Vec<u8>,
    cancel: Arc<AtomicBool>,
}
impl Read for CancelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if !self.bytes.is_empty() {
            self.cancel.store(true, Ordering::Release);
        }
        let n = self.bytes.len().min(out.len());
        let source = self
            .bytes
            .get(..n)
            .ok_or_else(|| std::io::Error::other("read"))?;
        let target = out
            .get_mut(..n)
            .ok_or_else(|| std::io::Error::other("read"))?;
        target.copy_from_slice(source);
        self.bytes.drain(..n);
        Ok(n)
    }
}
struct CancellingDownload {
    bytes: Vec<u8>,
    cancel: Arc<AtomicBool>,
}
impl DownloadTransport for CancellingDownload {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, transport::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: vec![("content-length".into(), self.bytes.len().to_string())],
            body: Box::new(CancelReader {
                bytes: self.bytes.clone(),
                cancel: self.cancel.clone(),
            }),
        })
    }
}

fn wav() -> Vec<u8> {
    let samples = 2000u32;
    let data_len = samples * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8000u32.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.extend(std::iter::repeat_n(0u8, data_len as usize));
    bytes
}

#[test]
fn download_is_bounded_cancellable_and_publishes_only_complete_audio() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
    let good = BytesDownload {
        bytes: wav(),
        status: 200,
        headers: vec![("content-length".into(), wav().len().to_string())],
    };
    let downloads = MusicDownloads::new(library.clone(), Arc::new(good));
    let cached = downloads
        .download("https://cdn.example/good.mp3", &|| false, &|_, _| {})
        .unwrap();
    assert!(cached.duration().unwrap() > Tick::ZERO);
    assert!(cached.path.exists());

    let too_large = BytesDownload {
        bytes: wav(),
        status: 200,
        headers: vec![("content-length".into(), (33 << 20).to_string())],
    };
    let downloads = MusicDownloads::new(library.clone(), Arc::new(too_large));
    assert!(matches!(
        downloads.download("https://cdn.example/big.mp3", &|| false, &|_, _| {}),
        Err(engine::MusicError::Limit)
    ));

    let cancel = Arc::new(AtomicBool::new(false));
    let cancelling = CancellingDownload {
        bytes: wav(),
        cancel: cancel.clone(),
    };
    let downloads = MusicDownloads::new(library.clone(), Arc::new(cancelling));
    assert!(matches!(
        downloads.download(
            "https://cdn.example/cancel.mp3",
            &|| cancel.load(Ordering::Acquire),
            &|_, _| {}
        ),
        Err(engine::MusicError::Cancelled)
    ));
    assert!(
        !library
            .cache_path("https://cdn.example/cancel.mp3")
            .exists()
    );

    let missing = Download {
        status: 404,
        headers: Vec::new(),
        calls: AtomicUsize::new(0),
    };
    let downloads = MusicDownloads::new(library, Arc::new(missing));
    assert!(matches!(
        downloads.download("https://cdn.example/missing.mp3", &|| false, &|_, _| {}),
        Err(engine::MusicError::InvalidResponse)
    ));
}

struct Sink {
    plays: AtomicUsize,
    stops: AtomicUsize,
    device: bool,
}
impl PreviewSink for Sink {
    fn play(&self, clip: PreviewClip<'_>) -> libraries::sounds::Sound<()> {
        if !self.device {
            return Err(engine::SoundError::NoAudioDevice);
        }
        assert!(!clip.bytes.is_empty());
        self.plays.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn stop(&self) -> libraries::sounds::Sound<()> {
        self.stops.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[test]
fn preview_play_stop_and_missing_device_are_typed() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
    let music = result("preview");
    library.store_cached(&music.audio_url, &wav()).unwrap();
    let downloads = Arc::new(MusicDownloads::new(
        library.clone(),
        Arc::new(BytesDownload {
            bytes: Vec::new(),
            status: 500,
            headers: Vec::new(),
        }),
    ));
    let sink = Arc::new(Sink {
        plays: AtomicUsize::new(0),
        stops: AtomicUsize::new(0),
        device: true,
    });
    let preview = MusicPreview::new(downloads.clone(), sink.clone());
    preview.play(&music, &|| false).unwrap();
    preview.stop().unwrap();
    assert_eq!(sink.plays.load(Ordering::Relaxed), 1);
    assert_eq!(sink.stops.load(Ordering::Relaxed), 1);

    let no_device = Arc::new(Sink {
        plays: AtomicUsize::new(0),
        stops: AtomicUsize::new(0),
        device: false,
    });
    let preview = MusicPreview::new(downloads, no_device);
    assert!(matches!(
        preview.play(&music, &|| false),
        Err(engine::MusicError::NoAudioDevice)
    ));
}

#[test]
fn adding_a_cached_track_is_one_undo_step_and_round_trips_metadata() {
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("21");
    library.save(&music).unwrap();
    let cached = library.store_cached(&music.audio_url, &wav()).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(0), "music".into()).unwrap(), 8).unwrap();
    let media = tempfile::tempdir().unwrap();
    // A media directory that cannot be created refuses before any project change.
    let blocked = media.path().join("file");
    std::fs::write(&blocked, b"x").unwrap();
    assert!(
        add_saved(
            &mut session,
            &library,
            &music.id,
            &blocked,
            Placement::Auto,
            Tick::ZERO
        )
        .is_err()
    );
    assert_eq!(session.history_len(), (0, 0));
    let outcome = add_saved(
        &mut session,
        &library,
        &music.id,
        media.path(),
        Placement::Auto,
        Tick::ZERO,
    )
    .unwrap();
    assert_eq!(outcome.placements.len(), 1);
    assert_eq!(session.history_len(), (1, 0));
    let data = session.project().data();
    assert_eq!(data.assets.len(), 1);
    let owned = data.assets[0].path().to_owned();
    assert_ne!(owned, cached.path);
    assert!(owned.starts_with(media.path()));
    assert_eq!(data.assets[0].content_hash(), cached.content_hash);
    let item = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .next()
        .unwrap();
    let ItemParams::Audio(params) = &item.content else {
        panic!("expected audio item");
    };
    let AudioSource::Music { music: stored } = &params.source else {
        panic!("expected music source");
    };
    let stored = stored.clone();
    assert_eq!(stored.id.as_str(), "21");
    assert_eq!(stored.licence.as_str(), "CC BY 4.0");
    assert_eq!(stored.licence_url, music.licence_url);
    assert_eq!(stored.audio_url, music.audio_url);
    assert_eq!(stored.source_url, music.source_url);

    let bytes = project::encode(session.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap().project;
    let round_trip = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find_map(|item| match &item.content {
            ItemParams::Audio(params) => match &params.source {
                AudioSource::Music { music } => Some(music.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(round_trip, stored);

    session.apply(Command::Undo {}).unwrap();
    assert!(session.project().data().assets.is_empty());
    session.apply(Command::Redo {}).unwrap();
    assert_eq!(session.project().data().assets.len(), 1);

    // Every library cleanup path leaves the referenced project media intact.
    let mut changed = music.clone();
    changed.audio_url = "https://cdn.example/21-v2.mp3".into();
    for step in ["url change", "unsave", "clear all"] {
        match step {
            "url change" => {
                library.save(&changed).unwrap();
            }
            "unsave" => {
                assert!(library.unsave(&music.id).unwrap());
            }
            _ => {
                library.save(&music).unwrap();
                library.clear_all().unwrap();
            }
        }
        assert!(
            library.cached(&music.audio_url).unwrap().is_none(),
            "{step}: cache entry should be gone"
        );
        assert_eq!(
            ContentHash::read(&owned).unwrap(),
            cached.content_hash,
            "{step}: project media changed"
        );
        session.apply(Command::Undo {}).unwrap();
        session.apply(Command::Redo {}).unwrap();
        assert_eq!(session.project().data().assets[0].path(), owned, "{step}");
        let reloaded = project::decode(&project::encode(session.project(), false).unwrap())
            .unwrap()
            .project;
        assert_eq!(reloaded.data().assets[0].path(), owned, "{step}");
    }
}
#[test]
fn adding_an_uncached_track_is_typed_and_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let library = MusicLibrary::open(root.path()).unwrap();
    let music = result("22");
    library.save(&music).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(1), "music".into()).unwrap(), 8).unwrap();
    assert!(matches!(
        add_saved(
            &mut session,
            &library,
            &music.id,
            root.path(),
            Placement::Auto,
            Tick::ZERO
        ),
        Err(engine::control::Error::Music(engine::MusicError::NotCached))
    ));
    assert!(session.project().data().assets.is_empty());
    assert_eq!(session.history_len(), (0, 0));
}
#[test]
fn music_download_jobs_complete_and_cancel_through_the_shared_runner() {
    use engine::ByteBudget;
    use engine::jobs::{JobRunner, JobState, RunnerConfig, SystemClock};
    use std::time::Duration;
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(MusicLibrary::open(root.path()).unwrap());
    let good = BytesDownload {
        bytes: wav(),
        status: 200,
        headers: vec![("content-length".into(), wav().len().to_string())],
    };
    let downloads = MusicDownloads::new(library.clone(), Arc::new(good));
    let config = RunnerConfig {
        concurrency: [1, 1, 1],
        queue: 2,
        retained: 2,
        network_attempts: 2,
        backoff: Duration::from_millis(10),
        progress_interval: Duration::from_millis(10),
    };
    let mut runner = JobRunner::new(
        config,
        ByteBudget::new(8 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    let (id, _) = downloads
        .submit(&mut runner, "https://cdn.example/job.mp3")
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            assert_eq!(state, JobState::Completed);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(
        library
            .cached("https://cdn.example/job.mp3")
            .unwrap()
            .is_some()
    );
    let (id, cancel) = downloads
        .submit(&mut runner, "https://cdn.example/cancelled-job.mp3")
        .unwrap();
    cancel.cancel();
    runner.tick().unwrap();
    assert_eq!(runner.snapshot(id).unwrap().state, JobState::Cancelled);
    assert!(
        !library
            .cache_path("https://cdn.example/cancelled-job.mp3")
            .exists()
    );
    // A second concurrent submit is refused while one download is active.
    let (_, _) = downloads
        .submit(&mut runner, "https://cdn.example/third.mp3")
        .unwrap();
    assert!(matches!(
        downloads.submit(&mut runner, "https://cdn.example/fourth.mp3"),
        Err(engine::control::Error::Runtime(
            engine::RuntimeError::CacheBusy
        ))
    ));
}
