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
use engine::{ByteBudget, Command, Session};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, MediaError, SourceAsset, SourceProbe,
    SourceStream, StreamKind, TimeBase, VideoInfo,
};
use project::{ItemParams, Project, ProjectId, TrackKind};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{FrameRate, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, Error,
    catalog::{Locale, Preferences},
    egui,
    import::{ImportEvent, ImportProbe, ImportQueue, MAX_IMPORT_FILES},
    playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

fn stream(kind: StreamKind, duration: Tick) -> SourceStream {
    SourceStream {
        index: 0,
        kind,
        codec: match kind {
            StreamKind::Video => "h264".into(),
            StreamKind::Audio => "aac".into(),
            StreamKind::Other => "data".into(),
        },
        time_base: TimeBase::new(1, 1_000).unwrap(),
        duration: Some(duration),
        start: None,
        frame_rate: (kind == StreamKind::Video).then_some(FrameRate::FPS_30),
        frame_count: None,
        variable_frame_rate: None,
        video: (kind == StreamKind::Video).then_some(VideoInfo {
            width: 1920,
            height: 1080,
            color: ColorInfo::default(),
            bit_depth: Some(8),
        }),
        audio: (kind == StreamKind::Audio).then_some(AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            initial_padding: 0,
            trailing_padding: 0,
        }),
    }
}

fn source_asset(
    id: u64,
    kind: StreamKind,
    duration: Option<Tick>,
    path: impl Into<PathBuf>,
) -> SourceAsset {
    SourceAsset::try_from(AssetData {
        id: AssetId(id),
        path: path.into(),
        content_hash: ContentHash::from_bytes(&[id as u8; 8]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration,
            streams: vec![stream(kind, duration.unwrap_or(Tick::ZERO))],
        },
    })
    .unwrap()
}

#[derive(Default)]
struct Probe {
    delay: Mutex<Duration>,
    calls: Mutex<Vec<PathBuf>>,
}
impl Probe {
    fn slow(delay: Duration) -> Self {
        Self {
            delay: Mutex::new(delay),
            calls: Mutex::new(Vec::new()),
        }
    }
}
impl ImportProbe for Probe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<SourceAsset> {
        self.calls.lock().unwrap().push(path.clone());
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.contains("bad") {
            return Err(MediaError::Unsupported("test codec"));
        }
        if name.contains("noduration") {
            return Ok(source_asset(id.0, StreamKind::Video, None, path));
        }
        let kind = if name.contains("audio") {
            StreamKind::Audio
        } else {
            StreamKind::Video
        };
        Ok(source_asset(id.0, kind, Some(Tick(4 * SECOND)), path))
    }
}

/// Blocks the second file until the test releases it, so the overlay's cancel
/// can be observed while the batch is genuinely still running.
struct GateProbe {
    calls: AtomicUsize,
    second: (Mutex<bool>, Condvar),
    release: (Mutex<bool>, Condvar),
}
impl GateProbe {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            second: (Mutex::new(false), Condvar::new()),
            release: (Mutex::new(false), Condvar::new()),
        }
    }
    fn second_started(&self) -> bool {
        *self.second.0.lock().unwrap()
    }
    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}
impl ImportProbe for GateProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<SourceAsset> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            *self.second.0.lock().unwrap() = true;
            self.second.1.notify_all();
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
        }
        Ok(source_asset(
            id.0,
            StreamKind::Video,
            Some(Tick(4 * SECOND)),
            path,
        ))
    }
}

/// Production-style adapter: checks the cooperative token while "working" and
/// returns Cancelled, so the batch can stop without external release.
struct BlockingProbe {
    started: AtomicBool,
}
impl BlockingProbe {
    fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
        }
    }
    fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
}
impl ImportProbe for BlockingProbe {
    fn import(
        &self,
        _id: AssetId,
        _path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<SourceAsset> {
        self.started.store(true, Ordering::Release);
        while !cancelled.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(MediaError::Cancelled)
    }
}

fn editor() -> Editor {
    let session =
        Session::new(Project::empty(ProjectId(13), "Import".into()).unwrap(), 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}

fn wait_import(editor: &mut Editor) -> vibecut::Inspection {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.tick();
        if !editor.importing() {
            break;
        }
        assert!(Instant::now() < deadline, "import did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    editor.inspect().unwrap()
}

fn wait_harness_import(harness: &mut Harness<'_, Editor>) -> vibecut::Inspection {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(1);
        if !harness.state().importing() {
            break;
        }
        assert!(Instant::now() < deadline, "import did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.state().inspect().unwrap()
}

fn widget<'a>(harness: &'a Harness<'_, Editor>, id: &'a str) -> egui_kittest::Node<'a> {
    harness.get(By::new().predicate(|node| node.author_id() == Some(id)))
}

fn drop_file(path: impl Into<PathBuf>) -> Arc<dyn egui::DroppedFile + Send + Sync> {
    #[derive(Debug)]
    struct Dropped(PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }
    Arc::new(Dropped(path.into()))
}

#[test]
fn drag_drop_imports_good_files_and_reports_typed_partial_failure() {
    let mut editor = editor();
    editor.set_import_probe(Arc::new(Probe::default())).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(2);
    for name in ["video-a.mp4", "bad-clip.mp4", "audio-a.wav"] {
        harness
            .input_mut()
            .dropped_files
            .push(drop_file(format!("/tmp/vibecut-import/{name}")));
    }
    let inspection = wait_harness_import(&mut harness);
    assert_eq!(inspection.import_total, 3);
    assert_eq!(inspection.import_completed, 3);
    assert_eq!(inspection.import_imported, 2);
    assert_eq!(inspection.import_failures.len(), 1);
    assert!(inspection.import_failures[0].contains("bad-clip.mp4"));
    assert!(inspection.import_failures[0].contains("test codec"));
    assert!(!inspection.import_cancelled);
    let toast = inspection.import_toast.unwrap();
    assert!(toast.contains('2') && toast.contains('1'), "{toast}");
    let assets = &harness.state().session.project().data().assets;
    assert_eq!(assets.len(), 2);
    assert_eq!(assets[0].id(), AssetId(1));
    assert_eq!(assets[1].id(), AssetId(2));
    assert_eq!(assets[0].probe().streams[0].kind, StreamKind::Video);
    assert_eq!(assets[1].probe().streams[0].kind, StreamKind::Audio);
    assert!(
        inspection.import_notice.is_none(),
        "partial failure is not an insert/preview failure"
    );
}

#[test]
fn cancel_mid_batch_keeps_completed_assets_and_stops_the_rest() {
    let mut editor = editor();
    editor
        .set_import_probe(Arc::new(Probe::slow(Duration::from_millis(60))))
        .unwrap();
    editor
        .drop_files(
            (1..=4)
                .map(|index| PathBuf::from(format!("/tmp/vibecut-import/slow-{index}.mp4")))
                .collect(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.tick();
        if editor.inspect().unwrap().import_imported >= 1 {
            break;
        }
        assert!(Instant::now() < deadline, "first asset never imported");
        std::thread::sleep(Duration::from_millis(2));
    }
    editor.activate("import.cancel").unwrap();
    let inspection = wait_import(&mut editor);
    assert!(inspection.import_cancelled);
    assert!(inspection.import_imported >= 1);
    assert!(
        inspection.import_imported < 4,
        "cancel must stop later files: {inspection:?}"
    );
    assert_eq!(
        editor.session.project().data().assets.len(),
        inspection.import_imported,
        "every reported import is retained"
    );
    assert!(inspection.import_toast.unwrap().contains("cancelled"));
}

#[test]
fn import_batches_are_capped_and_require_shared_budget_admission() {
    let budget = ByteBudget::new(512 << 20).unwrap();
    let mut queue = ImportQueue::new(budget).unwrap();
    let oversized: Vec<PathBuf> = (0..=MAX_IMPORT_FILES)
        .map(|index| PathBuf::from(format!("/tmp/vibecut-import/many-{index}.mp4")))
        .collect();
    assert!(matches!(
        queue.start(oversized, 1),
        Err(Error::Invalid("import batch limit"))
    ));
    assert!(matches!(
        queue.start(vec![PathBuf::from("x".repeat(32 * 1024 + 1))], 1),
        Err(Error::Invalid("import path limit"))
    ));
    let tiny = ByteBudget::new(1).unwrap();
    let mut tiny_queue = ImportQueue::new(tiny.clone()).unwrap();
    assert!(matches!(
        tiny_queue.start(vec!["/tmp/review-1.mp4".into()], 1),
        Err(Error::Engine(engine::EditError::ArtifactTooLarge))
    ));
    assert_eq!(tiny.snapshot().unwrap().used, 0);
    assert_eq!(tiny.snapshot().unwrap().limit, 1);
}

#[test]
fn unpolled_import_backpressure_bounds_completed_metadata() {
    let budget = ByteBudget::new(512 << 20).unwrap();
    let mut queue = ImportQueue::new(budget.clone()).unwrap();
    let probe = Arc::new(Probe::default());
    queue.set_probe(probe.clone());
    queue
        .start(
            (0..32)
                .map(|index| format!("/tmp/vibecut-import/backpressure-{index}.mp4").into())
                .collect(),
            1,
        )
        .unwrap();
    let _ = queue.poll().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let produced = probe.calls.lock().unwrap().len();
    assert!(
        produced <= 10,
        "an unpolled batch must block on its bounded event channel, produced {produced}"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let events = queue.poll().unwrap();
        let done = events
            .iter()
            .any(|event| matches!(event, ImportEvent::Done { .. }));
        if done {
            break;
        }
        assert!(Instant::now() < deadline, "drain did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!queue.active());
    assert_eq!(budget.snapshot().unwrap().used, 0, "work lease released");
}

#[test]
fn cancel_interrupts_the_current_probe_and_shutdown_joins_it() {
    let mut editor = editor();
    let probe = Arc::new(BlockingProbe::new());
    editor.set_import_probe(probe.clone()).unwrap();
    editor
        .start_import(vec![
            "/tmp/vibecut-import/cancel-a.mp4".into(),
            "/tmp/vibecut-import/cancel-b.mp4".into(),
        ])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !probe.started() {
        editor.tick();
        assert!(Instant::now() < deadline, "probe never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    let started = Instant::now();
    editor.activate("import.cancel").unwrap();
    let inspection = wait_import(&mut editor);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancel waits for the whole probe: {:?}",
        started.elapsed()
    );
    assert!(inspection.import_cancelled);
    assert_eq!(inspection.import_imported, 0);
    assert_eq!(inspection.import_unfinished, 2);
    assert!(inspection.import_error.is_none());

    let mut second_editor = self::editor();
    let probe = Arc::new(BlockingProbe::new());
    second_editor.set_import_probe(probe.clone()).unwrap();
    second_editor
        .start_import(vec!["/tmp/vibecut-import/shutdown.mp4".into()])
        .unwrap();
    while !probe.started() {
        second_editor.tick();
        assert!(Instant::now() < deadline, "shutdown probe never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    let started = Instant::now();
    drop(second_editor);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "drop/close joins a cancelled probe: {:?}",
        started.elapsed()
    );
}

#[test]
fn worker_panic_is_a_typed_failure_not_complete() {
    struct PanicProbe;
    impl ImportProbe for PanicProbe {
        fn import(&self, _: AssetId, _: PathBuf, _: Arc<AtomicBool>) -> media::Result<SourceAsset> {
            panic!("review injected failure");
        }
    }
    let mut editor = editor();
    editor.set_import_probe(Arc::new(PanicProbe)).unwrap();
    editor
        .start_import(vec!["/tmp/vibecut-import/panic.mp4".into()])
        .unwrap();
    let inspection = wait_import(&mut editor);
    assert_eq!(inspection.import_completed, 0);
    assert_eq!(inspection.import_imported, 0);
    assert_eq!(inspection.import_failures.len(), 0);
    assert_eq!(inspection.import_unfinished, 1);
    assert!(matches!(
        inspection.import_error.as_deref(),
        Some(engine::EditError::WorkerPanicked)
    ));
    assert!(
        inspection
            .import_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("unwound"))
    );
    let toast = inspection.import_toast.unwrap();
    assert!(toast.contains("Import failed"), "{toast}");
    assert!(!toast.contains("Import complete"), "{toast}");
    assert!(toast.contains("Unfinished: 1"), "{toast}");
}

#[test]
fn import_overlay_disables_import_and_insert_and_cancel_stops_batch() {
    let mut editor = editor();
    editor
        .session
        .apply(Command::AddAsset {
            asset: source_asset(
                1,
                StreamKind::Video,
                Some(Tick(4 * SECOND)),
                "/tmp/vibecut-import/existing.mp4",
            ),
        })
        .unwrap();
    editor.select_media([AssetId(1)]);
    let probe = Arc::new(GateProbe::new());
    editor.set_import_probe(probe.clone()).unwrap();
    editor
        .drop_files(
            (1..=3)
                .map(|index| PathBuf::from(format!("/tmp/vibecut-import/gate-{index}.mp4")))
                .collect(),
        )
        .unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        harness.run_steps(1);
        if probe.second_started() && harness.state().inspect().unwrap().import_completed >= 1 {
            break;
        }
        assert!(Instant::now() < deadline, "import never progressed");
        std::thread::sleep(Duration::from_millis(2));
    }
    let inspection = harness.state().inspect().unwrap();
    assert!(inspection.importing);
    let widget_state = |id: &str| {
        inspection
            .widgets
            .iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing {id}"))
    };
    assert!(!widget_state("file.import").enabled);
    assert!(
        !widget_state("media.insert").enabled,
        "insert is disabled while the batch runs even with a selection"
    );
    assert!(widget_state("import.cancel").enabled);
    widget(&harness, "import.cancel").click();
    harness.run_steps(2);
    probe.release();
    let inspection = wait_harness_import(&mut harness);
    assert!(inspection.import_cancelled);
    assert_eq!(
        inspection.import_imported, 2,
        "the first file completed and the in-flight second file is retained"
    );
    assert_eq!(
        harness.state().session.project().data().assets.len(),
        inspection.import_imported + 1,
        "the pre-existing asset and completed imports survive"
    );
}

#[test]
fn multi_select_insert_targets_only_selected_assets_in_one_undo_step() {
    let mut session = Session::new(
        Project::empty(ProjectId(14), "Selection".into()).unwrap(),
        50,
    )
    .unwrap();
    for (id, kind) in [
        (1, StreamKind::Video),
        (2, StreamKind::Audio),
        (3, StreamKind::Video),
    ] {
        session
            .apply(Command::AddAsset {
                asset: source_asset(
                    id,
                    kind,
                    Some(Tick(4 * SECOND)),
                    format!("/tmp/vibecut-import/asset-{id}.mp4"),
                ),
            })
            .unwrap();
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(128 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let editor = Editor::new(session, playback, Preferences::default());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    widget(&harness, "media.asset.1").click();
    harness.run_steps(2);
    widget(&harness, "media.asset.2").click_modifiers(egui::Modifiers::COMMAND);
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().media_selection,
        vec![AssetId(1), AssetId(2)]
    );
    widget(&harness, "media.insert").click();
    harness.run_steps(3);
    let data = harness.state().session.project().data();
    let inserted: Vec<_> = data
        .timeline
        .tracks
        .iter()
        .flat_map(|track| track.items.iter())
        .map(|item| (item.source_asset.unwrap(), item.content.kind()))
        .collect();
    assert_eq!(inserted.len(), 2);
    assert!(inserted.contains(&(AssetId(1), TrackKind::Video)));
    assert!(inserted.contains(&(AssetId(2), TrackKind::Audio)));
    assert!(
        !inserted.iter().any(|(asset, _)| *asset == AssetId(3)),
        "unselected asset must stay out of the timeline"
    );
    for track in &data.timeline.tracks {
        assert_eq!(track.items.len(), 1);
        assert_eq!(track.items[0].start, Tick::ZERO);
        assert_eq!(
            track.items[0].end().unwrap(),
            Tick(4 * SECOND),
            "each selected clip lands on its compatible track without overlap"
        );
    }
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .session
            .project()
            .data()
            .timeline
            .tracks
            .is_empty(),
        "multi-asset insert is one undo step"
    );
    assert_eq!(
        harness.state().session.project().data().assets.len(),
        3,
        "undo restores the timeline without touching imported assets"
    );
}

#[test]
fn insert_refusal_keeps_assets_and_the_timeline_unchanged() {
    let mut session =
        Session::new(Project::empty(ProjectId(15), "Refusal".into()).unwrap(), 20).unwrap();
    session
        .apply(Command::AddAsset {
            asset: source_asset(
                1,
                StreamKind::Video,
                None,
                "/tmp/vibecut-import/noduration.mp4",
            ),
        })
        .unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.select_media([AssetId(1)]);
    editor.activate("media.insert").unwrap();
    let inspection = editor.inspect().unwrap();
    assert!(
        inspection
            .import_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("duration")),
        "{:?}",
        inspection.import_notice
    );
    let data = editor.session.project().data();
    assert_eq!(data.assets.len(), 1, "the imported asset stays available");
    assert!(data.timeline.tracks.is_empty(), "refused insert is atomic");
    assert_eq!(data.timeline.tracks.len(), 0);
}

#[test]
fn insert_refreshes_playback_with_the_new_project_snapshot() {
    let mut session =
        Session::new(Project::empty(ProjectId(16), "Refresh".into()).unwrap(), 20).unwrap();
    let overlay = session
        .apply(Command::AddTrack {
            name: "Overlay".into(),
            kind: TrackKind::Overlay,
        })
        .unwrap();
    let engine::CreatedId::Track(track) = overlay.created[0] else {
        panic!("track")
    };
    session
        .apply(Command::InsertItem {
            track,
            item: engine::NewItem {
                animations: Default::default(),
                source_asset: None,
                source_range: time::SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([255, 0, 0, 255]),
                }),
            },
        })
        .unwrap();
    session
        .apply(Command::AddAsset {
            asset: source_asset(
                1,
                StreamKind::Video,
                Some(Tick(4 * SECOND)),
                "/tmp/vibecut-import/refresh.mp4",
            ),
        })
        .unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(128 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.playback.poll().unwrap();
        if editor.playback.frame().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "overlay preview never produced a frame"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(editor.playback.frame().is_some());
    editor.select_media([AssetId(1)]);
    editor.activate("media.insert").unwrap();
    assert!(
        editor.playback.frame().is_none(),
        "insert fences the old generation before the next frame"
    );
    assert_eq!(
        editor.session.project().data().timeline.tracks.len(),
        2,
        "the inserted video gets its own compatible track"
    );
    assert_eq!(
        editor
            .session
            .project()
            .data()
            .timeline
            .tracks
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|item| item.source_asset == Some(AssetId(1)))
            .count(),
        1
    );
}

fn screenshot_dir() -> PathBuf {
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        PathBuf::from(target).join("screenshots")
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots")
    }
}

#[test]
fn media_import_screenshot_and_receipt() {
    let mut editor = editor();
    editor.set_import_probe(Arc::new(Probe::default())).unwrap();
    editor.preferences.locale = Locale::Vi;
    editor
        .drop_files(vec![
            PathBuf::from("/tmp/vibecut-import/video-a.mp4"),
            PathBuf::from("/tmp/vibecut-import/bad-clip.mp4"),
            PathBuf::from("/tmp/vibecut-import/audio-a.wav"),
        ])
        .unwrap();
    let inspection = wait_import(&mut editor);
    assert_eq!(inspection.import_imported, 2);
    editor.select_media([AssetId(1)]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(4);
    let inspection = harness.state().inspect().unwrap();
    let image = harness.render().unwrap();
    assert_eq!(image.width(), 1440);
    assert_eq!(image.height(), 900);
    let mut colors = std::collections::BTreeSet::new();
    for pixel in image.pixels().step_by(97) {
        colors.insert(pixel.0);
    }
    assert!(colors.len() > 32, "screenshot must show the drawn editor");
    let widgets: Vec<serde_json::Value> = inspection
        .widgets
        .iter()
        .map(|widget| {
            serde_json::json!({
                "id": widget.id,
                "enabled": widget.enabled,
                "rect": [
                    widget.rect.min.x,
                    widget.rect.min.y,
                    widget.rect.max.x,
                    widget.rect.max.y
                ],
            })
        })
        .collect();
    let receipt = serde_json::json!({
        "name": "p0-13-import",
        "locale": "vi",
        "theme": "dark",
        "importing": inspection.importing,
        "import_total": inspection.import_total,
        "import_imported": inspection.import_imported,
        "import_failures": inspection.import_failures,
        "import_toast": inspection.import_toast,
        "media_selection": inspection.media_selection.iter().map(|id| id.0).collect::<Vec<_>>(),
        "widgets": widgets,
    });
    let dir = screenshot_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let image_path = dir.join("p0-13-import.png");
    let receipt_path = dir.join("p0-13-import.json");
    image.save(&image_path).unwrap();
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    assert!(image_path.is_file());
    assert!(receipt_path.is_file());
    println!(
        "media import screenshot: {} {}",
        image_path.display(),
        receipt_path.display()
    );
}
