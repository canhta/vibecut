#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Session};
use project::{Color, ItemParams, OverlayParams, Project, ProjectId, TrackKind};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{Locale, PreferenceStore, Preferences},
    egui,
    playback::{Files, Playback},
};
fn session() -> Session {
    let mut session =
        Session::new(Project::empty(ProjectId(1), "Test".into()).unwrap(), 10).unwrap();
    let outcome = session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "Overlay".into(),
            kind: TrackKind::Video,
        })
        .unwrap();
    let CreatedId::Track(track) = outcome.created[0] else {
        panic!("track")
    };
    session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                animations: Default::default(),
                source_asset: None,
                source_range: SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND * 10)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Overlay(OverlayParams {
                    color: Color([255, 0, 0, 255]),
                }),
            },
        })
        .unwrap();
    session
}
fn editor() -> Editor {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}
fn empty_editor() -> Editor {
    let session = Session::new(Project::empty(ProjectId(1), "Test".into()).unwrap(), 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}
#[test]
fn every_widget_has_unique_locale_stable_id_and_input_reaches_engine() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor());
    harness.run_steps(3);
    assert!(
        harness.query_all(By::new().label("Media")).count() > 0,
        "English catalog label is visible before switching"
    );
    let before = harness.state().inspect().unwrap();
    let ids: BTreeSet<_> = before.widgets.iter().map(|w| w.id.clone()).collect();
    assert_eq!(ids.len(), before.widgets.len());
    assert!(
        ids.contains("edit.undo")
            && ids.contains("timeline.ruler")
            && ids.contains("layout.timeline")
    );
    for id in &ids {
        assert_eq!(
            harness
                .query_all(By::new().predicate(|n| n.author_id() == Some(id.as_str())))
                .count(),
            1,
            "id {id}"
        );
    }
    let initial = harness.state().session.project().clone();
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.undo")))
        .click();
    harness.run_steps(3);
    assert_ne!(harness.state().session.project(), &initial);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.redo")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().session.project(), &initial);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("appearance.locale")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::Vi);
    assert!(
        harness.query_all(By::new().label("Tư liệu")).count() > 0,
        "Vietnamese catalog label is visible after switching without a restart"
    );
    assert_eq!(harness.state().session.project(), &initial);
    assert_eq!(
        harness
            .state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .map(|w| w.id.clone())
            .collect::<BTreeSet<_>>(),
        ids
    );
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.undo")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::En);
    assert_eq!(harness.state().session.project(), &initial);
    assert!(
        harness.query_all(By::new().label("Media")).count() > 0,
        "locale undo restores the English catalog"
    );
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.redo")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::Vi);
    assert_eq!(harness.state().session.project(), &initial);
    assert!(
        harness.query_all(By::new().label("Tư liệu")).count() > 0,
        "locale redo restores the Vietnamese catalog"
    );
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("transport.forward")))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(TICKS_PER_SECOND)
    );
}
#[test]
fn locale_change_uses_app_history_and_leaves_project_history_untouched() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), empty_editor());
    harness.run_steps(3);
    let initial = harness.state().session.project().clone();
    assert!(!harness.state().session.can_undo());
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("appearance.locale")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::Vi);
    assert_eq!(harness.state().session.project(), &initial);
    assert!(
        !harness.state().session.can_undo(),
        "a locale change must not enter Project history"
    );
    let enabled = |harness: &Harness<Editor>, id: &str| {
        harness
            .state()
            .inspect()
            .unwrap()
            .widgets
            .into_iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing widget {id}"))
            .enabled
    };
    assert!(enabled(&harness, "edit.undo"));
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.undo")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::En);
    assert_eq!(harness.state().session.project(), &initial);
    assert!(enabled(&harness, "edit.redo"));
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.redo")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::Vi);
    assert_eq!(harness.state().session.project(), &initial);
    assert!(!harness.state().session.can_redo());
}
#[test]
fn preview_uses_budget_skips_late_video_and_invalidates_seek_and_clear() {
    let mut editor = editor();
    let port = editor.playback.audio_port();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.playback.poll().unwrap();
        if editor.playback.frame().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", editor.playback.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    let first_epoch = editor.playback.frame().unwrap().epoch;
    editor.activate("transport.forward").unwrap();
    assert!(editor.playback.frame().is_none());
    editor.activate("transport.play").unwrap();
    let mut audio = [0.0; 960];
    for _ in 0..201 {
        port.fill(&mut audio, 0);
        editor.playback.poll().unwrap();
    }
    assert!(editor.playback.position().unwrap() >= Tick(TICKS_PER_SECOND * 3));
    editor.playback.clear().unwrap();
    assert!(editor.playback.playing());
    assert!(editor.playback.frame().is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // Keep the simulated device callback active while transport is playing.
        port.fill(&mut audio, 0);
        editor.playback.poll().unwrap();
        if let Some(frame) = editor.playback.frame() {
            assert!(frame.epoch > first_epoch);
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", editor.playback.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    let memory = editor.playback.budget().snapshot().unwrap();
    assert!(memory.used <= memory.limit);
    editor.activate("transport.pause").unwrap();
    let at = editor.playback.position().unwrap();
    port.fill(&mut audio, 0);
    assert!(audio.iter().all(|x| *x == 0.0));
    assert_eq!(editor.playback.position().unwrap(), at);
    assert!(
        editor
            .playback
            .seek(Tick(-1), false, editor.session.project().clone())
            .is_err()
    );
}
#[test]
fn frame_exceeding_budget_is_typed_and_shutdown_joins_workers() {
    let session = session();
    let budget = ByteBudget::new(2 << 20).unwrap();
    let mut playback = Playback::new(&session, budget.clone(), Arc::new(Files)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        playback.poll().unwrap();
        if playback.error().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(playback.frame().is_none());
    assert!(budget.snapshot().unwrap().used <= 2 << 20);
    drop(playback);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn preference_store_roundtrips_locale_and_refuses_hostile_size() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let p = Preferences {
        locale: Locale::Vi,
        ..Default::default()
    };
    store.save(&p).unwrap();
    assert_eq!(store.load().unwrap().locale, Locale::Vi);
    std::fs::write(store.path(), vec![b' '; 4097]).unwrap();
    assert!(store.load().is_err());
}

struct PanickingSources;
impl vibecut::playback::Sources for PanickingSources {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        panic!("injected worker failure")
    }
}
#[test]
fn engine_catches_preview_worker_panic_and_app_reports_failure() {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(PanickingSources),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Err(error) = playback.poll() {
            assert!(matches!(error, vibecut::Error::Worker(_)));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn capture_headless_has_bounded_typed_timeout() {
    let context = egui::Context::default();
    let capture =
        vibecut::capture::Capture::request(&context, None, Duration::from_millis(1)).unwrap();
    std::thread::sleep(Duration::from_millis(2));
    assert!(matches!(
        capture.poll(),
        Err(vibecut::Error::CaptureTimeout)
    ));
    assert!(vibecut::capture::Capture::request(&context, None, Duration::from_secs(6)).is_err());
}

#[test]
fn failed_device_pauses_without_wall_clock_substitution() {
    let mut editor = editor();
    editor.activate("transport.play").unwrap();
    editor.playback.audio_port().fill(&mut [0.0; 960], 0);
    editor.playback.audio_port().fail();
    assert!(matches!(
        editor.playback.poll(),
        Err(vibecut::Error::PreviewUnavailable(_))
    ));
    assert!(!editor.playback.playing());
    let at = editor.playback.position().unwrap();
    editor.playback.poll().unwrap();
    assert_eq!(editor.playback.position().unwrap(), at);
    assert!(editor.activate("transport.play").is_err());
}
#[test]
fn repeated_forward_backward_seeks_and_clears_retain_no_unbounded_frames() {
    let mut editor = editor();
    for n in 0..200 {
        let at = Tick((n % 10) * TICKS_PER_SECOND);
        editor
            .playback
            .seek(at, false, editor.session.project().clone())
            .unwrap();
        editor.playback.poll().unwrap();
        if n % 5 == 0 {
            editor.playback.clear().unwrap();
        }
        let snapshot = editor.playback.budget().snapshot().unwrap();
        assert!(snapshot.used <= snapshot.limit);
    }
    let budget = editor.playback.budget().clone();
    drop(editor);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn vietnamese_light_controls_fit_the_minimum_window() {
    let mut state = editor();
    state.preferences.locale = Locale::Vi;
    state.preferences.dark = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), state);
    harness.run_steps(3);
    for widget in harness.state().inspect().unwrap().widgets {
        assert!(widget.rect.is_finite());
        assert!(
            widget.rect.min.x >= 0.0
                && widget.rect.min.y >= 0.0
                && widget.rect.max.x <= 1280.0
                && widget.rect.max.y <= 800.0,
            "{}: {:?}",
            widget.id,
            widget.rect
        );
    }
}

#[test]
fn empty_player_keeps_its_accessible_instruction_after_a_black_frame_is_ready() {
    let session = Session::new(Project::empty(ProjectId(2), "Empty".into()).unwrap(), 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while playback.frame().is_none() {
        playback.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let editor = Editor::new(session, playback, Preferences::default());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    harness.get_by_label("Project is empty");
    assert!(harness.state().presented_frame().is_none());
}

#[test]
fn transport_k_pauses_with_keyboard_focus_on_a_control() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor());
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("transport.play")))
        .click();
    harness.run_steps(2);
    assert!(harness.state().playback.playing());
    harness.key_press(egui::Key::Tab);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|w| w.focused)
    );
    harness.key_press(egui::Key::K);
    harness.run_steps(2);
    assert!(
        !harness.state().playback.playing(),
        "K must pause while a non-text control has keyboard focus"
    );
    harness.key_press(egui::Key::Space);
    harness.run_steps(2);
    assert!(harness.state().playback.playing());
}

struct Hooks;
struct Lease;
impl render::Reservation for Lease {}
impl render::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(Lease))
    }
}
#[test]
fn transparent_canvas_preview_matches_black_backed_render() {
    let mut data = session().project().as_ref().clone().into_data();
    data.canvas.background = Color([0, 0, 0, 0]);
    data.scene_mut().timeline.overlays[0].items[0].content = ItemParams::Overlay(OverlayParams {
        color: Color([255, 0, 0, 128]),
    });
    let project = Project::new(data).unwrap();
    let session = Session::new(project, 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while playback.frame().is_none() {
        playback.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut settings = render::RenderSettings::from_project(session.project());
    settings.width = 640;
    settings.height = 360;
    let mut renderer = render::Renderer::new(session.project(), settings).unwrap();
    let image = renderer
        .render_frame(Tick::ZERO, &mut render::FileSources::default(), &Hooks)
        .unwrap();
    let opaque = image.video_rgba8(13, &Hooks).unwrap();
    let preview = playback.frame().unwrap().image.pixels[0];
    assert_eq!(
        preview.r(),
        opaque.bytes[0],
        "preview over black must match render's black-backed pixels"
    );
}

struct PanicOnce(std::sync::atomic::AtomicBool);
impl vibecut::playback::Sources for PanicOnce {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        if self.0.swap(false, std::sync::atomic::Ordering::SeqCst) {
            panic!("transient worker panic");
        }
        Box::new(render::FileSources::default())
    }
}
#[test]
fn retry_recovers_a_transient_worker_failure() {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(PanicOnce(std::sync::atomic::AtomicBool::new(true))),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if playback.poll().is_err() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    playback.clear().unwrap();
    assert!(
        playback.poll().is_ok(),
        "retry should replace the failed preview job"
    );
}

#[test]
fn retry_widget_recovers_after_a_transient_panic_and_clears_its_error_state() {
    let session = session();
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(PanicOnce(std::sync::atomic::AtomicBool::new(true))),
    )
    .unwrap();
    let editor = Editor::new(session, playback, Preferences::default());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        harness.run_steps(1);
        if harness.state().playback.error().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let retry = harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == "preview.retry")
        .unwrap();
    assert!(
        harness.state().panels["preview"].contains_rect(retry.rect),
        "Retry must be inside the player panel: {:?} versus {:?}",
        retry.rect,
        harness.state().panels["preview"]
    );
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("preview.retry")))
        .click();
    loop {
        harness.run_steps(1);
        if harness.state().playback.frame().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(harness.state().playback.error().is_none());
    assert!(
        harness
            .query(By::new().predicate(|node| node.author_id() == Some("preview.retry")))
            .is_none()
    );
}

struct GatedFailure {
    calls: std::sync::atomic::AtomicUsize,
    blocked: std::sync::atomic::AtomicBool,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}
impl GatedFailure {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
impl vibecut::playback::Sources for GatedFailure {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        match self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            0 => {
                // Failure polling cancels the companion. Ensure it has entered
                // the blocking adapter before allowing that cancellation.
                {
                    let mut released = self.released.lock().unwrap();
                    while !self.blocked.load(std::sync::atomic::Ordering::Acquire) && !*released {
                        released = self.wake.wait(released).unwrap();
                    }
                }
                panic!("transient worker failure with an active companion");
            }
            1 => {
                let mut released = self.released.lock().unwrap();
                self.blocked
                    .store(true, std::sync::atomic::Ordering::Release);
                self.wake.notify_all();
                while !*released {
                    released = self.wake.wait(released).unwrap();
                }
            }
            _ => {}
        }
        Box::new(render::FileSources::default())
    }
}
// Release a deliberately noncooperating adapter before Playback's joining Drop,
// including when an assertion fails, so a regression fails instead of hanging CI.
struct ReleaseFactory(Arc<GatedFailure>);
impl Drop for ReleaseFactory {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[test]
fn failed_workers_pause_fence_and_join_before_retry_without_blocking_the_action() {
    let factory = Arc::new(GatedFailure {
        calls: std::sync::atomic::AtomicUsize::new(0),
        blocked: std::sync::atomic::AtomicBool::new(false),
        released: std::sync::Mutex::new(false),
        wake: std::sync::Condvar::new(),
    });
    let session = session();
    let budget = ByteBudget::new(64 << 20).unwrap();
    let mut playback = Playback::new(&session, budget.clone(), factory.clone()).unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    let release = ReleaseFactory(factory.clone());
    editor
        .playback
        .seek(
            Tick(3 * TICKS_PER_SECOND),
            true,
            editor.session.project().clone(),
        )
        .unwrap();
    let port = editor.playback.audio_port();
    port.fill(&mut [0.0; 960], 0);
    port.fill(&mut [0.0; 960], 0);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let failed = matches!(editor.playback.poll(), Err(vibecut::Error::Worker(_)));
        if failed && factory.blocked.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let at = editor.playback.position().unwrap();
    let stopped_bytes = budget.snapshot().unwrap().used;
    assert!(!editor.playback.playing());
    assert!(editor.playback.frame().is_none());
    assert!(editor.activate("transport.play").is_err());
    let mut silence = [1.0; 960];
    port.fill(&mut silence, 0);
    assert!(silence.iter().all(|sample| *sample == 0.0));
    assert_eq!(editor.playback.position().unwrap(), at);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let action = scope.spawn(|| tx.send(editor.activate("preview.retry")).unwrap());
        let result = rx.recv_timeout(Duration::from_secs(2));
        if result.is_err() {
            factory.release();
        }
        action.join().unwrap();
        result.unwrap().unwrap();
    });
    for _ in 0..5 {
        editor.playback.poll().unwrap();
        editor.activate("preview.retry").unwrap();
        assert!(!editor.playback.playing());
        assert_eq!(editor.playback.position().unwrap(), at);
    }
    assert_eq!(
        factory.calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "obsolete companion must stop before replacement jobs are admitted"
    );
    factory.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.playback.poll().unwrap();
        if let Some(frame) = editor.playback.frame() {
            assert!(frame.epoch > 1);
            assert!(frame.at <= at);
            assert_eq!(frame.image.pixels[0], egui::Color32::RED);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(editor.playback.error().is_none());
    assert_eq!(editor.playback.position().unwrap(), at);
    assert!(
        budget.snapshot().unwrap().used > stopped_bytes,
        "replacement pixels must be charged to the original aggregate budget"
    );
    assert!(budget.snapshot().unwrap().used <= budget.snapshot().unwrap().limit);
    editor.activate("transport.play").unwrap();
    assert!(editor.playback.playing());
    editor.activate("transport.pause").unwrap();
    drop(release);
    drop(port);
    drop(editor);
    assert_eq!(
        budget.snapshot().unwrap().used,
        0,
        "shutdown joins replacement workers and releases every lease"
    );
}

#[test]
fn packaged_app_startup_runs_without_display_or_audio_device() {
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_vibecut"))
        .arg("--headless-smoke")
        .arg(directory.path().join("workspace"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_vibecut"))
        .arg("--headless-smoke")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(!refused.status.success());
}

#[test]
fn new_seek_cancels_audio_preroll_and_keeps_the_replacement_epoch_alive() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Gate {
        first: AtomicBool,
        entered: AtomicBool,
        cancelled: AtomicBool,
    }
    struct Factory(Arc<Gate>);
    struct Source(Arc<Gate>);
    impl vibecut::playback::Sources for Factory {
        fn create(&self) -> Box<dyn render::SourceProvider> {
            Box::new(Source(self.0.clone()))
        }
    }
    impl render::SourceProvider for Source {
        fn status(
            &mut self,
            _: &media::SourceAsset,
            _: bool,
            _: &dyn render::ExecutionHooks,
        ) -> render::Result<Option<render::UnavailableReason>> {
            Ok(None)
        }
        fn video(
            &mut self,
            _: &media::SourceAsset,
            _: project::ItemId,
            _: Tick,
            _: &dyn render::ExecutionHooks,
        ) -> render::Result<Option<render::Image>> {
            Ok(None)
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: project::ItemId,
            _: i64,
            frames: usize,
            rate: u32,
            hooks: &dyn render::ExecutionHooks,
        ) -> render::Result<render::AudioBlock> {
            if self.0.first.swap(false, Ordering::SeqCst) {
                self.0.entered.store(true, Ordering::Release);
                while !hooks.cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                self.0.cancelled.store(true, Ordering::Release);
                return Err(render::Error::Cancelled);
            }
            let mut block =
                render::AudioBlock::new(rate, 2, frames, render::MemoryKind::Decoded, hooks)?;
            for plane in block.planes_mut() {
                plane.fill(0.1);
            }
            Ok(block)
        }
    }
    let duration = Tick(TICKS_PER_SECOND * 60);
    let mut data = Project::empty(ProjectId(8), "Audio seek".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(1),
            path: std::env::temp_dir().join("vibecut-audio-seek.wav"),
            content_hash: media::ContentHash::from_bytes(&[8]),
            probe: media::SourceProbe {
                format: "wav".into(),
                duration: Some(duration),
                streams: vec![media::SourceStream {
                    index: 0,
                    kind: media::StreamKind::Audio,
                    codec: "pcm_f32le".into(),
                    time_base: media::TimeBase::new(1, 48000).unwrap(),
                    start: None,
                    duration: Some(duration),
                    frame_rate: None,
                    frame_count: None,
                    variable_frame_rate: None,
                    video: None,
                    audio: Some(media::AudioInfo {
                        sample_rate: 48000,
                        channels: 2,
                        initial_padding: 0,
                        trailing_padding: 0,
                    }),
                }],
            },
        })
        .unwrap(),
    );
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![project::Item {
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            content: ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    let session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let gate = Arc::new(Gate {
        first: AtomicBool::new(true),
        entered: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
    });
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Factory(gate.clone())),
    )
    .unwrap();
    playback.set_device_available(true);
    playback
        .seek(Tick(30 * TICKS_PER_SECOND), true, session.project().clone())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gate.entered.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    playback
        .seek(Tick::ZERO, true, session.project().clone())
        .unwrap();
    let port = playback.audio_port();
    loop {
        let mut samples = [0.0; 64];
        port.fill(&mut samples, 0);
        playback.poll().unwrap();
        if samples.iter().any(|v| *v != 0.0) {
            assert!(gate.cancelled.load(Ordering::Acquire));
            assert!(
                samples
                    .iter()
                    .filter(|v| **v != 0.0)
                    .all(|v| (*v - 0.098).abs() < 1e-6)
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "replacement audio did not arrive after cancelling preroll"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(playback.error().is_none());
    playback
        .seek(Tick::ZERO, false, session.project().clone())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while playback.budget().snapshot().unwrap().owners[1] != 0 {
        assert!(Instant::now() < deadline);
        playback.poll().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
}
