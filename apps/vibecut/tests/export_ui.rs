#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session, jobs::JobState};
use media::{
    AssetData, AssetId, ContentHash, SourceAsset, SourceProbe, SourceStream, StreamKind, TimeBase,
    VideoInfo,
};
use project::{ItemParams, Project, ProjectId};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
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
                id: AssetId(1),
                path: PathBuf::from("/nonexistent-vibecut-export-source/video.mp4"),
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
        d.timeline.main.items.push(project::Item {
            id: project::ItemId(1),
            source_asset: source.then_some(AssetId(1)),
            source_range: range,
            start: Tick::ZERO,
            content: if source {
                ItemParams::Video(project::VideoParams {
                    source_audio_enabled: false,
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
fn wait(h: &mut Harness<'_, Editor>) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run_steps(1);
        let s = h
            .state()
            .export
            .as_ref()
            .unwrap()
            .inspect()
            .job
            .unwrap()
            .state;
        if matches!(
            s,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return s;
        }
        assert!(Instant::now() < deadline, "export stalled");
        std::thread::yield_now();
    }
}
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
        assert!(!enabled(&h, "export.start"));
        for id in [
            "export.format.mp4",
            "export.quality.high",
            "export.audio.include",
        ] {
            assert!(
                !h.state()
                    .inspect()
                    .unwrap()
                    .widgets
                    .iter()
                    .any(|w| w.id == id)
            );
        }
        click(&mut h, "export.format");
        click(&mut h, "export.format.webm");
        click(&mut h, "export.quality");
        click(&mut h, "export.quality.very_high");
        click(&mut h, "export.audio");
        click(&mut h, "export.audio.include");
        assert_eq!(
            h.state().inspect().unwrap().export_options,
            ExportOptions {
                format: ExportFormat::Webm,
                quality: ExportQuality::VeryHigh,
                include_audio: false
            }
        );
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
    // Existing preferences acquire the same defaults rather than a second store.
    std::fs::write(
        store.path(),
        br#"{"locale":"en","dark":true,"panels":[0.25,0.25,0.4]}"#,
    )
    .unwrap();
    assert_eq!(store.load().unwrap().export, ExportOptions::default());
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
        h: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        self.0.entered.lock().unwrap().send(()).unwrap();
        let mut release = self.0.release.0.lock().unwrap();
        while !*release {
            release = self.0.release.1.wait(release).unwrap();
        }
        if h.cancelled() {
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
#[test]
fn export_widget_start_cancel_and_failure_preserve_destination() {
    for cancel in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("existing.mp4");
        std::fs::write(&output, b"previous").unwrap();
        let (send, recv) = mpsc::channel();
        let gate = Arc::new(Gate {
            entered: Mutex::new(send),
            release: (Mutex::new(false), Condvar::new()),
        });
        let mut h = harness(editor(false, true));
        let _release = Release(gate.clone());
        h.run_steps(2);
        click(&mut h, "file.export");
        h.state_mut().export_picked_file(output.clone()).unwrap();
        h.state_mut()
            .export
            .as_mut()
            .unwrap()
            .set_sources(Arc::new(GateFactory(gate.clone())))
            .unwrap();
        h.run_steps(1);
        assert!(!enabled(&h, "export.start"));
        click(&mut h, "export.replace");
        assert!(
            h.state().export.as_ref().unwrap().inspect().replace,
            "replace unchecked"
        );
        assert!(enabled(&h, "export.start"), "start disabled");
        click(&mut h, "export.start");
        assert!(
            h.state().export.as_ref().unwrap().inspect().job.is_some(),
            "start not submitted: {:?}",
            h.state().export.as_ref().unwrap().inspect().error
        );
        recv.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(h.state().export.as_ref().unwrap().active());
        assert!(!enabled(&h, "export.destination"));
        assert!(!enabled(&h, "export.format"));
        if cancel {
            click(&mut h, "export.cancel");
        }
        *gate.release.0.lock().unwrap() = true;
        gate.release.1.notify_all();
        assert_eq!(
            wait(&mut h),
            if cancel {
                JobState::Cancelled
            } else {
                JobState::Failed
            }
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
// Availability changes without changing the immutable project or audio scope.
struct Restorable {
    restored: Arc<std::sync::atomic::AtomicBool>,
    decoded: Arc<std::sync::atomic::AtomicUsize>,
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
        Ok((!self.restored.load(std::sync::atomic::Ordering::SeqCst))
            .then_some(render::UnavailableReason::Missing))
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        self.decoded
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
#[test]
fn missing_media_needs_listed_consent_and_a_new_project_invalidates_it() {
    for restored_case in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("export.mp4");
        std::fs::write(&output, b"previous").unwrap();
        let restored = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let decoded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut h = harness(editor(false, true));
        h.run_steps(2);
        click(&mut h, "file.export");
        h.state_mut().export_picked_file(output.clone()).unwrap();
        if restored_case {
            h.state_mut()
                .export
                .as_mut()
                .unwrap()
                .set_sources(Arc::new(Restorable {
                    restored: restored.clone(),
                    decoded: decoded.clone(),
                }))
                .unwrap();
        }
        let snapshot = h.state().session.project().clone();
        h.run_steps(1);
        click(&mut h, "export.replace");
        assert!(h.state().export.as_ref().unwrap().inspect().replace);
        assert!(enabled(&h, "export.start"));
        click(&mut h, "export.start");
        assert_eq!(wait(&mut h), JobState::Failed);
        let state = h.state().export.as_ref().unwrap().inspect();
        assert_eq!(state.unavailable.len(), 1);
        assert_eq!(state.unavailable[0].asset, AssetId(1));
        assert_eq!(
            state.unavailable[0].range,
            SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND / 10)).unwrap()
        );
        assert!(!state.consent);
        assert_eq!(std::fs::read(&output).unwrap(), b"previous");
        // A strict retry must reach fresh admission even before any source is restored.
        assert!(
            enabled(&h, "export.start"),
            "missing-source diagnostics must not disable strict re-admission"
        );
        let previous_job = state.job.unwrap().id;
        click(&mut h, "export.start");
        assert_eq!(wait(&mut h), JobState::Failed);
        let state = h.state().export.as_ref().unwrap().inspect();
        assert_ne!(state.job.as_ref().unwrap().id, previous_job);
        assert!(matches!(
            state.job.unwrap().error.as_deref(),
            Some(engine::EditError::Render(render::Error::Unavailable(_)))
        ));
        assert!(!state.consent);
        assert_eq!(std::fs::read(&output).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        if restored_case {
            restored.store(true, std::sync::atomic::Ordering::SeqCst);
            h.run_steps(3);
            assert!(!h.state().export.as_ref().unwrap().inspect().consent);
            assert!(
                enabled(&h, "export.start"),
                "restored source has no non-degraded retry action"
            );
        } else {
            click(&mut h, "export.consent");
            assert!(h.state().export.as_ref().unwrap().inspect().consent);
        }
        click(&mut h, "export.start");
        assert_eq!(
            wait(&mut h),
            JobState::Completed,
            "{:?}",
            h.state()
                .export
                .as_ref()
                .unwrap()
                .inspect()
                .job
                .unwrap()
                .error
        );
        assert!(Arc::ptr_eq(&snapshot, h.state().session.project()));
        assert_ne!(std::fs::read(&output).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        if restored_case {
            let state = h.state().export.as_ref().unwrap().inspect();
            assert!(!state.consent);
            assert!(state.unavailable.is_empty());
            assert!(
                decoded.load(std::sync::atomic::Ordering::SeqCst) > 0,
                "restored source was still omitted"
            );
        } else {
            click(&mut h, "export.close");
            click(&mut h, "file.export");
            assert!(!h.state().export.as_ref().unwrap().inspect().consent);
            assert!(!h.state().export.as_ref().unwrap().inspect().replace);
            click(&mut h, "export.consent");
            assert!(h.state().export.as_ref().unwrap().inspect().consent);
            h.state_mut()
                .session
                .apply(engine::Command::RenameProject {
                    name: "Changed".into(),
                })
                .unwrap();
            h.run_steps(1);
            assert!(!h.state().export.as_ref().unwrap().inspect().consent);
        }
    }
}
