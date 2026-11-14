#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::SoundError;
use engine::{ByteBudget, Session};
use libraries::sounds::{
    PreviewClip, PreviewSink, SoundDownloads, SoundLibrary, SoundPreview, SoundResult, Token,
};
use project::{Project, ProjectId};
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use transport::{DownloadTransport, HttpRequest, HttpResponse, HttpStreamResponse, HttpTransport};
use vibecut::{
    Client, Editor, Sounds, SoundsMode, SoundsProblem, egui,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

fn result(id: u64, licence: &str) -> SoundResult {
    SoundResult {
        id,
        name: format!("Sound {id}"),
        author: format!("Author {id}"),
        licence: licence.into(),
        attribution: format!("Sound {id} by Author {id} ({licence})"),
        source_url: format!("https://freesound.org/s/{id}/"),
        audio_url: format!("https://cdn.example/{id}.mp3"),
        duration_ms: 34_000,
        downloads: 100 - id,
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

struct JsonTransport {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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
        if self.status == 0 {
            return Err(transport::Error::Network);
        }
        if self.body.len() > max_bytes {
            return Err(transport::Error::Payload);
        }
        Ok(HttpResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        })
    }
}

fn json_client(
    status: u16,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
) -> (Client, Arc<JsonTransport>) {
    let transport = Arc::new(JsonTransport {
        status,
        headers,
        body: serde_json::to_vec(&body).unwrap(),
        requests: Mutex::new(Vec::new()),
    });
    (
        Client::new(Token::new("secret-token").unwrap(), transport.clone()),
        transport,
    )
}

fn page(results: Vec<SoundResult>, next: bool) -> serde_json::Value {
    serde_json::json!({
        "count": results.len(),
        "next": if next { Some("https://freesound.org/apiv2/search/?page=2") } else { None },
        "previous": null,
        "results": results.iter().map(|sound| serde_json::json!({
            "id": sound.id,
            "name": sound.name,
            "username": sound.author,
            "license": sound.licence,
            "num_downloads": sound.downloads,
            "previews": {"preview-hq-mp3": sound.audio_url},
            "duration": sound.duration_ms as f64 / 1000.0,
            "url": sound.source_url,
        })).collect::<Vec<_>>(),
    })
}

struct Gate {
    open: Mutex<bool>,
    signal: Condvar,
}

impl Gate {
    fn new() -> Self {
        Self {
            open: Mutex::new(false),
            signal: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.signal.wait(open).unwrap();
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.signal.notify_all();
    }
}

struct GateBody {
    bytes: Vec<u8>,
    position: usize,
    gate: Arc<Gate>,
}

impl Read for GateBody {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.position == 0 {
            self.gate.wait();
        }
        let remaining = self.bytes.len().saturating_sub(self.position);
        if remaining == 0 {
            return Ok(0);
        }
        let take = remaining.min(buffer.len());
        buffer[..take].copy_from_slice(&self.bytes[self.position..self.position + take]);
        self.position += take;
        Ok(take)
    }
}

struct Download {
    gate: Option<Arc<Gate>>,
    status: u16,
}

impl DownloadTransport for Download {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, transport::Error> {
        if self.status != 200 {
            return Ok(HttpStreamResponse {
                status: self.status,
                headers: Vec::new(),
                body: Box::new(std::io::Cursor::new(Vec::new())),
            });
        }
        let bytes = wav();
        let body: Box<dyn Read + Send> = match &self.gate {
            Some(gate) => Box::new(GateBody {
                bytes,
                position: 0,
                gate: gate.clone(),
            }),
            None => Box::new(std::io::Cursor::new(bytes)),
        };
        Ok(HttpStreamResponse {
            status: 200,
            headers: vec![("content-length".into(), wav().len().to_string())],
            body,
        })
    }
}

struct Sink {
    plays: AtomicUsize,
    stops: AtomicUsize,
    device: bool,
    gate: Option<Arc<Gate>>,
}

impl Sink {
    fn available() -> Arc<Self> {
        Arc::new(Self {
            plays: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            device: true,
            gate: None,
        })
    }

    fn unavailable() -> Arc<Self> {
        Arc::new(Self {
            plays: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            device: false,
            gate: None,
        })
    }

    fn gated(gate: Arc<Gate>) -> Arc<Self> {
        Arc::new(Self {
            plays: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            device: true,
            gate: Some(gate),
        })
    }
}

impl PreviewSink for Sink {
    fn play(&self, _clip: PreviewClip<'_>) -> Result<(), SoundError> {
        self.plays.fetch_add(1, Ordering::SeqCst);
        if !self.device {
            return Err(SoundError::NoAudioDevice);
        }
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        Ok(())
    }

    fn stop(&self) -> Result<(), SoundError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.release();
        }
        Ok(())
    }
}

fn library(root: &std::path::Path) -> Arc<SoundLibrary> {
    Arc::new(SoundLibrary::open(root).unwrap())
}

fn editor(
    library: Arc<SoundLibrary>,
    client: Option<Client>,
    download: Arc<dyn DownloadTransport>,
    sink: Arc<Sink>,
    media: PathBuf,
) -> Editor {
    let session =
        Session::new(Project::empty(ProjectId(90), "Sounds".into()).unwrap(), 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    let downloads = Arc::new(SoundDownloads::new(library.clone(), download));
    let preview = Arc::new(SoundPreview::new(downloads.clone(), sink));
    let sounds = Sounds::with_parts(
        library,
        client,
        downloads,
        preview,
        ByteBudget::new(128 << 20).unwrap(),
    )
    .unwrap();
    editor.set_sounds(sounds);
    editor.set_sound_media_dir(media);
    editor
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    build_sized(editor, egui::vec2(1440.0, 900.0))
}

fn build_sized(editor: Editor, size: egui::Vec2) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(size)
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn widget_rect(harness: &Harness<Editor>, id: &str) -> egui::Rect {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .rect
}

fn has_widget(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}

fn wait(harness: &mut Harness<Editor>, what: &str, mut predicate: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(1);
        if predicate(harness.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            harness.state().inspect().unwrap().sounds
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn project_bytes(editor: &Editor) -> Vec<u8> {
    serde_json::to_vec(editor.session.project().data()).unwrap()
}

#[test]
fn search_filter_save_preview_and_add_use_the_sounds_contract() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(1, "Creative Commons 0");
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    let (client, transport) = json_client(
        200,
        Vec::new(),
        page(
            vec![
                sound.clone(),
                result(2, "Attribution NonCommercial"),
                result(3, "Attribution"),
            ],
            false,
        ),
    );
    let gate = Arc::new(Gate::new());
    let sink = Sink::gated(gate.clone());
    let mut editor = editor(
        library.clone(),
        Some(client),
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        sink.clone(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    assert!(has_widget(&harness, "sounds.search"));
    assert!(has_widget(&harness, "sounds.commercial"));
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .sounds
            .unwrap()
            .commercial_only,
        "commercial-only starts on"
    );

    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("sound");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "first page", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| !sounds.searching && sounds.results.len() == 2)
    });
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert_eq!(sounds.results[0].licence, "Creative Commons 0");
    assert_eq!(sounds.results[1].licence, "Attribution");
    assert_eq!(sounds.results[0].attribution, sound.attribution);
    assert!(has_widget(&harness, "sounds.result.1"));
    assert!(!has_widget(&harness, "sounds.result.2"));
    assert!(has_widget(&harness, "sounds.preview.1"));
    assert!(has_widget(&harness, "sounds.save.1"));
    assert!(has_widget(&harness, "sounds.add.1"));
    let panel = harness.state().panels()["media"];
    for id in ["sounds.preview.1", "sounds.save.1", "sounds.add.1"] {
        assert!(
            panel.intersects(widget_rect(&harness, id)),
            "{id} must sit inside the assets panel"
        );
    }
    let before = project_bytes(harness.state());
    assert!(
        transport
            .requests
            .lock()
            .unwrap()
            .first()
            .is_some_and(|request| request.url.contains("filter=")),
        "the first request carries the commercial filter"
    );

    click(&mut harness, "sounds.commercial");
    wait(&mut harness, "all licences", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| !sounds.searching && sounds.results.len() == 3)
    });
    assert!(has_widget(&harness, "sounds.result.2"));
    assert!(
        transport
            .requests
            .lock()
            .unwrap()
            .last()
            .is_some_and(|request| !request.url.contains("filter=")),
        "turning the switch off omits the commercial filter"
    );

    click(&mut harness, "sounds.save.1");
    wait(&mut harness, "saved", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.saved_ids.contains(&1))
    });
    assert_eq!(library.saved().unwrap().len(), 1);
    click(&mut harness, "sounds.tab.saved");
    wait(&mut harness, "saved tab", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.mode == SoundsMode::Saved)
    });
    let saved = harness.state().inspect().unwrap().sounds.unwrap();
    assert_eq!(saved.saved.len(), 1);
    assert_eq!(saved.saved[0].sound.attribution, sound.attribution);
    assert_eq!(saved.saved[0].sound.licence, "Creative Commons 0");
    assert!(has_widget(&harness, "sounds.result.1"));
    assert_eq!(
        project_bytes(harness.state()),
        before,
        "search and save never edit the project"
    );

    click(&mut harness, "sounds.tab.discover");
    // Switching tabs starts a search on a thread; the "searching" line moves
    // the rows, so click only once the list has settled.
    wait(&mut harness, "discover results settled", |editor| {
        editor.inspect().unwrap().sounds.is_some_and(|sounds| {
            sounds.mode == SoundsMode::Discover && !sounds.searching && sounds.results.len() == 3
        })
    });
    harness.run_steps(1);
    click(&mut harness, "sounds.preview.1");
    wait(&mut harness, "preview started", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.previewing == Some(1))
    });
    // `previewing` is set on click; the sink is reached once the preview
    // thread has read the cached audio.
    let playing = sink.clone();
    wait(&mut harness, "sink playing", move |_| {
        playing.plays.load(Ordering::SeqCst) == 1
    });
    click(&mut harness, "sounds.preview.1");
    wait(&mut harness, "preview stopped", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.previewing.is_none())
    });
    assert_eq!(sink.stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        project_bytes(harness.state()),
        before,
        "preview never edits the project"
    );

    assert_eq!(harness.state().session.history_len(), (0, 0));
    click(&mut harness, "sounds.add.1");
    wait(&mut harness, "added item", |editor| {
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .any(|item| item.source_asset.is_some())
    });
    assert_eq!(harness.state().session.history_len(), (1, 0));
    let project = harness.state().session.project();
    let items: Vec<_> = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0].content, project::ItemParams::Audio(_)));
    assert_eq!(project.data().assets.len(), 1);
}

#[test]
fn provider_errors_and_empty_results_are_rendered_as_typed_states() {
    for (status, headers, expected) in [
        (401, Vec::new(), SoundsProblem::Auth),
        (402, Vec::new(), SoundsProblem::Quota),
        (
            429,
            vec![("retry-after".into(), "7".into())],
            SoundsProblem::RateLimit(Some(7)),
        ),
        (503, Vec::new(), SoundsProblem::Server),
        (0, Vec::new(), SoundsProblem::Network),
        (200, Vec::new(), SoundsProblem::Invalid),
    ] {
        let root = tempfile::tempdir().unwrap();
        let library = library(root.path());
        let body = serde_json::json!({"detail": "hostile"});
        let (client, _) = json_client(status, headers, body);
        let mut editor = editor(
            library,
            Some(client),
            Arc::new(Download {
                gate: None,
                status: 200,
            }),
            Sink::available(),
            root.path().join("media"),
        );
        editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
        let mut harness = build(editor);
        harness.run_steps(3);
        click(&mut harness, "sounds.search");
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
            .type_text("rain");
        harness.key_press(egui::Key::Enter);
        wait(&mut harness, "typed search state", |editor| {
            editor
                .inspect()
                .unwrap()
                .sounds
                .is_some_and(|sounds| !sounds.searching)
        });
        let sounds = harness.state().inspect().unwrap().sounds.unwrap();
        if status == 200 {
            assert_eq!(sounds.search_problem, Some(SoundsProblem::Invalid));
        } else {
            assert_eq!(sounds.search_problem, Some(expected), "status {status}");
        }
        assert!(has_widget(&harness, "sounds.retry"));
    }
}

#[test]
fn empty_search_and_empty_saved_tab_show_their_empty_states() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let (client, _) = json_client(200, Vec::new(), page(Vec::new(), false));
    let mut editor = editor(
        library,
        Some(client),
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("missing");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "empty search", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| !sounds.searching && sounds.results.is_empty())
    });
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert!(sounds.search_problem.is_none());
    assert!(!has_widget(&harness, "sounds.result.1"));
    click(&mut harness, "sounds.tab.saved");
    wait(&mut harness, "empty saved", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.mode == SoundsMode::Saved && sounds.saved.is_empty())
    });
    assert!(!has_widget(&harness, "sounds.result.1"));
}

#[test]
fn no_token_uses_the_local_library_and_the_saved_tab_works_offline() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(4, "Creative Commons 0");
    library.save(&sound).unwrap();
    let mut editor = editor(
        library,
        None,
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert!(!sounds.has_token);
    assert!(has_widget(&harness, "sounds.tab.saved"));
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("sound");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "local search", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| !sounds.searching && sounds.results.len() == 1)
    });
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert_eq!(sounds.source, libraries::sounds::SearchSource::Local);
    assert_eq!(sounds.results[0].attribution, sound.attribution);
    assert_eq!(sounds.results[0].licence, sound.licence);
    click(&mut harness, "sounds.tab.saved");
    wait(&mut harness, "saved tab", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.mode == SoundsMode::Saved)
    });
    assert!(has_widget(&harness, "sounds.result.4"));
    assert!(has_widget(&harness, "sounds.preview.4"));
    assert!(has_widget(&harness, "sounds.save.4"));
}

#[test]
fn no_audio_device_is_a_typed_preview_state() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(5, "Creative Commons 0");
    library.save(&sound).unwrap();
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    let mut editor = editor(
        library,
        None,
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        Sink::unavailable(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "sounds.tab.saved");
    wait(&mut harness, "saved tab", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.mode == SoundsMode::Saved)
    });
    click(&mut harness, "sounds.preview.5");
    wait(&mut harness, "no device", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.preview_problem == Some(SoundsProblem::NoAudioDevice))
    });
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert_eq!(sounds.preview_problem, Some(SoundsProblem::NoAudioDevice));
    assert!(sounds.previewing.is_none());
}

#[test]
fn failed_cache_download_offers_network_retry_without_editing_the_project() {
    struct Recovering(AtomicUsize);
    impl DownloadTransport for Recovering {
        fn get(
            &self,
            url: &str,
            headers: &[(String, String)],
        ) -> Result<HttpStreamResponse, transport::Error> {
            Download {
                gate: None,
                status: if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    503
                } else {
                    200
                },
            }
            .get(url, headers)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(6, "Creative Commons 0");
    library.save(&sound).unwrap();
    let http = Arc::new(Recovering(AtomicUsize::new(0)));
    let mut editor = editor(
        library.clone(),
        None,
        http.clone(),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("sound");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "local result", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.results.len() == 1)
    });
    let before = project_bytes(harness.state());
    click(&mut harness, "sounds.add.6");
    wait(&mut harness, "network failure", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.cache_problem == Some(SoundsProblem::Network))
    });
    assert_eq!(http.0.load(Ordering::SeqCst), 1);
    assert!(library.cached(&sound.audio_url).unwrap().is_none());
    assert_eq!(project_bytes(harness.state()), before);
    assert_eq!(harness.state().session.history_len(), (0, 0));
    assert!(has_widget(&harness, "sounds.add.6"));
    click(&mut harness, "sounds.add.6");
    wait(&mut harness, "retry added sound", |editor| {
        editor.session.history_len() == (1, 0)
    });
    assert_eq!(http.0.load(Ordering::SeqCst), 2);
    assert!(library.cached(&sound.audio_url).unwrap().is_some());
    assert_ne!(project_bytes(harness.state()), before);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .sounds
            .unwrap()
            .cache_problem
            .is_none()
    );
}

#[test]
fn shared_download_contention_keeps_the_cache_busy_category_and_allows_a_later_add() {
    use engine::jobs::{JobRunner, JobState, RunnerConfig, SystemClock};
    struct Counted(AtomicUsize);
    impl DownloadTransport for Counted {
        fn get(
            &self,
            url: &str,
            headers: &[(String, String)],
        ) -> Result<HttpStreamResponse, transport::Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Download {
                gate: None,
                status: 200,
            }
            .get(url, headers)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(6, "Creative Commons 0");
    library.save(&sound).unwrap();
    let http = Arc::new(Counted(AtomicUsize::new(0)));
    let downloads = Arc::new(SoundDownloads::new(library.clone(), http.clone()));
    let budget = ByteBudget::new(128 << 20).unwrap();
    let mut owner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(10),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    let (queued, cancel) = downloads.submit(&mut owner, &sound.audio_url).unwrap();
    let sink = Sink::available();
    let preview = Arc::new(SoundPreview::new(downloads.clone(), sink.clone()));
    let sounds = Sounds::with_parts(library.clone(), None, downloads, preview, budget).unwrap();
    let mut editor = editor(
        library.clone(),
        None,
        http.clone(),
        sink,
        root.path().join("media"),
    );
    editor.set_sounds(sounds);
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("sound");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "local result", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.results.len() == 1)
    });
    let before = project_bytes(harness.state());
    click(&mut harness, "sounds.add.6");
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert_eq!(sounds.cache_problem, Some(SoundsProblem::CacheBusy));
    assert!(sounds.cache.is_none());
    assert_eq!(http.0.load(Ordering::SeqCst), 0);
    assert!(library.cached(&sound.audio_url).unwrap().is_none());
    assert_eq!(project_bytes(harness.state()), before);
    assert_eq!(harness.state().session.history_len(), (0, 0));
    cancel.cancel();
    owner.tick().unwrap();
    assert_eq!(owner.snapshot(queued).unwrap().state, JobState::Cancelled);
    click(&mut harness, "sounds.add.6");
    wait(&mut harness, "add after contention releases", |editor| {
        editor.session.history_len() == (1, 0)
    });
    assert_eq!(http.0.load(Ordering::SeqCst), 1);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .sounds
            .unwrap()
            .cache_problem
            .is_none()
    );
    assert!(library.cached(&sound.audio_url).unwrap().is_some());
}

#[test]
fn add_waits_for_the_cache_job_and_cancel_leaves_the_project_untouched() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(6, "Creative Commons 0");
    library.save(&sound).unwrap();
    let gate = Arc::new(Gate::new());
    let mut editor = editor(
        library,
        None,
        Arc::new(Download {
            gate: Some(gate.clone()),
            status: 200,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    // A local search makes the uncached result visible without a token.
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("sound");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "local result", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.results.len() == 1)
    });
    let before = project_bytes(harness.state());
    click(&mut harness, "sounds.add.6");
    wait(&mut harness, "cache job", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.cache.is_some())
    });
    assert!(has_widget(&harness, "sounds.cancel_cache"));
    click(&mut harness, "sounds.cancel_cache");
    gate.release();
    wait(&mut harness, "cancelled cache", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.cache.is_none())
    });
    assert_eq!(project_bytes(harness.state()), before);
    assert_eq!(harness.state().session.history_len(), (0, 0));
}

#[test]
fn failed_saved_library_publication_is_reported_without_losing_the_list() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(7, "Creative Commons 0");
    library.save(&sound).unwrap();
    let mut editor = editor(
        library,
        None,
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    let path = root.path().join("library.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    click(&mut harness, "sounds.tab.saved");
    click(&mut harness, "sounds.save.7");
    wait(&mut harness, "publication failure", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.saved_problem.is_some())
    });
    let sounds = harness.state().inspect().unwrap().sounds.unwrap();
    assert!(sounds.saved_problem.is_some());
    assert!(
        sounds.saved_ids.contains(&7),
        "the in-memory list is retained"
    );
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-43-sounds.png"]
fn sounds_tab_renders_a_representative_result_list() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(1, "Creative Commons 0");
    library.save(&sound).unwrap();
    library.store_cached(&sound.audio_url, &wav()).unwrap();
    let (client, _) = json_client(
        200,
        Vec::new(),
        page(vec![sound, result(3, "Attribution")], true),
    );
    let mut editor = editor(
        library,
        Some(client),
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build_sized(editor, egui::vec2(1727.0, 1029.0));
    harness.run_steps(3);
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("whoosh");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "results", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| sounds.results.len() == 2)
    });
    let image = harness.render().expect("offscreen render");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-43-sounds.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

#[test]
fn sounds_favorites_star_list_unstar_and_show_the_empty_state() {
    use vibecut::preferences::SoundFavorite;

    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let sound = result(1, "Creative Commons 0");
    let (client, _transport) = json_client(
        200,
        Vec::new(),
        page(vec![sound.clone(), result(2, "Attribution")], false),
    );
    let gate = Arc::new(Gate::new());
    let sink = Sink::gated(gate.clone());
    let mut editor = editor(
        library.clone(),
        Some(client),
        Arc::new(Download {
            gate: None,
            status: 200,
        }),
        sink.clone(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "sounds.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("sounds.search")))
        .type_text("sound");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "first page", |editor| {
        editor
            .inspect()
            .unwrap()
            .sounds
            .is_some_and(|sounds| !sounds.searching && sounds.results.len() == 2)
    });

    // Star from the results: the metadata Favorites needs is stored with the star.
    click(&mut harness, "sounds.favorite.1");
    assert_eq!(
        harness.state().preferences.catalog_favorites.sounds(),
        [SoundFavorite {
            id: 1,
            title: sound.name.clone(),
            duration_ms: sound.duration_ms,
            preview_url: sound.audio_url.clone(),
        }]
    );
    assert!(
        harness
            .state()
            .preferences
            .catalog_favorites
            .contains("sounds", "1")
    );

    // Listed under Favorites; the unstarred result stays out of the list.
    click(&mut harness, "sounds.tab.favorites");
    harness.run_steps(3);
    assert!(has_widget(&harness, "sounds.result.1"));
    assert!(!has_widget(&harness, "sounds.result.2"));

    // Unstar from the Favorites list removes it and leaves the empty state.
    click(&mut harness, "sounds.favorite.1");
    harness.run_steps(3);
    assert!(
        !harness
            .state()
            .preferences
            .catalog_favorites
            .contains("sounds", "1")
    );
    assert!(
        harness
            .state()
            .preferences
            .catalog_favorites
            .sounds()
            .is_empty()
    );
    assert!(!has_widget(&harness, "sounds.result.1"));
    assert!(!has_widget(&harness, "sounds.favorite.1"));
}
