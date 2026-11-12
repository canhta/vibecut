#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use ai::transport::{
    DownloadTransport, HttpRequest, HttpResponse, HttpStreamResponse, HttpTransport,
};
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use libraries::music::{
    MusicClient, MusicDownloads, MusicLibrary, MusicPreview, MusicResult, PreviewClip, PreviewSink,
};
use project::{AudioSource, ItemParams, Project, ProjectId};
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use vibecut::{
    Editor, Music, MusicMode, egui,
    preferences::Preferences,
    services::music::MusicSession,
    services::playback::{Files, Playback},
};

fn result(id: &str) -> MusicResult {
    MusicResult {
        id: id.into(),
        title: format!("Track {id}"),
        author: format!("Artist {id}"),
        licence: "CC BY 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
        attribution: format!("Track {id} by Artist {id} is licensed under CC BY 4.0."),
        source_url: format!("https://source.example/{id}"),
        audio_url: format!("https://cdn.example/{id}.mp3"),
        duration_ms: 31_000,
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
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, ai::Error> {
        self.send_bounded(request, 1 << 20)
    }

    fn send_bounded(
        &self,
        request: HttpRequest,
        max_bytes: usize,
    ) -> Result<HttpResponse, ai::Error> {
        self.requests.lock().unwrap().push(request);
        if self.status == 0 {
            return Err(ai::Error::Network);
        }
        if self.body.len() > max_bytes {
            return Err(ai::Error::Payload);
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
) -> (MusicClient, Arc<JsonTransport>) {
    let transport = Arc::new(JsonTransport {
        status,
        headers,
        body: serde_json::to_vec(&body).unwrap(),
        requests: Mutex::new(Vec::new()),
    });
    (MusicClient::new(None, transport.clone()), transport)
}

fn page(results: Vec<MusicResult>, pages: u64) -> serde_json::Value {
    serde_json::json!({
        "result_count": results.len(),
        "page_count": pages,
        "page_size": 15,
        "page": 1,
        "results": results.iter().map(|track| serde_json::json!({
            "id": track.id,
            "title": track.title,
            "creator": track.author,
            "license": "by",
            "license_version": "4.0",
            "license_url": track.licence_url,
            "attribution": track.attribution,
            "foreign_landing_url": track.source_url,
            "url": track.audio_url,
            "duration": track.duration_ms,
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
    ) -> Result<HttpStreamResponse, ai::Error> {
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
    fn play(&self, _clip: PreviewClip<'_>) -> Result<(), engine::SoundError> {
        self.plays.fetch_add(1, Ordering::SeqCst);
        if !self.device {
            return Err(engine::SoundError::NoAudioDevice);
        }
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        Ok(())
    }

    fn stop(&self) -> Result<(), engine::SoundError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.release();
        }
        Ok(())
    }
}

fn library(root: &std::path::Path) -> Arc<MusicLibrary> {
    Arc::new(MusicLibrary::open(root).unwrap())
}

fn editor(
    library: Arc<MusicLibrary>,
    client: Option<MusicClient>,
    download: Arc<dyn DownloadTransport>,
    sink: Arc<Sink>,
    media: PathBuf,
) -> Editor {
    let session = Session::new(Project::empty(ProjectId(91), "Music".into()).unwrap(), 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    let downloads = Arc::new(MusicDownloads::new(library.clone(), download));
    let preview = Arc::new(MusicPreview::new(downloads.clone(), sink));
    let music = Music::with_parts(
        Arc::new(MusicSession::new(library)),
        client,
        downloads,
        preview,
        ByteBudget::new(128 << 20).unwrap(),
    )
    .unwrap();
    editor.set_music(music);
    editor.set_music_media_dir(media);
    editor
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn open_music(harness: &mut Harness<Editor>) {
    click(harness, "audio.section.music");
    harness.run_steps(2);
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
            harness.state().inspect().unwrap().music
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn project_bytes(editor: &Editor) -> Vec<u8> {
    serde_json::to_vec(editor.session.project().data()).unwrap()
}

fn music_count(editor: &Editor) -> usize {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .filter(|item| matches!(item.content, ItemParams::Audio(_)))
        .count()
}

#[test]
fn section_switches_between_music_and_sound_effects_and_loads_a_tag_page() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let (client, transport) = json_client(200, Vec::new(), page(vec![result("one")], 1));
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
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    assert!(has_widget(&harness, "audio.section.music"));
    assert!(has_widget(&harness, "audio.section.sounds"));
    assert!(
        !has_widget(&harness, "music.search"),
        "the Audio tab opens on sound effects"
    );
    open_music(&mut harness);
    assert!(!has_widget(&harness, "sounds.search"));
    assert!(has_widget(&harness, "music.search"));
    wait(&mut harness, "first tag page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| !music.searching && music.results.len() == 1)
    });
    let music = harness.state().inspect().unwrap().music.unwrap();
    assert_eq!(music.mode, MusicMode::Browse);
    assert_eq!(music.chip, "ambient");
    assert_eq!(music.results[0].licence, "CC BY 4.0");
    assert!(has_widget(&harness, "music.result.one"));
    assert!(has_widget(&harness, "music.save.one"));
    assert!(has_widget(&harness, "music.add.one"));
    let requests = transport.requests.lock().unwrap();
    let url = &requests[0].url;
    assert!(url.contains("tags=ambient"), "{url}");
    assert!(!url.contains("q="), "{url}");
    assert!(url.contains("license=cc0%2Cby"), "{url}");
    assert!(url.contains("category=music"), "{url}");
    drop(requests);

    // The genre rails are source tags, and the free-text search sends `q` only.
    click(&mut harness, "music.chip.piano");
    wait(&mut harness, "piano page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.chip == "piano" && !music.searching)
    });
    assert!(
        transport.requests.lock().unwrap()[1]
            .url
            .contains("tags=piano")
    );
    click(&mut harness, "music.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("music.search")))
        .type_text("rain");
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "query page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| !music.searching)
    });
    let requests = transport.requests.lock().unwrap();
    let last = requests.last().unwrap().url.clone();
    assert!(last.contains("q=rain"), "{last}");
    assert!(!last.contains("tags="), "{last}");
    drop(requests);

    click(&mut harness, "audio.section.sounds");
    assert!(!has_widget(&harness, "music.search"));
    assert!(has_widget(&harness, "sounds.tab.discover"));
}

#[test]
fn save_preview_and_download_then_add_is_one_undo_step_with_music_metadata() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let track = result("two");
    let (client, _) = json_client(200, Vec::new(), page(vec![track.clone()], 1));
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
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    wait(&mut harness, "page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.results.len() == 1)
    });
    let before = project_bytes(harness.state());

    click(&mut harness, "music.save.two");
    let music = harness.state().inspect().unwrap().music.unwrap();
    assert!(music.saved_ids.contains("two"));
    assert_eq!(
        project_bytes(harness.state()),
        before,
        "save is not an edit"
    );

    click(&mut harness, "music.preview.two");
    // The sink is gated; stop releases it and the row stops showing playback.
    wait(&mut harness, "preview start", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.previewing.as_deref() == Some("two"))
    });
    let playing = sink.clone();
    wait(&mut harness, "sink playing", move |_| {
        playing.plays.load(Ordering::SeqCst) == 1
    });
    assert_eq!(sink.plays.load(Ordering::SeqCst), 1);
    click(&mut harness, "music.preview.two");
    gate.release();
    wait(&mut harness, "preview stop", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.previewing.is_none())
    });
    assert_eq!(sink.stops.load(Ordering::SeqCst), 1);

    click(&mut harness, "music.add.two");
    wait(&mut harness, "add completes", |editor| {
        music_count(editor) == 1
    });
    assert_eq!(
        harness.state().session.history_len(),
        (1, 0),
        "one undo step"
    );
    let data = harness.state().session.project().data().clone();
    let item = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| matches!(item.content, ItemParams::Audio(_)))
        .unwrap();
    let ItemParams::Audio(params) = &item.content else {
        panic!("audio item");
    };
    let AudioSource::Music { music: stored } = &params.source else {
        panic!("music source");
    };
    assert_eq!(stored.id.as_str(), "two");
    assert_eq!(stored.title.as_str(), "Track two");
    assert_eq!(stored.author.as_str(), "Artist two");
    assert_eq!(stored.licence.as_str(), "CC BY 4.0");
    assert_eq!(stored.licence_url, track.licence_url);
    assert_eq!(stored.attribution.as_str(), track.attribution);
    assert_eq!(stored.source_url, track.source_url);
    assert!(
        library.cached(&track.audio_url).unwrap().is_some(),
        "the track is cached locally"
    );
    harness
        .state_mut()
        .session
        .apply(engine::Command::Undo {})
        .unwrap();
    assert_eq!(music_count(harness.state()), 0);
}

#[test]
fn saved_list_is_offline_and_credits_come_from_the_project() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let used = result("a");
    let unused = result("b");
    library.save(&used).unwrap();
    library.save(&unused).unwrap();
    let cached = library.store_cached(&used.audio_url, &wav()).unwrap();
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
    // Only track a is used in the project; track b stays saved but unused.
    libraries::music::add_to_timeline(
        &mut editor.session,
        &used,
        &cached,
        &root.path().join("media"),
        engine::Placement::Auto,
        time::Tick::ZERO,
    )
    .unwrap();
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    click(&mut harness, "music.tab.saved");
    wait(&mut harness, "saved list", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.mode == MusicMode::Saved && music.results.len() == 2)
    });
    assert!(has_widget(&harness, "music.result.a"));
    assert!(has_widget(&harness, "music.result.b"));
    assert!(has_widget(&harness, "music.credits"));
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("music.credits")))
        .click();
    harness.run_steps(1);
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("credits copy output");
    assert!(copied.contains("Track a by Artist a"));
    assert!(
        !copied.contains("Track b by Artist b"),
        "unused saved tracks are not project credits: {copied}"
    );
}

#[test]
fn rows_use_the_measured_48_by_60_pitch() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    library.save(&result("a")).unwrap();
    library.save(&result("b")).unwrap();
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
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    click(&mut harness, "music.tab.saved");
    wait(&mut harness, "saved rows", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.results.len() == 2)
    });
    let widgets = harness.state().inspect().unwrap().widgets;
    let row = |id: &str| {
        widgets
            .iter()
            .find(|widget| widget.id == id)
            .map(|widget| widget.rect)
            .unwrap()
    };
    let a = row("music.result.a");
    let b = row("music.result.b");
    assert_eq!(a.height(), 48.0);
    assert_eq!(b.height(), 48.0);
    assert_eq!(b.top() - a.top(), 60.0);
}

#[test]
fn settings_apply_and_remove_the_session_key_without_touching_the_project() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
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
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    let before = project_bytes(harness.state());
    click(&mut harness, "music.settings");
    assert!(has_widget(&harness, "music.key"));
    assert!(has_widget(&harness, "music.terms"));
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("music.key")))
        .type_text("key-123");
    harness.run_steps(2);
    click(&mut harness, "music.key.apply");
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.key_set)
    );
    assert_eq!(project_bytes(harness.state()), before);
    assert!(!String::from_utf8_lossy(&before).contains("key-123"));
    click(&mut harness, "music.key.remove");
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| !music.key_set)
    );
    assert_eq!(project_bytes(harness.state()), before);
}

struct FlakyDownload {
    calls: AtomicUsize,
}

impl DownloadTransport for FlakyDownload {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, ai::Error> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let status = if call == 0 { 500 } else { 200 };
        Ok(HttpStreamResponse {
            status,
            headers: if status == 200 {
                vec![("content-length".into(), wav().len().to_string())]
            } else {
                Vec::new()
            },
            body: Box::new(std::io::Cursor::new(wav())),
        })
    }
}

#[test]
fn failed_add_retry_resubmits_the_track_without_reloading_search() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    library.save(&result("retry")).unwrap();
    let download = Arc::new(FlakyDownload {
        calls: AtomicUsize::new(0),
    });
    let mut editor = editor(
        library,
        None,
        download.clone(),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    click(&mut harness, "music.tab.saved");
    wait(&mut harness, "saved row", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.results.len() == 1)
    });
    click(&mut harness, "music.add.retry");
    wait(&mut harness, "failed add", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.cache_problem.is_some())
    });
    assert_eq!(music_count(harness.state()), 0);
    assert_eq!(download.calls.load(Ordering::SeqCst), 1);
    click(&mut harness, "music.retry.cache");
    wait(&mut harness, "retry add", |editor| music_count(editor) == 1);
    assert_eq!(download.calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.state().session.history_len(), (1, 0));
}

#[test]
fn provider_failures_are_typed_states_and_leave_the_project_untouched() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    for (status, headers, expected) in [
        (429, vec![("retry-after".into(), "7".into())], "RateLimit"),
        (503, Vec::new(), "Network"),
        (500, Vec::new(), "Server"),
        (0, Vec::new(), "Network"),
    ] {
        let (client, _) = json_client(status, headers, serde_json::json!({}));
        let mut editor = editor(
            library.clone(),
            Some(client),
            Arc::new(Download {
                gate: None,
                status: 200,
            }),
            Sink::available(),
            root.path().join("media"),
        );
        editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
        let mut harness = build(editor);
        harness.run_steps(3);
        open_music(&mut harness);
        wait(&mut harness, expected, |editor| {
            editor
                .inspect()
                .unwrap()
                .music
                .is_some_and(|music| music.search_problem.is_some())
        });
        let problem = harness
            .state()
            .inspect()
            .unwrap()
            .music
            .unwrap()
            .search_problem
            .unwrap();
        match expected {
            "RateLimit" => assert_eq!(
                format!("{problem:?}"),
                "RateLimit(Some(7))",
                "retry-after is surfaced"
            ),
            "Network" => assert_eq!(format!("{problem:?}"), "Network"),
            "Server" => assert_eq!(format!("{problem:?}"), "Server"),
            _ => panic!("unexpected"),
        }
        assert!(has_widget(&harness, "music.retry"), "{expected}");
        assert_eq!(music_count(harness.state()), 0);
        assert!(
            harness.state().session.history_len() == (0, 0),
            "a provider failure is not an edit"
        );
    }
}

#[test]
fn a_failed_download_leaves_no_project_media_and_a_cancel_publishes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let track = result("three");
    let (client, _) = json_client(200, Vec::new(), page(vec![track.clone()], 1));
    let gate = Arc::new(Gate::new());
    let mut editor = editor(
        library.clone(),
        Some(client),
        Arc::new(Download {
            gate: Some(gate.clone()),
            status: 200,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    wait(&mut harness, "page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.results.len() == 1)
    });
    let before = project_bytes(harness.state());
    click(&mut harness, "music.add.three");
    wait(&mut harness, "cache starts", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.cache.is_some())
    });
    click(&mut harness, "music.cancel_cache");
    // Release the transport so cancellation happens after the stream begins.
    gate.release();
    wait(&mut harness, "cache cancelled", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.cache.is_none())
    });
    assert_eq!(project_bytes(harness.state()), before);
    assert_eq!(music_count(harness.state()), 0);
    assert!(library.cached(&track.audio_url).unwrap().is_none());
}

#[test]
fn a_rejected_download_is_reported_and_publishes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let track = result("four");
    let (client, _) = json_client(200, Vec::new(), page(vec![track.clone()], 1));
    let mut editor = editor(
        library.clone(),
        Some(client),
        Arc::new(Download {
            gate: None,
            status: 404,
        }),
        Sink::available(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    wait(&mut harness, "page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.results.len() == 1)
    });
    let before = project_bytes(harness.state());
    click(&mut harness, "music.add.four");
    wait(&mut harness, "download refused", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.cache_problem.is_some() && music.cache.is_none())
    });
    assert_eq!(project_bytes(harness.state()), before);
    assert!(library.cached(&track.audio_url).unwrap().is_none());
}

#[test]
fn a_missing_audio_device_is_a_typed_preview_failure() {
    let root = tempfile::tempdir().unwrap();
    let library = library(root.path());
    let track = result("five");
    library.store_cached(&track.audio_url, &wav()).unwrap();
    let (client, _) = json_client(200, Vec::new(), page(vec![track.clone()], 1));
    let mut editor = editor(
        library,
        Some(client),
        Arc::new(Download {
            gate: None,
            status: 500,
        }),
        Sink::unavailable(),
        root.path().join("media"),
    );
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Sounds;
    let mut harness = build(editor);
    harness.run_steps(3);
    open_music(&mut harness);
    wait(&mut harness, "page", |editor| {
        editor
            .inspect()
            .unwrap()
            .music
            .is_some_and(|music| music.results.len() == 1)
    });
    click(&mut harness, "music.preview.five");
    wait(&mut harness, "device failure", |editor| {
        editor.inspect().unwrap().music.is_some_and(|music| {
            music.preview_problem == Some(vibecut::MusicProblem::NoAudioDevice)
        })
    });
}
