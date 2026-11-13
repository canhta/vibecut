//! Drives the export dialog alone over the one-area bench.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{ExportFormat, ExportOptions, ExportQuality};
use crate::editor::context::bench::Bench;
use crate::editor::{AppCommand, Editor, egui};
use crate::preferences::{PreferenceStore, Preferences};
use crate::services::playback::{Files, Playback, Sources};
use crate::{Error, i18n::Locale};
use egui_kittest::kittest::Queryable;
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

const SECOND: i64 = TICKS_PER_SECOND;

fn temp_target(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}
fn project(empty: bool, source: bool) -> Project {
    let mut d = Project::empty(ProjectId(18), "Xuất video".into())
        .unwrap()
        .into_data();
    d.canvas.width = 64;
    d.canvas.height = 64;
    let range = SourceRange::new(Tick::ZERO, Tick(SECOND / 10)).unwrap();
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
                    cleanup: Default::default(),
                    background: project::ClipBackground::None,
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
fn bench(editor: Editor) -> Bench<super::super::State, super::super::Action> {
    Bench::hosted(
        editor,
        |editor| std::mem::take(&mut editor.dialogs),
        |editor, state| editor.dialogs = state,
        super::super::show,
        super::super::apply,
        egui::vec2(1280.0, 800.0),
    )
}
type DialogBench = Bench<super::super::State, super::super::Action>;
fn enabled(bench: &DialogBench, id: &str) -> bool {
    bench
        .editor()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .find(|w| w.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .enabled
}
fn open(bench: &mut DialogBench) {
    bench.editor_mut().activate("file.export").unwrap();
    bench.harness_mut().run_steps(2);
}
fn wait(bench: &mut DialogBench) -> super::ExportPhase {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        bench.editor_mut().tick();
        bench.step();
        let inspect = bench.editor().export_state().unwrap().inspect();
        if matches!(
            inspect.phase,
            super::ExportPhase::Ready | super::ExportPhase::Failed | super::ExportPhase::Settings
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
fn restored_source(restored: bool) -> (Arc<Restorable>, Arc<AtomicUsize>) {
    let flag = Arc::new(AtomicBool::new(restored));
    let decoded = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Restorable {
            restored: flag,
            decoded: decoded.clone(),
        }),
        decoded,
    )
}

#[test]
fn export_choices_default_persist_and_empty_timeline_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    for locale in [Locale::En, Locale::Vi] {
        let mut e = editor(true, false);
        e.preferences.locale = locale;
        assert_eq!(e.preferences.export, ExportOptions::default());
        let mut bench = bench(e);
        open(&mut bench);
        let state = bench.editor().export_state().unwrap().inspect();
        assert!(state.open && state.empty);
        assert!(!enabled(&bench, "export.run"));
        bench.click("export.format");
        bench.click("export.format.webm");
        bench.click("export.quality");
        bench.click("export.quality.very_high");
        bench.click("export.audio.include");
        assert_eq!(
            bench.editor().inspect().unwrap().export_options,
            ExportOptions {
                format: ExportFormat::Webm,
                quality: ExportQuality::VeryHigh,
                include_audio: false,
                ..Default::default()
            }
        );
        bench.editor_mut().activate("edit.undo").unwrap();
        assert!(bench.editor().preferences.export.include_audio);
        bench.editor_mut().activate("edit.redo").unwrap();
        assert!(!bench.editor().preferences.export.include_audio);
        store.save(&bench.editor().preferences).unwrap();
        assert_eq!(
            store.load_with_warning().0.export,
            bench.editor().preferences.export
        );
        // Audio-only formats are admitted rows of the same format control.
        bench.click("export.format");
        bench.click("export.format.m4a");
        assert!(
            bench
                .editor()
                .inspect()
                .unwrap()
                .export_options
                .format
                .audio_only()
        );
        assert!(
            bench
                .editor()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .all(|w| w.id != "export.codec")
        );
        let screenshot =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/84-export-m4a.png");
        std::fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
        bench
            .harness_mut()
            .render()
            .unwrap()
            .save(&screenshot)
            .unwrap();
        bench.click("export.format");
        bench.click("export.format.webm");
        bench.click("export.close");
        open(&mut bench);
        assert_eq!(bench.editor().preferences.export.format, ExportFormat::Webm);
        assert!(matches!(
            bench
                .editor_mut()
                .export_dialog()
                .unwrap()
                .start(Arc::new(project(true, false)), ExportOptions::default()),
            Err(Error::EmptyTimeline)
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

#[test]
fn export_progress_then_save_writes_toast_and_closes_without_an_extra_screen() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("My Project.mp4");
    let mut bench = bench(editor(false, false));
    let project_before = bench.editor().session.project().clone();
    // The command must create the dialog with the shared preview/export render state.
    bench.editor_mut().activate("export.run").unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.finished.as_ref().is_some_and(|path| path.exists()));
    assert!(!state.open || state.phase == super::ExportPhase::Ready);
    for id in ["export.retry", "export.download", "export.copy_file"] {
        assert!(
            !bench
                .editor()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|w| w.id == id)
        );
    }
    assert!(enabled(&bench, "export.save"));
    assert!(Arc::ptr_eq(
        &project_before,
        bench.editor().session.project()
    ));
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut bench);
    assert!(destination.exists());
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(!state.open);
    assert!(state.finished.is_none());
    assert!(bench.editor_mut().activate("export.retry").is_err());
    let toast = bench
        .editor()
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
    let mut bench = bench(editor(false, true));
    open(&mut bench);
    let (sources, _) = restored_source(false);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.format");
    bench.click("export.format.webm");
    bench.click("export.quality");
    bench.click("export.quality.very_high");
    bench.click("export.audio.include");
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    let state = bench.editor().export_state().unwrap().inspect();
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
    bench.editor_mut().preferences.export.format = ExportFormat::Mp4;
    assert!(!diagnostics.is_empty());
    assert!(enabled(&bench, "export.copy"));
    assert!(enabled(&bench, "export.retry"));
    assert!(
        !bench
            .editor()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == "export.run")
    );
    bench
        .harness_mut()
        .get(egui_kittest::kittest::By::new().predicate(|n| n.author_id() == Some("export.copy")))
        .click();
    bench.harness_mut().run_steps(1);
    assert!(
        bench
            .harness()
            .output()
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(
                command,
                egui::OutputCommand::CopyText(text) if text == &diagnostics
            ))
    );
    let (sources, _) = restored_source(true);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.retry");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert_eq!(bench.editor().export_save_format(), ExportFormat::Webm);
    assert!(bench.editor().export_save_defaults().0.ends_with(".webm"));
    assert_eq!(
        bench.editor().export_state().unwrap().inspect().attempt,
        Some(ExportOptions {
            format: ExportFormat::Webm,
            quality: ExportQuality::VeryHigh,
            include_audio: false,
            ..Default::default()
        })
    );
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut bench);
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
    let mut bench = bench(editor(false, true));
    let _release = Release(gate.clone());
    open(&mut bench);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(Arc::new(GateFactory(gate.clone())))
        .unwrap();
    bench.click("export.run");
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(bench.editor().export_state().unwrap().active());
    let running_job = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .job
        .unwrap()
        .id;
    assert!(bench.editor_mut().activate("export.retry").is_err());
    assert_eq!(
        bench
            .editor()
            .export_state()
            .unwrap()
            .inspect()
            .job
            .unwrap()
            .id,
        running_job
    );
    bench.click("export.cancel");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    let state = bench.editor().export_state().unwrap().inspect();
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
        let mut bench = bench(editor_project(Project::new(data).unwrap()));
        let project_before = bench.editor().session.project().clone();
        open(&mut bench);
        let (sources, _) = restored_source(false);
        bench
            .editor_mut()
            .export_dialog()
            .unwrap()
            .set_sources(sources)
            .unwrap();
        bench.click("export.run");
        assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
        let state = bench.editor().export_state().unwrap().inspect();
        assert_eq!(state.unavailable.len(), count as usize);
        assert_eq!(state.unavailable[0].asset, AssetId(1));
        assert_eq!(
            state.unavailable[0].range,
            SourceRange::new(Tick::ZERO, Tick(SECOND / 10)).unwrap()
        );
        assert!(!state.consent);
        assert!(Arc::ptr_eq(
            &project_before,
            bench.editor().session.project()
        ));
        let retry = bench
            .editor()
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
        bench
            .harness_mut()
            .get(
                egui_kittest::kittest::By::new()
                    .predicate(|node| node.author_id() == Some("export.consent")),
            )
            .click_accesskit();
        bench.harness_mut().run_steps(3);
        assert!(bench.editor().export_state().unwrap().inspect().consent);
        bench.click("export.retry");
        assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
        bench
            .editor_mut()
            .export_picked_file(destination.clone())
            .unwrap();
        wait(&mut bench);
        assert!(destination.exists());
        assert!(Arc::ptr_eq(
            &project_before,
            bench.editor().session.project()
        ));
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
    let mut bench = bench(e);
    open(&mut bench);
    let (sources, _) = restored_source(true);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    bench.editor_mut().export_save_cancelled();
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.open);
    assert!(state.finished.as_ref().is_some_and(|path| path.exists()));
    let finished = state.finished.unwrap();
    assert!(
        bench
            .editor()
            .export_save_defaults()
            .0
            .ends_with("-scene-0.mp4")
    );
    assert!(bench.editor_mut().activate("export.retry").is_err());
    // A failed save must retain the finished output and leave existing data intact.
    let blocked = dir.path().join("directory.mp4");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("keep"), b"previous").unwrap();
    bench
        .editor_mut()
        .export_picked_file(blocked.clone())
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    // Let the modal settle after progress changes to the diagnostics layout.
    bench.harness_mut().run_steps(3);
    assert!(enabled(&bench, "export.copy"));
    assert!(enabled(&bench, "export.retry"));
    let diagnostics = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .diagnostics
        .unwrap();
    bench
        .harness_mut()
        .get(egui_kittest::kittest::By::new().predicate(|n| n.author_id() == Some("export.copy")))
        .click();
    bench.harness_mut().run_steps(1);
    assert_eq!(bench.harness().output().platform_output.commands.iter().filter(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &diagnostics)).count(), 1);
    bench.click("export.retry");
    assert_eq!(
        bench
            .editor()
            .export_state()
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
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut bench);
    assert!(destination.exists());
    assert_ne!(std::fs::read(&destination).unwrap(), b"previous export");
    assert!(!finished.exists());
    assert_eq!(
        bench.editor().export_state().unwrap().phase(),
        super::ExportPhase::Ready
    );
    assert!(
        bench
            .editor()
            .export_save_defaults()
            .0
            .ends_with("-scene-2.mp4")
    );
    bench.editor_mut().export_save_cancelled();
    let second_finished = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .finished
        .unwrap();
    assert!(second_finished.exists());
    let second_destination = dir.path().join("second-scene.mp4");
    bench
        .editor_mut()
        .export_picked_file(second_destination.clone())
        .unwrap();
    wait(&mut bench);
    assert!(second_destination.is_file());
    assert!(!second_finished.exists());
    assert!(!bench.editor().export_state().unwrap().inspect().open);
}

#[test]
fn retry_reuses_settings_into_new_output_and_preserves_a_prior_file() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("prior.mp4");
    std::fs::write(&destination, b"previous").unwrap();
    let mut bench = bench(editor(false, true));
    open(&mut bench);
    let (sources, _) = restored_source(false);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    bench.click("export.retry");
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    bench.click("export.close");
    open(&mut bench);
    assert_eq!(
        bench.editor().export_state().unwrap().phase(),
        super::ExportPhase::Settings
    );
    assert!(enabled(&bench, "export.run"));
}

#[test]
fn filenames_are_sanitized_and_export_preferences_roundtrip() {
    assert_eq!(
        super::default_file_name("My: Project?", "mp4"),
        "My_ Project_.mp4"
    );
    assert_eq!(
        super::default_file_name("D\u{1ef1} \u{00e1}n phim", "webm"),
        "D\u{1ef1} \u{00e1}n phim.webm"
    );
    assert_eq!(super::default_file_name("  spaced  ", "mp4"), "spaced.mp4");
    assert_eq!(super::default_file_name("CON", "mov"), "_CON.mov");
    assert_eq!(super::default_file_name("LPT9.txt", "mp4"), "_LPT9.txt.mp4");
    assert_eq!(
        super::default_file_name("   ", "mp4"),
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
    let preferences = Preferences {
        export: options,
        export_directory: Some(dir.path().join("exports")),
        ..Default::default()
    };
    store.save(&preferences).unwrap();
    let loaded = store.load_with_warning().0;
    assert_eq!(loaded.export, options);
    assert_eq!(loaded.export_directory, preferences.export_directory);
}

#[test]
fn export_output_panel_persists_scene_range_refuses_empty_and_never_edits_project() {
    let mut bench = bench(editor(false, false));
    let rate = bench.editor().session.project().data().canvas.fps;
    bench
        .editor_mut()
        .dispatch(AppCommand::Seek(rate.tick_of(1).unwrap()))
        .unwrap();
    bench.editor_mut().activate("export.mark_in").unwrap();
    bench
        .editor_mut()
        .dispatch(AppCommand::Seek(rate.tick_of(2).unwrap()))
        .unwrap();
    bench.editor_mut().activate("export.mark_out").unwrap();
    assert_eq!(
        bench.editor().preferences.export.output.range,
        render::RangeChoice::InOut {
            start: rate.tick_of(1).unwrap(),
            end: rate.tick_of(2).unwrap()
        }
    );
    let before = project::encode(bench.editor().session.project(), false).unwrap();
    open(&mut bench);
    bench.click("export.resolution");
    bench.click("export.resolution.720");
    assert_eq!(
        bench.editor().preferences.export.output.resolution,
        Some([720, 720])
    );
    bench.click("export.custom.size");
    assert!(enabled(&bench, "export.width"));
    for id in [
        "export.resolution",
        "export.fps",
        "export.codec",
        "export.profile",
        "export.scene",
        "export.range",
    ] {
        assert!(enabled(&bench, id), "{id}");
    }
    bench.click("export.range");
    assert!(!enabled(&bench, "export.range.selected"));
    bench.click("export.range.full");
    bench
        .editor_mut()
        .select([project::ItemId(1)].into())
        .unwrap();
    bench.click("export.range");
    bench.click("export.range.selected");
    assert!(matches!(
        bench.editor().preferences.export.output.range,
        render::RangeChoice::Selected { .. }
    ));
    bench.click("export.scene");
    bench.click("export.scene.all");
    bench.click("export.range");
    bench.click("export.range.in_out");
    assert!(enabled(&bench, "export.range.in"));
    bench.editor_mut().preferences.export.output.range = render::RangeChoice::InOut {
        start: Tick::ZERO,
        end: Tick::ZERO,
    };
    bench.harness_mut().run_steps(3);
    assert!(!enabled(&bench, "export.run"));
    let invalid = bench.editor().preferences.export;
    assert!(
        bench
            .editor_mut()
            .export_dialog()
            .unwrap()
            .start(Arc::new(project(false, false)), invalid)
            .is_err()
    );
    let fps = bench.editor().session.project().data().canvas.fps;
    bench.editor_mut().preferences.export.output.range = render::RangeChoice::InOut {
        start: fps.tick_of(1).unwrap(),
        end: fps.tick_of(2).unwrap(),
    };
    bench.editor_mut().preferences.export.output.resolution = Some([64, 64]);
    bench.editor_mut().preferences.export.output.bit_rate = Some(100_000);
    bench.harness_mut().run_steps(3);
    assert!(enabled(&bench, "export.run"));
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    store.save(&bench.editor().preferences).unwrap();
    assert_eq!(
        store.load_with_warning().0.export,
        bench.editor().preferences.export
    );
    let options = bench.editor().preferences.export;
    bench
        .editor_mut()
        .dispatch(AppCommand::Export(options))
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert_eq!(
        project::encode(bench.editor().session.project(), false).unwrap(),
        before
    );
    bench.editor_mut().activate("edit.undo").unwrap();
    bench.editor_mut().activate("edit.redo").unwrap();
    assert_eq!(
        project::encode(bench.editor().session.project(), false).unwrap(),
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
    let mut bench = bench(Editor::new(session, playback, Preferences::default()));
    open(&mut bench);
    let (sources, _) = restored_source(true);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    let before = serde_json::to_vec(bench.editor().session.project().as_ref()).unwrap();
    let finished = bench
        .editor()
        .export_state()
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
        bench.editor_mut().export_picked_file(destination).unwrap();
        assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
        let state = bench.editor().export_state().unwrap().inspect();
        let error = state.job.unwrap().error;
        assert!(
            matches!(
                error
                    .as_deref()
                    .and_then(|error| (error as &(dyn std::error::Error + 'static))
                        .downcast_ref::<engine::RuntimeError>()),
                Some(engine::RuntimeError::Render(render::Error::Invalid(
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
            serde_json::to_vec(bench.editor().session.project().as_ref()).unwrap(),
            before
        );
        let result = bench.editor().export_state().unwrap().inspect().phase;
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            b"original source media",
            "saving an export replaced source media; save result: {result:?}"
        );
        assert_eq!(result, super::ExportPhase::Failed);
        bench.click("export.retry");
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
    let mut bench = bench(editor(false, false));
    open(&mut bench);
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    let finished = bench
        .editor()
        .export_state()
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
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_publication_fs(Arc::new(StorageGate(gate.clone())))
        .unwrap();
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    bench.harness_mut().run_steps(1);
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    bench.harness_mut().run_steps(3);
    assert!(enabled(&bench, "export.cancel"));
    bench.click("export.cancel");
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert!(finished.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
