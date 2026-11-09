#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use media::{AssetData, AssetId, ContentHash, SourceAsset, TimeBase};
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
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, egui,
    export::{ExportFormat, ExportOptions, ExportQuality},
    i18n::Locale,
    playback::{Files, Playback, Sources},
    preferences::{PreferenceStore, Preferences},
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
            testkit::source::asset(1, temp_target("vibecut-export-source-missing.mp4"))
                .format("mp4")
                .duration(range.duration())
                .content_hash(ContentHash::from_bytes(b"source"))
                .stream(
                    testkit::source::video()
                        .time_base(TimeBase::new(1, 30).unwrap())
                        .duration(range.duration())
                        .frame_count(3)
                        .variable_frame_rate(false)
                        .size(64, 64)
                        .build(),
                )
                .build(),
        );
    }
    if !empty {
        d.scene_mut().timeline.main.items.push(project::Item {
            hidden: false,
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
                    crop: None,
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
    editor_project(project(empty, source))
}
fn editor_project(project: Project) -> Editor {
    let session = Session::new(project, 10).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(768 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Editor::new(session, playback, Preferences::default())
}
fn harness(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor)
}
fn click(h: &mut Harness<'_, Editor>, id: &str) {
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some(id)))
        .scroll_to_me();
    // Editor styling reinstalls scroll animation after harness setup. Let it
    // settle before the mouse coordinates for the next click are chosen.
    h.run_steps(20);
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
        .unwrap_or_else(|| panic!("missing widget {id}"))
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
                include_audio: false,
                ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
    for count in [1, 20] {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("degraded.mp4");
        let mut data = project(false, true).into_data();
        let first = data.scene().timeline.main.items[0].clone();
        for index in 1..count {
            let mut item = first.clone();
            item.id = project::ItemId(index + 1);
            item.start = Tick(first.source_range.duration().0 * index as i64);
            data.scene_mut().timeline.main.items.push(item);
        }
        let mut h = harness(editor_project(Project::new(data).unwrap()));
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
        assert_eq!(state.unavailable.len(), count as usize);
        assert_eq!(state.unavailable[0].asset, AssetId(1));
        assert_eq!(
            state.unavailable[0].range,
            SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND / 10)).unwrap()
        );
        assert!(!state.consent);
        assert!(Arc::ptr_eq(&project_before, h.state().session.project()));
        let retry = h
            .state()
            .inspect()
            .unwrap()
            .widgets
            .into_iter()
            .find(|widget| widget.id == "export.retry")
            .unwrap();
        assert!(
            retry.rect.bottom() <= 800.0,
            "failure footer clipped: {:?}",
            retry.rect
        );
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
}

#[test]
fn cancelled_save_keeps_the_finished_export_for_a_later_save() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("second-save.mp4");
    let mut data = project(false, true).into_data();
    let mut secondary = data.scene().clone();
    secondary.id = project::SceneId(2);
    secondary.is_main = false;
    secondary.name = "Second".into();
    for track in secondary.timeline.iter_mut() {
        track.id = project::TrackId(track.id.0 + 10);
        for item in &mut track.items {
            item.id = project::ItemId(item.id.0 + 10);
        }
    }
    data.scenes.insert(secondary).unwrap();
    let mut e = editor_project(Project::new(data).unwrap());
    e.preferences.export.output.scene = render::SceneChoice::All;
    let mut h = harness(e);
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
    assert!(h.state().export_save_defaults().0.ends_with("-scene-0.mp4"));
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
    assert_eq!(
        h.state().export.as_ref().unwrap().phase(),
        vibecut::export::ExportPhase::Ready
    );
    assert!(h.state().export_save_defaults().0.ends_with("-scene-2.mp4"));
    h.state_mut().export_save_cancelled();
    let second_finished = h
        .state()
        .export
        .as_ref()
        .unwrap()
        .inspect()
        .finished
        .unwrap();
    assert!(second_finished.exists());
    let second_destination = dir.path().join("second-scene.mp4");
    h.state_mut()
        .export_picked_file(second_destination.clone())
        .unwrap();
    wait(&mut h);
    assert!(second_destination.is_file());
    assert!(!second_finished.exists());
    assert!(!h.state().export.as_ref().unwrap().inspect().open);
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
    click(&mut h, "export.close");
    click(&mut h, "file.export");
    assert_eq!(
        h.state().export.as_ref().unwrap().phase(),
        vibecut::export::ExportPhase::Settings
    );
    assert!(enabled(&h, "export.run"));
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
        ..Default::default()
    };
    assert!(!options.settings(&project(false, false)).include_audio);
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    let preferences = vibecut::preferences::Preferences {
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
fn export_output_panel_persists_scene_range_refuses_empty_and_never_edits_project() {
    let mut h = harness(editor(false, false));
    let rate = h.state().session.project().data().canvas.fps;
    h.state_mut()
        .dispatch(
            "playback.seek",
            serde_json::json!({"time":rate.tick_of(1).unwrap().0}),
        )
        .unwrap();
    h.state_mut().activate("export.mark_in").unwrap();
    h.state_mut()
        .dispatch(
            "playback.seek",
            serde_json::json!({"time":rate.tick_of(2).unwrap().0}),
        )
        .unwrap();
    h.state_mut().activate("export.mark_out").unwrap();
    assert_eq!(
        h.state().preferences.export.output.range,
        render::RangeChoice::InOut {
            start: rate.tick_of(1).unwrap(),
            end: rate.tick_of(2).unwrap()
        }
    );
    let before = project::encode(h.state().session.project(), false).unwrap();
    click(&mut h, "file.export");
    click(&mut h, "export.resolution");
    click(&mut h, "export.resolution.720");
    assert_eq!(
        h.state().preferences.export.output.resolution,
        Some([720, 720])
    );
    click(&mut h, "export.custom.size");
    assert!(enabled(&h, "export.width"));
    for id in [
        "export.resolution",
        "export.fps",
        "export.codec",
        "export.profile",
        "export.scene",
        "export.range",
    ] {
        assert!(enabled(&h, id), "{id}");
    }
    click(&mut h, "export.range");
    assert!(!enabled(&h, "export.range.selected"));
    click(&mut h, "export.range.full");
    h.state_mut().selection.insert(project::ItemId(1));
    click(&mut h, "export.range");
    click(&mut h, "export.range.selected");
    assert!(matches!(
        h.state().preferences.export.output.range,
        render::RangeChoice::Selected { .. }
    ));
    click(&mut h, "export.scene");
    click(&mut h, "export.scene.all");
    click(&mut h, "export.range");
    click(&mut h, "export.range.in_out");
    assert!(enabled(&h, "export.range.in"));
    h.state_mut().preferences.export.output.range = render::RangeChoice::InOut {
        start: Tick::ZERO,
        end: Tick::ZERO,
    };
    h.run_steps(3);
    assert!(!enabled(&h, "export.run"));
    let invalid = h.state().preferences.export;
    assert!(
        h.state_mut()
            .export
            .as_mut()
            .unwrap()
            .start(Arc::new(project(false, false)), invalid)
            .is_err()
    );
    let fps = h.state().session.project().data().canvas.fps;
    h.state_mut().preferences.export.output.range = render::RangeChoice::InOut {
        start: fps.tick_of(1).unwrap(),
        end: fps.tick_of(2).unwrap(),
    };
    h.state_mut().preferences.export.output.resolution = Some([64, 64]);
    h.state_mut().preferences.export.output.bit_rate = Some(100_000);
    h.run_steps(3);
    assert!(enabled(&h, "export.run"));
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    store.save(&h.state().preferences).unwrap();
    assert_eq!(store.load().unwrap().export, h.state().preferences.export);
    let screenshot = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/p1-74-export-options.png");
    if let Some(parent) = screenshot.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    h.render().unwrap().save(&screenshot).unwrap();
    let options = h.state().preferences.export;
    h.state_mut()
        .dispatch("export.run", serde_json::to_value(options).unwrap())
        .unwrap();
    assert_eq!(wait(&mut h), vibecut::export::ExportPhase::Ready);
    assert_eq!(
        project::encode(h.state().session.project(), false).unwrap(),
        before
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.state_mut().activate("edit.redo").unwrap();
    assert_eq!(
        project::encode(h.state().session.project(), false).unwrap(),
        before
    );
}
#[test]
fn save_refuses_every_source_alias_without_replacing_media() {
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

#[test]
fn closing_one_window_preserves_another_windows_dirty_edit_and_running_export() {
    use vibecut::workspace::Workspace;
    fn idle(app: &mut Workspace) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.busy() {
            app.tick();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let root = tempfile::tempdir().unwrap();
    let first_path = root.path().join("first.vibecut");
    let second_path = root.path().join("second.vibecut");
    project::WriterLease::acquire(&first_path)
        .unwrap()
        .save(&Project::empty(ProjectId(1), "first".into()).unwrap())
        .unwrap();
    project::WriterLease::acquire(&second_path)
        .unwrap()
        .save(&project(false, true))
        .unwrap();
    let second_bytes = std::fs::read(&second_path).unwrap();
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    app.open(first_path.clone()).unwrap();
    idle(&mut app);
    app.open(second_path.clone()).unwrap();
    idle(&mut app);
    let first = ProjectId(1);
    let second = ProjectId(18);
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let release = Release(gate.clone());
    let editor = app.editor_mut(second).unwrap();
    editor
        .session
        .execute(
            "project.rename",
            serde_json::json!({"name":"unsaved second"}),
        )
        .unwrap();
    editor.activate("file.export").unwrap();
    editor
        .export
        .as_mut()
        .unwrap()
        .set_sources(Arc::new(GateFactory(gate.clone())))
        .unwrap();
    editor.activate("export.run").unwrap();
    let revision = editor.session.revision();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.tick();
        if recv.try_recv().is_ok() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let job = app
        .editor_mut(second)
        .unwrap()
        .export
        .as_ref()
        .unwrap()
        .inspect()
        .job
        .unwrap()
        .id;
    app.request_close();
    assert!(!app.home_open());
    assert!(!app.ready_to_close());
    app.focus(first).unwrap();
    app.windows.get_mut(&first).unwrap().request_close();
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            move |ui, a: &mut Workspace| {
                a.tick();
                if a.windows.contains_key(&first) {
                    a.window_ui(first, ui);
                }
            },
            app,
        );
    let _release = release;
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.close.cancel")))
        .click();
    h.run_steps(3);
    assert!(
        h.state_mut().editor_mut(first).is_some(),
        "cancel retains only the requested window"
    );
    h.state_mut()
        .windows
        .get_mut(&first)
        .unwrap()
        .request_close();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.close.recover")))
        .click();
    h.run_steps(2);
    idle(h.state_mut());
    assert!(!h.state().windows.contains_key(&first));
    assert!(project::WriterLease::acquire(&first_path).is_ok());
    let editor = h.state_mut().editor_mut(second).unwrap();
    assert_eq!(editor.session.revision(), revision);
    assert_eq!(
        editor.session.project().data().name.as_str(),
        "unsaved second"
    );
    assert!(editor.session.autosave().unwrap().is_dirty());
    assert!(editor.export.as_ref().unwrap().active());
    assert_eq!(
        editor.export.as_ref().unwrap().inspect().job.unwrap().id,
        job
    );
    assert!(matches!(
        project::WriterLease::acquire(&second_path),
        Err(project::Error::Busy)
    ));
    assert_eq!(std::fs::read(&second_path).unwrap(), second_bytes);
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    while h
        .state_mut()
        .editor_mut(second)
        .unwrap()
        .export
        .as_ref()
        .unwrap()
        .active()
    {
        h.state_mut().tick();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    h.state_mut()
        .windows
        .get_mut(&second)
        .unwrap()
        .persist(None, true, true, true)
        .unwrap();
    idle(h.state_mut());
    assert!(h.state().ready_to_close());
    assert!(project::WriterLease::acquire(&second_path).is_ok());
}
