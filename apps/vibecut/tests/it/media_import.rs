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
    AssetData, AssetId, ContentHash, ImportedSource, MediaError, ProbeCapabilities, SourceAsset,
    SourceStream, StreamKind,
};
use project::{ItemParams, Project, ProjectId, TrackKind};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{FrameRate, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, Error, egui,
    import::{ImportEvent, ImportProbe, ImportQueue, MAX_IMPORT_FILES},
    preferences::Preferences,
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

fn path(name: impl std::fmt::Display) -> PathBuf {
    std::env::temp_dir()
        .join("vibecut-import")
        .join(name.to_string())
}

fn stream(kind: StreamKind, duration: Tick) -> SourceStream {
    testkit::source::stream(kind).duration(duration).build()
}

fn source_asset(
    id: u64,
    kind: StreamKind,
    duration: Option<Tick>,
    path: impl Into<PathBuf>,
) -> SourceAsset {
    let path: PathBuf = path.into();
    let mut builder = testkit::source::asset(id, std::path::absolute(path).unwrap())
        .format("mov,mp4")
        .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
        .stream(stream(kind, duration.unwrap_or(Tick::ZERO)));
    if let Some(duration) = duration {
        builder = builder.duration(duration);
    }
    builder.build()
}

fn imported(asset: SourceAsset) -> ImportedSource {
    let streams: Vec<_> = asset
        .probe()
        .streams
        .iter()
        .filter(|stream| stream.kind != StreamKind::Other)
        .map(|stream| (stream.index, stream.kind, stream.codec.clone()))
        .collect();
    ImportedSource {
        asset,
        capabilities: ProbeCapabilities::decodable(streams),
    }
}

#[derive(Default)]
struct Probe {
    calls: Mutex<Vec<PathBuf>>,
}
impl ImportProbe for Probe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        self.calls.lock().unwrap().push(path.clone());
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.contains("bad") {
            return Err(MediaError::Unsupported("test codec"));
        }
        if name.contains("noduration") {
            return Ok(imported(source_asset(id.0, StreamKind::Video, None, path)));
        }
        let kind = if name.contains("audio") {
            StreamKind::Audio
        } else {
            StreamKind::Video
        };
        let mut asset = AssetData::from(source_asset(id.0, kind, Some(Tick(4 * SECOND)), path));
        if name.contains("highfps") {
            asset.probe.streams[0].frame_rate = Some(FrameRate::FPS_60);
        }
        SourceAsset::try_from(asset).map(imported)
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
    ) -> media::Result<ImportedSource> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            *self.second.0.lock().unwrap() = true;
            self.second.1.notify_all();
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
        }
        Ok(imported(source_asset(
            id.0,
            StreamKind::Video,
            Some(Tick(4 * SECOND)),
            path,
        )))
    }
}

/// Reports fractions of one file's bytes, then waits for the test to pump the
/// runner, so observed job progress is deterministic.
struct FractionProbe {
    fractions: Vec<f32>,
    signal: mpsc::Sender<()>,
    step: Mutex<mpsc::Receiver<()>>,
}
impl ImportProbe for FractionProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        Ok(imported(source_asset(
            id.0,
            StreamKind::Audio,
            Some(Tick(4 * SECOND)),
            path,
        )))
    }
    fn import_cached(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
        _cached: &mut dyn FnMut(ContentHash) -> Option<media::SourceProbe>,
        progress: &mut dyn FnMut(u64, u64),
    ) -> media::Result<(ImportedSource, Option<media::Thumbnail>)> {
        // Let the file's initial job stage settle past the runner interval.
        std::thread::sleep(Duration::from_millis(30));
        for fraction in &self.fractions {
            progress((fraction * 1000.0) as u64, 1000);
            self.signal.send(()).unwrap();
            self.step.lock().unwrap().recv().unwrap();
        }
        Ok((self.import(id, path, cancelled)?, None))
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
    ) -> media::Result<ImportedSource> {
        self.started.store(true, Ordering::Release);
        while !cancelled.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(MediaError::Cancelled)
    }
}

/// Seeded records still enter through the real restoration/validation protocol.
fn validate_assets(editor: &mut Editor) {
    struct RecordedProbe(Vec<SourceAsset>);
    impl ImportProbe for RecordedProbe {
        fn import(
            &self,
            id: AssetId,
            path: PathBuf,
            _cancelled: Arc<AtomicBool>,
        ) -> media::Result<ImportedSource> {
            let asset = self
                .0
                .iter()
                .find(|asset| asset.id() == id && asset.path() == path)
                .ok_or(MediaError::Invalid("missing validation fixture"))?;
            Ok(imported(asset.clone()))
        }
    }
    let assets = editor.session.project().data().assets.clone();
    editor
        .set_import_probe(Arc::new(RecordedProbe(assets)))
        .unwrap();
    editor.tick();
    wait_import(editor);
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
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(2);
    for name in ["video-a.mp4", "bad-clip.mp4", "audio-a.wav"] {
        harness
            .input_mut()
            .dropped_files
            .push(drop_file(path(name)));
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
fn picker_result_imports_through_workspace_and_real_editor_widgets() {
    use vibecut::workspace::Workspace;
    let root = tempfile::tempdir().unwrap();
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy() {
        app.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    app.create().unwrap();
    while app.busy() {
        app.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    app.focused_editor_mut()
        .unwrap()
        .set_import_probe(Arc::new(Probe::default()))
        .unwrap();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, app: &mut Workspace| {
                app.tick();
                if let Some(id) = app.focused_project() {
                    app.window_ui(id, ui);
                } else {
                    app.ui(ui);
                }
            },
            app,
        );
    harness.run_steps(2);
    // Inject the OS picker's returned paths at the production result seam.
    harness
        .state_mut()
        .focused_window_mut()
        .unwrap()
        .import_picked_files(vec![path("picker-video.mp4"), path("picker-bad.mp4")])
        .unwrap();
    loop {
        harness.run_steps(1);
        if !harness.state().focused_editor().unwrap().importing() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let inspection = harness.state().focused_editor().unwrap().inspect().unwrap();
    assert_eq!(inspection.import_imported, 1);
    assert_eq!(inspection.import_failures.len(), 1);
    assert_eq!(inspection.import_unfinished, 0);
    assert!(inspection.import_failures[0].contains("test codec"));
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("media.asset.1")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("media.insert")))
        .click();
    harness.run_steps(2);
    let data = harness
        .state()
        .focused_editor()
        .unwrap()
        .session
        .project()
        .data();
    assert_eq!(data.assets.len(), 1);
    assert_eq!(
        data.scene().timeline.main.items[0].source_asset,
        Some(AssetId(1))
    );
}

#[test]
fn cancel_mid_batch_keeps_completed_assets_and_stops_the_rest() {
    let mut editor = editor();
    let probe = Arc::new(GateProbe::new());
    editor.set_import_probe(probe.clone()).unwrap();
    editor
        .drop_files(
            (1..=4)
                .map(|index| path(format!("slow-{index}.mp4")))
                .collect(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.tick();
        if probe.second_started() && editor.inspect().unwrap().import_imported >= 1 {
            break;
        }
        assert!(Instant::now() < deadline, "first asset never imported");
        std::thread::sleep(Duration::from_millis(2));
    }
    editor.activate("import.cancel").unwrap();
    probe.release();
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
        .map(|index| path(format!("many-{index}.mp4")))
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
fn probe_job_progress_advances_within_each_file() {
    // A hashed file counts 0.99 of its share: probing still follows the hash,
    // so the batch shows 100% only once it is done.
    for (files, expected) in [
        (1usize, vec![0.495f32, 0.99]),
        (
            3,
            vec![
                0.495 / 3.0,
                0.99 / 3.0,
                1.495 / 3.0,
                1.99 / 3.0,
                2.495 / 3.0,
                2.99 / 3.0,
            ],
        ),
    ] {
        let budget = ByteBudget::new(512 << 20).unwrap();
        let mut queue = ImportQueue::new(budget).unwrap();
        let (signal, signals) = mpsc::channel();
        let (step, steps) = mpsc::channel();
        queue.set_probe(Arc::new(FractionProbe {
            fractions: vec![0.5, 1.0],
            signal,
            step: Mutex::new(steps),
        }));
        queue
            .start(
                (0..files)
                    .map(|index| path(format!("fraction-{index}.wav")))
                    .collect(),
                1,
            )
            .unwrap();
        // Submit admits the probe; the first tick starts it.
        queue.poll().unwrap();
        let mut observed = Vec::new();
        for _ in 0..files * 2 {
            signals
                .recv_timeout(Duration::from_secs(5))
                .expect("probe fraction");
            std::thread::sleep(Duration::from_millis(30));
            let events = queue.poll().unwrap();
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, ImportEvent::Done { .. })),
                "batch ended before its last fraction"
            );
            observed.push(queue.progress().expect("probe progress mid-batch"));
            step.send(()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = queue.poll().unwrap();
            if events
                .iter()
                .any(|event| matches!(event, ImportEvent::Done { .. }))
            {
                break;
            }
            assert!(Instant::now() < deadline, "fraction batch did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            observed.windows(2).all(|pair| pair[0] <= pair[1]),
            "{observed:?}"
        );
        for (actual, expected) in observed.iter().zip(&expected) {
            assert!(
                (actual - expected).abs() < 1e-3,
                "{files} files: observed {observed:?}, expected {expected:?}"
            );
        }
        assert!(*observed.last().unwrap() < 0.9999, "{observed:?}");
    }
}

#[test]
fn unpolled_import_backpressure_bounds_completed_metadata() {
    let budget = ByteBudget::new(512 << 20).unwrap();
    let mut queue = ImportQueue::new(budget.clone()).unwrap();
    let probe = Arc::new(GateProbe::new());
    queue.set_probe(probe.clone());
    queue
        .start(
            (0..32)
                .map(|index| path(format!("backpressure-{index}.mp4")))
                .collect(),
            1,
        )
        .unwrap();
    let _ = queue.poll().unwrap();
    // The initial poll starts the worker and can consume its first event.
    // Release the second probe only afterwards so the bound excludes racing
    // production and draining during that poll.
    probe.release();
    std::thread::sleep(Duration::from_millis(100));
    let produced = probe.calls.load(Ordering::SeqCst);
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
        .start_import(vec![path("cancel-a.mp4"), path("cancel-b.mp4")])
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
        .start_import(vec![path("shutdown.mp4")])
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
fn cancel_before_worker_starts_reports_cancellation_without_failure() {
    let mut editor = editor();
    editor.set_import_probe(Arc::new(Probe::default())).unwrap();
    editor
        .start_import(vec![path("cancel-before-start.mp4")])
        .unwrap();
    editor.activate("import.cancel").unwrap();
    let inspection = wait_import(&mut editor);
    assert!(inspection.import_cancelled);
    assert_eq!(inspection.import_unfinished, 1);
    assert!(inspection.import_error.is_none());
    let toast = inspection.import_toast.unwrap();
    assert!(toast.contains("cancelled"), "{toast}");
    assert!(!toast.contains("Import failed"), "{toast}");
}

#[test]
fn shutdown_keeps_work_allowance_until_probe_exits() {
    use std::sync::mpsc;
    struct ExitingProbe {
        started: mpsc::SyncSender<()>,
        cancelled: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl ImportProbe for ExitingProbe {
        fn import(
            &self,
            _: AssetId,
            _: PathBuf,
            cancelled: Arc<AtomicBool>,
        ) -> media::Result<ImportedSource> {
            self.started.send(()).unwrap();
            while !cancelled.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.cancelled.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Err(MediaError::Cancelled)
        }
    }
    let (started_send, started_receive) = mpsc::sync_channel(1);
    let (cancelled_send, cancelled_receive) = mpsc::sync_channel(1);
    let (release_send, release_receive) = mpsc::sync_channel(1);
    let budget = ByteBudget::new(512 << 20).unwrap();
    let mut queue = ImportQueue::new(budget.clone()).unwrap();
    queue.set_probe(Arc::new(ExitingProbe {
        started: started_send,
        cancelled: cancelled_send,
        release: Mutex::new(release_receive),
    }));
    queue.start(vec![path("shutdown-lease.mp4")], 1).unwrap();
    queue.poll().unwrap();
    started_receive
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let shutdown = std::thread::spawn(move || drop(queue));
    cancelled_receive
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let during_shutdown = budget.snapshot().unwrap().used;
    release_send.send(()).unwrap();
    shutdown.join().unwrap();
    assert!(
        during_shutdown >= 256 << 20,
        "probe still running but allowance was released: {during_shutdown}"
    );
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn worker_panic_is_a_typed_failure_not_complete() {
    struct PanicProbe;
    impl ImportProbe for PanicProbe {
        fn import(
            &self,
            _: AssetId,
            _: PathBuf,
            _: Arc<AtomicBool>,
        ) -> media::Result<ImportedSource> {
            panic!("review injected failure");
        }
    }
    let mut editor = editor();
    editor.set_import_probe(Arc::new(PanicProbe)).unwrap();
    editor.start_import(vec![path("panic.mp4")]).unwrap();
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
                path("existing.mp4"),
            ),
        })
        .unwrap();
    validate_assets(&mut editor);
    editor.select_media([AssetId(1)]);
    let probe = Arc::new(GateProbe::new());
    editor.set_import_probe(probe.clone()).unwrap();
    editor
        .drop_files(
            (1..=3)
                .map(|index| path(format!("gate-{index}.mp4")))
                .collect(),
        )
        .unwrap();
    let mut harness = crate::gpu::builder()
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
        (4, StreamKind::Video),
    ] {
        session
            .apply(Command::AddAsset {
                asset: source_asset(
                    id,
                    kind,
                    Some(Tick(4 * SECOND)),
                    path(format!("asset-{id}.mp4")),
                ),
            })
            .unwrap();
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    validate_assets(&mut editor);
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    widget(&harness, "media.asset.1").click();
    harness.run_steps(2);
    widget(&harness, "media.asset.2").click_modifiers(egui::Modifiers::COMMAND);
    harness.run_steps(2);
    widget(&harness, "media.asset.4").click_modifiers(egui::Modifiers::COMMAND);
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().media_selection,
        vec![AssetId(1), AssetId(2), AssetId(4)]
    );
    widget(&harness, "media.insert").click();
    harness.run_steps(3);
    let data = harness.state().session.project().data();
    let inserted: Vec<_> = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .map(|item| (item.source_asset.unwrap(), item.content.kind()))
        .collect();
    assert_eq!(inserted.len(), 3);
    assert!(inserted.contains(&(AssetId(1), TrackKind::Video)));
    assert!(inserted.contains(&(AssetId(2), TrackKind::Audio)));
    assert!(
        !inserted.iter().any(|(asset, _)| *asset == AssetId(3)),
        "unselected asset must stay out of the timeline"
    );
    assert!(data.scene().timeline.overlays.is_empty());
    assert_eq!(
        data.scene()
            .timeline
            .main
            .items
            .iter()
            .map(|i| (i.source_asset, i.start))
            .collect::<Vec<_>>(),
        vec![
            (Some(AssetId(1)), Tick::ZERO),
            (Some(AssetId(4)), Tick(4 * SECOND))
        ]
    );
    assert_eq!(data.scene().timeline.audio[0].items[0].start, Tick::ZERO);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .is_empty(),
        "multi-asset insert is one undo step"
    );
    assert_eq!(
        harness.state().session.project().data().assets.len(),
        4,
        "undo restores the timeline without touching imported assets"
    );
}

#[test]
fn insert_refusal_keeps_assets_and_the_timeline_unchanged() {
    let mut session =
        Session::new(Project::empty(ProjectId(15), "Refusal".into()).unwrap(), 20).unwrap();
    session
        .apply(Command::AddAsset {
            asset: source_asset(1, StreamKind::Video, None, path("noduration.mp4")),
        })
        .unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    validate_assets(&mut editor);
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
    assert!(
        data.scene().timeline.overlays.is_empty(),
        "refused insert is atomic"
    );
    assert_eq!(data.scene().timeline.overlays.len(), 0);
}

#[test]
fn import_and_insert_refresh_playback_with_the_new_project_snapshot() {
    for importing in [false, true] {
        let mut session =
            Session::new(Project::empty(ProjectId(16), "Refresh".into()).unwrap(), 20).unwrap();
        let overlay = session
            .apply(Command::AddTrack {
                placement: engine::TrackPlacement::Auto,
                name: "Overlay".into(),
                kind: TrackKind::Video,
            })
            .unwrap();
        let engine::CreatedId::Track(track) = overlay.created[0] else {
            panic!("track")
        };
        session
            .apply(Command::InsertItem {
                placement: engine::Placement::Track { track },
                item: engine::NewItem {
                    retime: Default::default(),
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
            ByteBudget::new(512 << 20).unwrap(),
            Arc::new(Files),
        )
        .unwrap();
        playback.set_device_available(true);
        let mut editor = Editor::new(session, playback, Preferences::default());
        validate_assets(&mut editor);
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
        if importing {
            let old_epoch = editor.playback.frame().unwrap().epoch;
            editor.set_import_probe(Arc::new(Probe::default())).unwrap();
            editor
                .start_import(vec!["/tmp/vibecut-import/highfps.mp4".into()])
                .unwrap();
            assert_eq!(wait_import(&mut editor).import_imported, 1);
            assert_eq!(
                editor.session.project().data().canvas.fps,
                FrameRate::FPS_60
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                editor.playback.poll().unwrap();
                if let Some(frame) = editor.playback.frame() {
                    assert!(
                        frame.epoch > old_epoch,
                        "import fences the old FPS generation"
                    );
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "imported project preview never produced a frame"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            continue;
        }
        editor.select_media([AssetId(1)]);
        editor.activate("media.insert").unwrap();
        assert!(
            editor.playback.frame().is_none(),
            "insert fences the old generation before the next frame"
        );
        assert_eq!(
            editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .track_count(),
            2,
            "the inserted video gets its own compatible track"
        );
        assert_eq!(
            editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| track.items.iter())
                .filter(|item| item.source_asset == Some(AssetId(1)))
                .count(),
            1
        );
    }
}

#[test]
fn ephemeral_assets_are_hidden_and_cannot_enter_library_selection() {
    let mut session = Session::new(
        Project::empty(ProjectId(15), "Internal".into()).unwrap(),
        10,
    )
    .unwrap();
    for id in [1, 2] {
        let mut asset = AssetData::from(source_asset(
            id,
            StreamKind::Video,
            Some(Tick(4 * SECOND)),
            format!("/tmp/vibecut-import/internal-{id}.mp4"),
        ));
        asset.ephemeral = id == 2;
        session
            .apply(Command::AddAsset {
                asset: SourceAsset::try_from(asset).unwrap(),
            })
            .unwrap();
    }
    let playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut editor = Editor::new(session, playback, Preferences::default());
    validate_assets(&mut editor);
    editor.select_media([AssetId(2)]);
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    assert!(
        harness
            .query(By::new().predicate(|node| node.author_id() == Some("media.asset.1")))
            .is_some()
    );
    assert!(
        harness
            .query(By::new().predicate(|node| node.author_id() == Some("media.asset.2")))
            .is_none()
    );
    assert_eq!(harness.state().session.project().data().assets.len(), 2);
    // Internal IDs still reserve identities for later public imports.
    harness
        .state_mut()
        .set_import_probe(Arc::new(Probe::default()))
        .unwrap();
    harness
        .state_mut()
        .start_import(vec!["/tmp/vibecut-import/visible.mp4".into()])
        .unwrap();
    let imported = wait_harness_import(&mut harness);
    assert_eq!(imported.import_imported, 1);
    assert!(
        harness
            .state()
            .session
            .project()
            .data()
            .assets
            .iter()
            .any(|asset| asset.id() == AssetId(3))
    );
    // UI refresh prunes the hidden source from the public selection entry point.
    widget(&harness, "media.asset.1").click();
    harness.run_steps(2);
    widget(&harness, "media.insert").click();
    harness.run_steps(2);
    let items = &harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .main
        .items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source_asset, Some(AssetId(1)));
}
