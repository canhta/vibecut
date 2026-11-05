#![allow(clippy::unwrap_used, clippy::panic)]
use ai::transport::{
    DownloadTransport, HttpRequest, HttpResponse, HttpStreamResponse, HttpTransport,
};
use engine::sounds::{
    PreviewClip, PreviewSink, SearchRequest, SoundClient, SoundDownloads, SoundLibrary,
    SoundPreview, SoundResult, Token, add_saved,
};
use engine::{Command, EditError, Placement, Session};
use project::{AudioSource, ItemParams, Project, ProjectId};
use std::{
    io::{Cursor, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use time::Tick;

struct JsonTransport {
    response: HttpResponse,
    requests: Mutex<Vec<HttpRequest>>,
}
impl HttpTransport for JsonTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, ai::Error> {
        self.send_bounded(request, 1 << 20)
    }
    fn send_bounded(
        &self,
        request: HttpRequest,
        max_bytes: usize,
    ) -> Result<HttpResponse, ai::Error> {
        self.requests.lock().unwrap().push(request);
        if self.response.body.len() > max_bytes {
            return Err(ai::Error::Payload);
        }
        Ok(HttpResponse {
            status: self.response.status,
            headers: self.response.headers.clone(),
            body: self.response.body.clone(),
        })
    }
}
fn client(body: serde_json::Value) -> (SoundClient, Arc<JsonTransport>) {
    let transport = Arc::new(JsonTransport {
        response: HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: serde_json::to_vec(&body).unwrap(),
        },
        requests: Mutex::new(Vec::new()),
    });
    (
        SoundClient::new(Token::new("secret-token").unwrap(), transport.clone()),
        transport,
    )
}
fn result(id: u64, licence: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": format!("Sound {id}"),
        "username": format!("user{id}"),
        "license": licence,
        "num_downloads": 100 - id,
        "previews": {"preview-hq-mp3": format!("https://cdn.example/{id}.mp3")},
        "duration": 1.5,
        "url": format!("https://freesound.org/s/{id}/"),
    })
}
fn page(results: Vec<serde_json::Value>, next: bool) -> serde_json::Value {
    serde_json::json!({
        "count": 40,
        "next": if next { Some("https://freesound.org/apiv2/search/?page=2") } else { None },
        "previous": null,
        "results": results,
    })
}
fn request() -> SearchRequest {
    SearchRequest {
        query: "whoosh".into(),
        page: 1,
        page_size: 3,
        commercial_only: true,
    }
}

#[test]
fn search_requests_exact_provider_contract_and_filters_commercial_licences() {
    let (client, transport) = client(page(
        vec![
            result(1, "Creative Commons 0"),
            result(2, "Attribution"),
            result(3, "Attribution NonCommercial"),
        ],
        true,
    ));
    let page = client.search(&request()).unwrap();
    assert_eq!(page.results.len(), 2);
    assert!(
        page.results
            .iter()
            .all(|sound| sound.licence != "Attribution NonCommercial")
    );
    assert!(page.has_next);
    assert_eq!(page.results[0].author, "user1");
    assert_eq!(page.results[0].audio_url, "https://cdn.example/1.mp3");
    assert_eq!(
        page.results[0].attribution,
        "Sound 1 by user1 (Creative Commons 0)"
    );
    assert_eq!(page.results[0].duration_ms, 1500);
    let requests = transport.requests.lock().unwrap();
    let url = &requests[0].url;
    assert!(
        url.starts_with("https://freesound.org/apiv2/search/?"),
        "{url}"
    );
    assert!(url.contains("query=whoosh"));
    assert!(url.contains("sort=downloads_desc"));
    assert!(url.contains("page=1"));
    assert!(url.contains("page_size=3"));
    assert!(url.contains(
        "id%2Cname%2Cusername%2Clicense%2Cnum_downloads%2Cpreviews%2Cdownload%2Cduration%2Curl"
    ));
    assert!(url.contains("license%3A%28%22Creative%20Commons%200%22%20OR%20%22Attribution%22%29"));
    assert!(!url.contains("secret-token"));
    assert_eq!(requests[0].headers[0].0, "Authorization");
    assert_eq!(requests[0].headers[0].1, "Token secret-token");
    assert!(!format!("{:?}", requests[0]).contains("secret-token"));
}
#[test]
fn search_all_licences_omits_filter_and_parses_page_two() {
    let (client, transport) = client(page(
        vec![
            result(1, "Creative Commons 0"),
            result(3, "Attribution NonCommercial"),
        ],
        false,
    ));
    let mut request = request();
    request.commercial_only = false;
    request.page = 2;
    let page = client.search(&request).unwrap();
    assert_eq!(page.results.len(), 2);
    assert!(!page.has_next);
    let requests = transport.requests.lock().unwrap();
    assert!(!requests[0].url.contains("filter="));
    assert!(requests[0].url.contains("page=2"));
}
#[test]
fn hostile_search_responses_and_provider_errors_are_typed() {
    let transport = Arc::new(JsonTransport {
        response: HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: vec![b'x'; (1 << 20) + 1],
        },
        requests: Mutex::new(Vec::new()),
    });
    let client = SoundClient::new(Token::new("k").unwrap(), transport);
    assert!(matches!(
        client.search(&request()),
        Err(engine::sounds::Error::Limit)
    ));

    for (status, headers, expected) in [
        (401, Vec::new(), "Auth"),
        (402, Vec::new(), "Quota"),
        (403, Vec::new(), "Auth"),
        (429, vec![("retry-after".into(), "7".into())], "RateLimit"),
        (503, Vec::new(), "Server"),
    ] {
        let transport = Arc::new(JsonTransport {
            response: HttpResponse {
                status,
                headers,
                body: b"{\"detail\":\"hostile detail\"}".to_vec(),
            },
            requests: Mutex::new(Vec::new()),
        });
        let client = SoundClient::new(Token::new("k").unwrap(), transport);
        let error = client.search(&request()).unwrap_err();
        match expected {
            "Auth" => assert!(matches!(error, engine::sounds::Error::Auth)),
            "Quota" => assert!(matches!(error, engine::sounds::Error::Quota)),
            "RateLimit" => assert!(matches!(
                error,
                engine::sounds::Error::RateLimit {
                    retry_after_seconds: Some(7)
                }
            )),
            "Server" => assert!(matches!(error, engine::sounds::Error::Server)),
            _ => panic!("unexpected error variant"),
        }
    }
    assert!(!format!("{:?}", Token::new("s3cr3t-value").unwrap()).contains("s3cr3t"));
}
#[test]
fn malformed_result_urls_and_bad_pages_are_refused() {
    let mut raw = result(1, "Creative Commons 0");
    raw["url"] = "http://insecure.example/1".into();
    let (bad_client, _) = client(page(vec![raw], false));
    assert!(matches!(
        bad_client.search(&request()),
        Err(engine::sounds::Error::InvalidResponse)
    ));
    let (client, _) = client(page(vec![result(1, "Creative Commons 0")], false));
    let mut bad = request();
    bad.page_size = 151;
    assert!(matches!(
        client.search(&bad),
        Err(engine::sounds::Error::Invalid("search request"))
    ));
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
    ) -> Result<HttpStreamResponse, ai::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(HttpStreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::new(Cursor::new(Vec::new())),
        })
    }
}
struct TrackedDownload {
    bytes: Vec<u8>,
    status: u16,
    headers: Vec<(String, String)>,
}
impl DownloadTransport for TrackedDownload {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, ai::Error> {
        Ok(HttpStreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::new(Cursor::new(self.bytes.clone())),
        })
    }
}
struct CancelReader {
    bytes: Vec<u8>,
    first: bool,
    cancel: Arc<AtomicBool>,
}
impl Read for CancelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.first {
            self.first = false;
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
    ) -> Result<HttpStreamResponse, ai::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: vec![("content-length".into(), self.bytes.len().to_string())],
            body: Box::new(CancelReader {
                bytes: self.bytes.clone(),
                first: true,
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
fn sound(id: u64, url: &str) -> SoundResult {
    SoundResult {
        id,
        name: format!("Sound {id}"),
        author: "Author".into(),
        licence: "Attribution".into(),
        attribution: format!("Sound {id} by Author (Attribution)"),
        source_url: format!("https://freesound.org/s/{id}/"),
        audio_url: url.into(),
        duration_ms: 250,
        downloads: 5,
    }
}

#[test]
fn saved_library_round_trips_offline_and_clear_all_removes_cache() {
    let root = tempfile::tempdir().unwrap();
    let sound = sound(7, "https://cdn.example/7.mp3");
    {
        let library = SoundLibrary::open(root.path()).unwrap();
        library.save(&sound).unwrap();
        assert!(library.is_saved(7).unwrap());
        library.store_cached(&sound.audio_url, &wav()).unwrap();
        assert!(library.cached(&sound.audio_url).unwrap().is_some());
        let page = library
            .search_local(&SearchRequest {
                query: "sound".into(),
                ..SearchRequest::default()
            })
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.source, engine::sounds::SearchSource::Local);
        assert!(
            !root
                .path()
                .join("library.json")
                .to_str()
                .unwrap()
                .contains("token")
        );
    }
    let library = SoundLibrary::open(root.path()).unwrap();
    assert!(library.is_saved(7).unwrap());
    assert!(library.cached(&sound.audio_url).unwrap().is_some());
    library.unsave(7).unwrap();
    assert!(!library.is_saved(7).unwrap());
    assert!(library.cached(&sound.audio_url).unwrap().is_none());
    library.save(&sound).unwrap();
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    assert_eq!(library.clear_all().unwrap(), 1);
    assert!(library.saved().unwrap().is_empty());
    assert!(library.cached(&sound.audio_url).unwrap().is_none());
}
#[test]
fn changed_source_url_invalidates_the_cached_preview() {
    let root = tempfile::tempdir().unwrap();
    let library = SoundLibrary::open(root.path()).unwrap();
    let mut sound = sound(9, "https://cdn.example/9-a.mp3");
    library.save(&sound).unwrap();
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    assert!(library.cached(&sound.audio_url).unwrap().is_some());
    sound.audio_url = "https://cdn.example/9-b.mp3".into();
    library.save(&sound).unwrap();
    assert!(
        library
            .cached("https://cdn.example/9-a.mp3")
            .unwrap()
            .is_none()
    );
    assert!(library.cached(&sound.audio_url).unwrap().is_none());
}
#[test]
fn local_search_honours_query_commercial_filter_and_pagination() {
    let root = tempfile::tempdir().unwrap();
    let library = SoundLibrary::open(root.path()).unwrap();
    for (id, licence, author, downloads) in [
        (1, "Creative Commons 0", "Alice", 5),
        (2, "Attribution NonCommercial", "Bob", 90),
        (3, "Attribution", "Alice", 50),
    ] {
        let mut result = sound(id, &format!("https://cdn.example/{id}.mp3"));
        result.licence = licence.into();
        result.author = author.into();
        result.attribution = format!("Sound by {author} ({licence})");
        result.downloads = downloads;
        library.save(&result).unwrap();
    }
    let page = library
        .search_local(&SearchRequest {
            query: "alice".into(),
            page: 1,
            page_size: 1,
            commercial_only: true,
        })
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.results[0].id, 3);
    assert!(page.has_next);
    let page = library
        .search_local(&SearchRequest {
            query: String::new(),
            page: 1,
            page_size: 15,
            commercial_only: false,
        })
        .unwrap();
    assert_eq!(page.count, 3);
    assert_eq!(page.results[0].id, 2);
}
#[test]
fn download_is_bounded_cancellable_and_publishes_only_complete_audio() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(SoundLibrary::open(root.path()).unwrap());
    let good = TrackedDownload {
        bytes: wav(),
        status: 200,
        headers: vec![("content-length".into(), wav().len().to_string())],
    };
    let downloads = SoundDownloads::new(library.clone(), Arc::new(good));
    let cached = downloads
        .download("https://cdn.example/good.mp3", &|| false, &|_, _| {})
        .unwrap();
    assert!(cached.duration().unwrap() > Tick::ZERO);
    assert!(cached.path.exists());

    let too_large = TrackedDownload {
        bytes: wav(),
        status: 200,
        headers: vec![("content-length".into(), (33 << 20).to_string())],
    };
    let downloads = SoundDownloads::new(library.clone(), Arc::new(too_large));
    assert!(matches!(
        downloads.download("https://cdn.example/big.mp3", &|| false, &|_, _| {}),
        Err(engine::sounds::Error::Limit)
    ));

    let cancel = Arc::new(AtomicBool::new(false));
    let cancelling = CancellingDownload {
        bytes: wav(),
        cancel: cancel.clone(),
    };
    let downloads = SoundDownloads::new(library.clone(), Arc::new(cancelling));
    assert!(matches!(
        downloads.download(
            "https://cdn.example/cancel.mp3",
            &|| cancel.load(Ordering::Acquire),
            &|_, _| {}
        ),
        Err(engine::sounds::Error::Cancelled)
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
    let downloads = SoundDownloads::new(library, Arc::new(missing));
    assert!(matches!(
        downloads.download("https://cdn.example/missing.mp3", &|| false, &|_, _| {}),
        Err(engine::sounds::Error::InvalidResponse)
    ));
}

struct Sink {
    plays: AtomicUsize,
    stops: AtomicUsize,
    device: bool,
}
impl PreviewSink for Sink {
    fn play(&self, clip: PreviewClip<'_>) -> engine::sounds::Sound<()> {
        if !self.device {
            return Err(engine::sounds::Error::NoAudioDevice);
        }
        assert!(!clip.bytes.is_empty());
        assert_eq!(clip.audio_url, "https://cdn.example/preview.mp3");
        assert!(!(clip.cancelled)());
        self.plays.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn stop(&self) -> engine::sounds::Sound<()> {
        self.stops.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[test]
fn preview_play_stop_and_missing_device_are_typed() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(SoundLibrary::open(root.path()).unwrap());
    let sound = sound(11, "https://cdn.example/preview.mp3");
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    let downloads = Arc::new(SoundDownloads::new(
        library.clone(),
        Arc::new(TrackedDownload {
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
    let preview = SoundPreview::new(downloads.clone(), sink.clone());
    preview.play(&sound).unwrap();
    preview.stop().unwrap();
    assert_eq!(sink.plays.load(Ordering::Relaxed), 1);
    assert_eq!(sink.stops.load(Ordering::Relaxed), 1);

    let no_device = Arc::new(Sink {
        plays: AtomicUsize::new(0),
        stops: AtomicUsize::new(0),
        device: false,
    });
    let preview = SoundPreview::new(downloads, no_device);
    assert!(matches!(
        preview.play(&sound),
        Err(engine::sounds::Error::NoAudioDevice)
    ));
    assert!(matches!(
        preview.play(&sound),
        Err(engine::sounds::Error::NoAudioDevice)
    ));
}
#[test]
fn adding_a_cached_sound_is_one_undo_step_and_round_trips_metadata() {
    let root = tempfile::tempdir().unwrap();
    let library = SoundLibrary::open(root.path()).unwrap();
    let sound = sound(21, "https://cdn.example/21.mp3");
    library.save(&sound).unwrap();
    let cached = library.store_cached(&sound.audio_url, &wav()).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(0), "sounds".into()).unwrap(), 8).unwrap();
    let outcome = add_saved(&mut session, &library, 21, Placement::Auto, Tick::ZERO).unwrap();
    assert_eq!(outcome.placements.len(), 1);
    assert_eq!(session.history_len(), (1, 0));
    let data = session.project().data();
    assert_eq!(data.assets.len(), 1);
    assert_eq!(data.assets[0].path(), cached.path.as_path());
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
    let AudioSource::Library { sound: stored } = &params.source else {
        panic!("expected library source");
    };
    let stored = stored.clone();
    assert_eq!(stored.id, 21);
    assert_eq!(stored.licence.as_str(), "Attribution");
    assert_eq!(stored.audio_url, sound.audio_url);

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
                AudioSource::Library { sound } => Some(sound.clone()),
                AudioSource::Upload => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(round_trip, stored);

    session.apply(Command::Undo {}).unwrap();
    assert!(session.project().data().assets.is_empty());
    assert!(
        session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .all(|track| track.items.is_empty())
    );
    session.apply(Command::Redo {}).unwrap();
    assert_eq!(session.project().data().assets.len(), 1);
}
#[test]
fn adding_an_uncached_sound_is_typed_and_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let library = SoundLibrary::open(root.path()).unwrap();
    let sound = sound(22, "https://cdn.example/22.mp3");
    library.save(&sound).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(1), "sounds".into()).unwrap(), 8).unwrap();
    assert!(matches!(
        add_saved(&mut session, &library, 22, Placement::Auto, Tick::ZERO),
        Err(EditError::Sounds(engine::sounds::Error::NotCached))
    ));
    assert!(session.project().data().assets.is_empty());
    assert_eq!(session.history_len(), (0, 0));
}
#[test]
fn library_refuses_symlinked_or_oversized_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("library.json");
    std::fs::write(&path, vec![b'x'; (1 << 20) + 1]).unwrap();
    assert!(matches!(
        SoundLibrary::open(root.path()),
        Err(engine::sounds::Error::LibraryCorrupt)
    ));
}

struct BlockingSink {
    started: Arc<AtomicBool>,
    stopped: AtomicUsize,
}
impl PreviewSink for BlockingSink {
    fn play(&self, clip: PreviewClip<'_>) -> engine::sounds::Sound<()> {
        self.started.store(true, Ordering::Release);
        for _ in 0..10_000_000 {
            if (clip.cancelled)() {
                return Err(engine::sounds::Error::Cancelled);
            }
            std::hint::spin_loop();
        }
        Err(engine::sounds::Error::Cancelled)
    }
    fn stop(&self) -> engine::sounds::Sound<()> {
        self.stopped.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[test]
fn preview_stop_cancels_playback_already_running_on_the_device() {
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(SoundLibrary::open(root.path()).unwrap());
    let sound = sound(12, "https://cdn.example/preview-stop.mp3");
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    let downloads = Arc::new(SoundDownloads::new(
        library,
        Arc::new(TrackedDownload {
            bytes: Vec::new(),
            status: 500,
            headers: Vec::new(),
        }),
    ));
    let sink = Arc::new(BlockingSink {
        started: Arc::new(AtomicBool::new(false)),
        stopped: AtomicUsize::new(0),
    });
    let preview = Arc::new(SoundPreview::new(downloads, sink.clone()));
    let playing = preview.clone();
    let thread = std::thread::spawn(move || playing.play(&sound));
    while !sink.started.load(Ordering::Acquire) {
        std::hint::spin_loop();
    }
    preview.stop().unwrap();
    assert!(matches!(
        thread.join().unwrap(),
        Err(engine::sounds::Error::Cancelled)
    ));
    assert_eq!(sink.stopped.load(Ordering::Relaxed), 1);
}

#[test]
fn sound_download_jobs_complete_and_cancel_through_the_shared_runner() {
    use engine::ByteBudget;
    use engine::jobs::{JobRunner, JobState, RunnerConfig, SystemClock};
    use std::time::Duration;
    let root = tempfile::tempdir().unwrap();
    let library = Arc::new(SoundLibrary::open(root.path()).unwrap());
    let good = TrackedDownload {
        bytes: wav(),
        status: 200,
        headers: vec![("content-length".into(), wav().len().to_string())],
    };
    let downloads = SoundDownloads::new(library.clone(), Arc::new(good));
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
}
