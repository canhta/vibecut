#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use media::{
    AssetData, AssetId, ContentHash, SourceAsset, SourceProbe, SourceStream, StreamKind, TimeBase,
    VideoInfo,
};
use project::{ItemParams, Project, ProjectId};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{Locale, PreferenceStore, Preferences},
    egui,
    export::{ExportFormat, ExportOptions, ExportQuality},
    playback::{Files, Playback, Sources},
};

fn temp_target(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}
fn project(empty: bool, source: bool) -> Project {
    let mut d = Project::empty(ProjectId(18), "Xuất video".into())
        .unwrap()
        .into_data();
    d.canvas.width = 64;
    d.canvas.height = 64;
    let range = SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND / 10)).unwrap();
    if source {
        d.assets.push(
            SourceAsset::try_from(AssetData {
                ephemeral: false,
                id: AssetId(1),
                path: temp_target("vibecut-export-source-missing.mp4"),
                content_hash: ContentHash::from_bytes(b"source"),
                probe: SourceProbe {
                    format: "mp4".into(),
                    duration: Some(range.duration()),
                    streams: vec![SourceStream {
                        index: 0,
                        kind: StreamKind::Video,
                        codec: "h264".into(),
                        time_base: TimeBase::new(1, 30).unwrap(),
                        duration: Some(range.duration()),
                        start: None,
                        frame_rate: Some(FrameRate::FPS_30),
                        frame_count: Some(3),
                        variable_frame_rate: Some(false),
                        video: Some(VideoInfo {
                            width: 64,
                            height: 64,
                            color: Default::default(),
                            bit_depth: Some(8),
                        }),
                        audio: None,
                    }],
                },
            })
            .unwrap(),
        );
    }
    if !empty {
        d.scene_mut().timeline.main.items.push(project::Item {
            blend_mode: Default::default(),
            retime: Default::default(),
            id: project::ItemId(1),
            source_asset: source.then_some(AssetId(1)),
            source_range: range,
            start: Tick::ZERO,
            content: if source {
                ItemParams::Video(project::VideoParams {
                    fades: Default::default(),
                    source_audio_enabled: false,
                    volume: 0.0,
                    muted: false,
                })
            } else {
                ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([100, 150, 200, 255]),
                })
            },
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
        });
    }
    Project::new(d).unwrap()
}
fn editor(empty: bool, source: bool) -> Editor {
    let session = Session::new(project(empty, source), 10).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(768 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Editor::new(session, playback, Preferences::default())
}
fn harness(editor: Editor) -> Harness<'static, Editor> {
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor)
}
fn click(h: &mut Harness<'_, Editor>, id: &str) {
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some(id)))
        .click();
    h.run_steps(3);
}
fn enabled(h: &Harness<'_, Editor>, id: &str) -> bool {
    h.state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .find(|w| w.id == id)
        .unwrap()
        .enabled
}
fn wait(h: &mut Harness<'_, Editor>) -> vibecut::export::ExportPhase {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run_steps(1);
        let inspect = h.state().export.as_ref().unwrap().inspect();
        if matches!(
            inspect.phase,
            vibecut::export::ExportPhase::Ready
                | vibecut::export::ExportPhase::Failed
                | vibecut::export::ExportPhase::Settings
        ) {
            return inspect.phase;
        }
        assert!(Instant::now() < deadline, "export stalled");
        std::thread::yield_now();
    }
}

struct Gate {
    entered: Mutex<mpsc::Sender<()>>,
    release: (Mutex<bool>, Condvar),
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
struct GateFactory(Arc<Gate>);
impl Sources for GateFactory {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Gated(self.0.clone()))
    }
}
struct Gated(Arc<Gate>);
impl render::SourceProvider for Gated {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        self.0.entered.lock().unwrap().send(()).unwrap();
        let mut release = self.0.release.0.lock().unwrap();
        while !*release {
            release = self.0.release.1.wait(release).unwrap();
        }
        if hooks.cancelled() {
            Err(render::Error::Cancelled)
        } else {
            Err(render::Error::Unsupported("injected source failure"))
        }
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        panic!("unexpected source request after injected refusal")
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: i64,
        _: usize,
        _: u32,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        panic!("unexpected source request after injected refusal")
    }
}
struct Restorable {
    restored: Arc<AtomicBool>,
    decoded: Arc<AtomicUsize>,
}
impl Sources for Restorable {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Self {
            restored: self.restored.clone(),
            decoded: self.decoded.clone(),
        })
    }
}
impl render::SourceProvider for Restorable {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok((!self.restored.load(Ordering::SeqCst)).then_some(render::UnavailableReason::Missing))
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        self.decoded.fetch_add(1, Ordering::SeqCst);
        render::Image::new(64, 64, [0.0, 0.0, 0.0, 1.0], hooks).map(Some)
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: i64,
        _: usize,
        _: u32,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        panic!("source audio is disabled in this fixture")
    }
}

static SCREENSHOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn export_choices_default_persist_and_empty_timeline_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    for locale in [Locale::En, Locale::Vi] {
        let mut e = editor(true, false);
        e.preferences.locale = locale;
        assert_eq!(e.preferences.export, ExportOptions::default());
        let mut h = harness(e);
        h.run_steps(2);
        click(&mut h, "file.export");
        let state = h.state().export.as_ref().unwrap().inspect();
        assert!(state.open && state.empty);
        assert!(!enabled(&h, "export.run"));
        click(&mut h, "export.format");
        click(&mut h, "export.format.webm");
        click(&mut h, "export.quality");
        click(&mut h, "export.quality.very_high");
        click(&mut h, "export.audio.include");
        assert_eq!(
            h.state().inspect().unwrap().export_options,
            ExportOptions {
                format: ExportFormat::Webm,
                quality: ExportQuality::VeryHigh,
                include_audio: false
            }
        );
        h.state_mut().activate("edit.undo").unwrap();
        assert!(h.state().preferences.export.include_audio);
        h.state_mut().activate("edit.redo").unwrap();
        assert!(!h.state().preferences.export.include_audio);
        store.save(&h.state().preferences).unwrap();
        assert_eq!(store.load().unwrap().export, h.state().preferences.export);
        click(&mut h, "export.close");
        click(&mut h, "file.export");
        assert_eq!(h.state().preferences.export.format, ExportFormat::Webm);
        assert!(matches!(
            h.state_mut()
                .export
                .as_mut()
                .unwrap()
                .start(Arc::new(project(true, false)), ExportOptions::default()),
            Err(vibecut::Error::EmptyTimeline)
        ));
    }
    // Unversioned documents are refused and retained, never silently upgraded.
    std::fs::write(
        store.path(),
        br#"{"locale":"en","dark":true,"panels":[0.25,0.25,0.4]}"#,
    )
    .unwrap();
    let previous = std::fs::read(store.path()).unwrap();
    assert!(matches!(
        store.load(),
        Err(vibecut::Error::Unsupported("preferences version"))
    ));
    assert_eq!(
        store.load_with_warning().1,
        Some(vibecut::catalog::PreferenceIssue::Version)
    );
    assert!(store.save(&Preferences::default()).is_err());
    assert_eq!(std::fs::read(store.path()).unwrap(), previous);
    for (quality, q, b) in [
        (ExportQuality::Low, 55, 3_000_000),
        (ExportQuality::Medium, 70, 6_000_000),
        (ExportQuality::High, 85, 12_000_000),
        (ExportQuality::VeryHigh, 95, 24_000_000),
    ] {
        let settings = ExportOptions {
            quality,
            ..Default::default()
        }
        .settings(&project(false, false));
        assert_eq!((settings.quality, settings.bit_rate), (q, b));
        assert!(matches!(settings.mode, render::ExportMode::Encode));
    }
}

fn restored_source(restored: bool) -> (Arc<Restorable>, Arc<std::sync::atomic::AtomicUsize>) {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(restored));
    let decoded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    (
        Arc::new(Restorable {
            restored: flag,
            decoded: decoded.clone(),
        }),
        decoded,
    )
}

#[test]
fn export_progress_then_save_writes_toast_and_closes_without_an_extra_screen() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("My Project.mp4");
    let mut h = harness(editor(false, false));
    h.run_steps(2);
    let project_before = h.state().session.project().clone();
    // The command must create the dialog with the shared preview/export render state.
    h.state_mut().activate("export.run").unwrap();
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    let state = h.state().export.as_ref().unwrap().inspect();
    assert!(state.finished.as_ref().is_some_and(|path| path.exists()));
    assert!(!state.open || state.phase == vibecut::export::ExportPhase::Ready);
    for id in ["export.retry", "export.download", "export.copy_file"] {
        assert!(
            !h.state()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|w| w.id == id)
        );
    }
    assert!(enabled(&h, "export.save"));
    assert!(Arc::ptr_eq(&project_before, h.state().session.project()));
    h.state_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut h);
    assert!(destination.exists());
    let state = h.state().export.as_ref().unwrap().inspect();
    assert!(!state.open);
    assert!(state.finished.is_none());
    assert!(h.state_mut().activate("export.retry").is_err());
    let toast = h
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("My Project.mp4"), "{toast}");
}

#[test]
fn export_failure_panel_copies_only_diagnostics_and_retry_keeps_settings() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("retried.webm");
    let mut h = harness(editor(false, true));
    h.run_steps(2);
    click(&mut h, "file.export");
    let (sources, _) = restored_source(false);
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    click(&mut h, "export.format");
    click(&mut h, "export.format.webm");
    click(&mut h, "export.quality");
    click(&mut h, "export.quality.very_high");
    click(&mut h, "export.audio.include");
    click(&mut h, "export.run");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Failed);
    let state = h.state().export.as_ref().unwrap().inspect();
    assert_eq!(
        state.attempt,
        Some(ExportOptions {
            format: ExportFormat::Webm,
            quality: ExportQuality::VeryHigh,
            include_audio: false,
        })
    );
    assert_eq!(state.unavailable.len(), 1);
    let diagnostics = state.diagnostics.clone().unwrap();
    // Settings may be edited while the failure panel is visible. Retry and its
    // eventual save dialog must still describe the completed attempt's container.
    h.state_mut().preferences.export.format = ExportFormat::Mp4;
    assert!(!diagnostics.is_empty());
    assert!(enabled(&h, "export.copy"));
    assert!(enabled(&h, "export.retry"));
    assert!(
        !h.state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == "export.run")
    );
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-51-export.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let _guard = SCREENSHOT_LOCK.lock().unwrap();
        h.render().unwrap().save(&path).unwrap();
    }
    h.get(By::new().predicate(|n| n.author_id() == Some("export.copy")))
        .click();
    h.run_steps(1);
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(
                command,
                egui::OutputCommand::CopyText(text) if text == &diagnostics
            ))
    );
    let (sources, _) = restored_source(true);
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    click(&mut h, "export.retry");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    assert_eq!(h.state().export_save_format(), ExportFormat::Webm);
    assert!(h.state().export_save_defaults().0.ends_with(".webm"));
    assert_eq!(
        h.state().export.as_ref().unwrap().inspect().attempt,
        Some(ExportOptions {
            format: ExportFormat::Webm,
            quality: ExportQuality::VeryHigh,
            include_audio: false,
        })
    );
    h.state_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut h);
    assert!(destination.exists());
}

#[test]
fn cancelling_export_returns_to_settings_and_leaves_no_final_file() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("cancelled.mp4");
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let mut h = harness(editor(false, true));
    let _release = Release(gate.clone());
    h.run_steps(2);
    click(&mut h, "file.export");
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(Arc::new(GateFactory(gate.clone())))
        .unwrap();
    click(&mut h, "export.run");
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(h.state().export.as_ref().unwrap().active());
    let running_job = h.state().export.as_ref().unwrap().inspect().job.unwrap().id;
    assert!(h.state_mut().activate("export.retry").is_err());
    assert_eq!(
        h.state().export.as_ref().unwrap().inspect().job.unwrap().id,
        running_job
    );
    click(&mut h, "export.cancel");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Settings);
    let state = h.state().export.as_ref().unwrap().inspect();
    assert!(state.open);
    assert!(state.finished.is_none());
    assert!(state.job.is_none());
    assert!(!destination.exists());
}

#[test]
fn missing_media_refusal_needs_consent_and_retry_reports_the_same_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("degraded.mp4");
    let mut h = harness(editor(false, true));
    h.run_steps(2);
    let project_before = h.state().session.project().clone();
    click(&mut h, "file.export");
    let (sources, _) = restored_source(false);
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    click(&mut h, "export.run");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Failed);
    let state = h.state().export.as_ref().unwrap().inspect();
    assert_eq!(state.unavailable.len(), 1);
    assert_eq!(state.unavailable[0].asset, AssetId(1));
    assert_eq!(
        state.unavailable[0].range,
        SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND / 10)).unwrap()
    );
    assert!(!state.consent);
    assert!(Arc::ptr_eq(&project_before, h.state().session.project()));
    click(&mut h, "export.consent");
    assert!(h.state().export.as_ref().unwrap().inspect().consent);
    click(&mut h, "export.retry");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    h.state_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut h);
    assert!(destination.exists());
    assert!(Arc::ptr_eq(&project_before, h.state().session.project()));
}

#[test]
fn cancelled_save_keeps_the_finished_export_for_a_later_save() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("second-save.mp4");
    let mut h = harness(editor(false, true));
    h.run_steps(2);
    click(&mut h, "file.export");
    let (sources, _) = restored_source(true);
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    click(&mut h, "export.run");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    h.state_mut().export_save_cancelled();
    let state = h.state().export.as_ref().unwrap().inspect();
    assert!(state.open);
    assert!(state.finished.as_ref().is_some_and(|path| path.exists()));
    let finished = state.finished.unwrap();
    assert!(h.state_mut().activate("export.retry").is_err());
    // A failed save must retain the finished output and leave existing data intact.
    let blocked = dir.path().join("directory.mp4");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("keep"), b"previous").unwrap();
    h.state_mut().export_picked_file(blocked.clone()).unwrap();
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Failed);
    // Let the modal settle after progress changes to the diagnostics layout.
    h.run_steps(3);
    assert!(enabled(&h, "export.copy"));
    assert!(enabled(&h, "export.retry"));
    let diagnostics = h
        .state()
        .export
        .as_ref()
        .unwrap()
        .inspect()
        .diagnostics
        .unwrap();
    h.get(By::new().predicate(|n| n.author_id() == Some("export.copy")))
        .click();
    h.run_steps(1);
    assert_eq!(h.output().platform_output.commands.iter().filter(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &diagnostics)).count(), 1);
    click(&mut h, "export.retry");
    assert_eq!(
        h.state()
            .export
            .as_ref()
            .unwrap()
            .inspect()
            .finished
            .as_ref(),
        Some(&finished)
    );
    assert_eq!(std::fs::read(blocked.join("keep")).unwrap(), b"previous");
    assert!(finished.exists());
    assert!(!destination.exists());
    std::fs::write(&destination, b"previous export").unwrap();
    h.state_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut h);
    assert!(destination.exists());
    assert_ne!(std::fs::read(&destination).unwrap(), b"previous export");
    assert!(!finished.exists());
}

#[test]
fn retry_reuses_settings_into_new_output_and_preserves_a_prior_file() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("prior.mp4");
    std::fs::write(&destination, b"previous").unwrap();
    let mut h = harness(editor(false, true));
    h.run_steps(2);
    click(&mut h, "file.export");
    let (sources, _) = restored_source(false);
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    click(&mut h, "export.run");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Failed);
    click(&mut h, "export.retry");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Failed);
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn filenames_are_sanitized_and_export_preferences_roundtrip() {
    assert_eq!(
        vibecut::export::default_file_name("My: Project?", "mp4"),
        "My_ Project_.mp4"
    );
    assert_eq!(
        vibecut::export::default_file_name("D\u{1ef1} \u{00e1}n phim", "webm"),
        "D\u{1ef1} \u{00e1}n phim.webm"
    );
    assert_eq!(
        vibecut::export::default_file_name("  spaced  ", "mp4"),
        "spaced.mp4"
    );
    assert_eq!(vibecut::export::default_file_name("CON", "mov"), "_CON.mov");
    assert_eq!(
        vibecut::export::default_file_name("LPT9.txt", "mp4"),
        "_LPT9.txt.mp4"
    );
    assert_eq!(
        vibecut::export::default_file_name("   ", "mp4"),
        "Untitled Project.mp4"
    );
    let options = ExportOptions {
        format: ExportFormat::Webm,
        quality: ExportQuality::VeryHigh,
        include_audio: false,
    };
    assert!(!options.settings(&project(false, false)).include_audio);
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    let preferences = vibecut::catalog::Preferences {
        export: options,
        export_directory: Some(dir.path().join("exports")),
        ..Default::default()
    };
    store.save(&preferences).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.export, options);
    assert_eq!(loaded.export_directory, preferences.export_directory);
}
#[test]
fn review_reproduction_save_refuses_to_replace_source_media() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("original.mp4");
    std::fs::write(&source_path, b"original source media").unwrap();
    let mut data = project(false, true).into_data();
    let mut asset: AssetData = data.assets.remove(0).into();
    asset.path = source_path.clone();
    asset.content_hash = ContentHash::from_bytes(b"original source media");
    data.assets.push(SourceAsset::try_from(asset).unwrap());
    let session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(768 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut h = harness(Editor::new(session, playback, Preferences::default()));
    h.run_steps(2);
    click(&mut h, "file.export");
    let (sources, _) = restored_source(true);
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    click(&mut h, "export.run");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    let before = serde_json::to_vec(h.state().session.project().as_ref()).unwrap();
    let finished = h
        .state()
        .export
        .as_ref()
        .unwrap()
        .inspect()
        .finished
        .unwrap();
    let alias = dir.path().join("alias.mp4");
    std::fs::hard_link(&source_path, &alias).unwrap();
    for destination in [
        source_path.clone(),
        dir.path().join(".").join("original.mp4"),
        alias,
    ] {
        h.state_mut().export_picked_file(destination).unwrap();
        assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Failed);
        let state = h.state().export.as_ref().unwrap().inspect();
        let error = state.job.unwrap().error;
        assert!(
            matches!(
                error.as_deref(),
                Some(engine::EditError::Render(render::Error::Invalid(
                    "output aliases a source"
                        | "output aliases a source by hard link"
                        | "output hard-links a source"
                        | "output may alias source content"
                )))
            ),
            "unexpected source-protection failure: {error:?}"
        );
        assert!(state.diagnostics.unwrap().contains("source"));
        assert_eq!(state.finished.as_ref(), Some(&finished));
        assert_eq!(
            serde_json::to_vec(h.state().session.project().as_ref()).unwrap(),
            before
        );
        let result = h.state().export.as_ref().unwrap().inspect().phase;
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            b"original source media",
            "saving an export replaced source media; save result: {result:?}"
        );
        assert_eq!(result, vibecut::export::ExportPhase::Failed);
        click(&mut h, "export.retry");
    }
}

#[test]
fn publication_keeps_ui_responsive_and_cancel_preserves_previous_file() {
    struct StorageGate(Arc<Gate>);
    impl project::atomic::AtomicFs for StorageGate {
        fn sync(&self, _: &std::fs::File) -> std::io::Result<()> {
            self.0.entered.lock().unwrap().send(()).unwrap();
            let mut release = self.0.release.0.lock().unwrap();
            while !*release {
                release = self.0.release.1.wait(release).unwrap();
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("prior.mp4");
    std::fs::write(&destination, b"previous").unwrap();
    let mut h = harness(editor(false, false));
    h.run_steps(2);
    click(&mut h, "file.export");
    click(&mut h, "export.run");
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    let finished = h
        .state()
        .export
        .as_ref()
        .unwrap()
        .inspect()
        .finished
        .unwrap();
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let _release = Release(gate.clone());
    h.state_mut()
        .export
        .as_mut()
        .unwrap()
        .set_publication_fs(Arc::new(StorageGate(gate.clone())))
        .unwrap();
    h.state_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    h.run_steps(1);
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    h.run_steps(3);
    assert!(enabled(&h, "export.cancel"));
    click(&mut h, "export.cancel");
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    assert!(finished.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
