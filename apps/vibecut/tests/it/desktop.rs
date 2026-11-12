#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
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
    Editor, egui,
    i18n::Locale,
    preferences::{PreferenceStore, Preferences},
    services::playback::{Files, Playback},
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
                retime: Default::default(),
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
fn open_window_menu(harness: &mut Harness<Editor>) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
        .click();
    harness.run_steps(2);
}
#[test]
fn every_widget_has_unique_locale_stable_id_and_input_reaches_engine() {
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor());
    harness.run_steps(3);
    assert!(
        harness.query_all(By::new().label("Media")).count() > 0,
        "English catalog label is visible before switching"
    );
    open_window_menu(&mut harness);
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
    open_window_menu(&mut harness);
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
    open_window_menu(&mut harness);
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
        .get(By::new().predicate(|n| n.author_id() == Some("preview.menu")))
        .click();
    harness.run_steps(3);
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
fn transport_widget_identity_survives_preview_frame_arrival_and_empty_transition() {
    // The preview frame arrives asynchronously, so the canvas is drawn either
    // with its loading label or with the frame. Controls below it must keep
    // their egui widget id (the AccessKit node id): egui drops a click when the
    // press and release frames resolve to different widget ids.
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor());
    harness.run_steps(3);
    let node_id = |harness: &Harness<Editor>| {
        format!(
            "{:?}",
            harness
                .get(By::new().predicate(|n| n.author_id() == Some("transport.play")))
                .accesskit_node()
                .id()
        )
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.state().playback.frame().is_none() {
        assert!(Instant::now() < deadline, "preview frame did not arrive");
        harness.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
    let with_frame = node_id(&harness);

    // Remove the only item so the canvas takes its empty branch. The play
    // control stays rendered (disabled) with the same automation id.
    let item = {
        let project = harness.state().session.project();
        project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .map(|item| item.id)
            .next()
            .expect("fixture item")
    };
    harness.state_mut().selection.insert(item);
    harness.state_mut().activate("timeline.delete").unwrap();
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().duration(),
        Tick::ZERO,
        "fixture must reach the empty canvas"
    );
    assert_eq!(
        node_id(&harness),
        with_frame,
        "the transport widget id must not change when the preview canvas switches between frame and empty content"
    );
}

#[test]
fn locale_change_uses_app_history_and_leaves_project_history_untouched() {
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), empty_editor());
    harness.run_steps(3);
    let initial = harness.state().session.project().clone();
    assert!(!harness.state().session.can_undo());
    open_window_menu(&mut harness);
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
        editor.playback.poll(std::time::Instant::now()).unwrap();
        if editor.playback.frame().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", editor.playback.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    let first_epoch = editor.playback.frame().unwrap().generation;
    editor.activate("transport.forward").unwrap();
    assert!(editor.playback.frame().is_none());
    editor.activate("transport.play").unwrap();
    let mut audio = [0.0; 960];
    for _ in 0..201 {
        port.fill(&mut audio, 0);
        editor.playback.poll(std::time::Instant::now()).unwrap();
    }
    assert!(editor.playback.position().unwrap() >= Tick(TICKS_PER_SECOND * 3));
    editor.playback.clear().unwrap();
    assert!(editor.playback.playing());
    assert!(editor.playback.frame().is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // Keep the simulated device callback active while transport is playing.
        port.fill(&mut audio, 0);
        editor.playback.poll(std::time::Instant::now()).unwrap();
        if let Some(frame) = editor.playback.frame() {
            assert!(frame.generation > first_epoch);
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
        playback.poll(std::time::Instant::now()).unwrap();
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
impl vibecut::services::playback::Sources for PanickingSources {
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
        if let Err(error) = playback.poll(std::time::Instant::now()) {
            let vibecut::Error::Worker(cause) = error else {
                panic!("lost the worker diagnostic")
            };
            assert!(matches!(
                (cause.as_ref() as &(dyn std::error::Error + 'static))
                    .downcast_ref::<engine::RuntimeError>(),
                Some(engine::RuntimeError::WorkerPanicked)
            ));
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
        editor.playback.poll(std::time::Instant::now()),
        Err(vibecut::Error::PreviewUnavailable(_))
    ));
    assert!(!editor.playback.playing());
    let at = editor.playback.position().unwrap();
    editor.playback.poll(std::time::Instant::now()).unwrap();
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
        editor.playback.poll(std::time::Instant::now()).unwrap();
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
    let mut harness = crate::gpu::builder()
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
        playback.poll(std::time::Instant::now()).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let editor = Editor::new(session, playback, Preferences::default());
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    harness.get_by_label("Project is empty");
    assert!(harness.state().presented_frame().is_none());
}

#[test]
fn transport_k_pauses_with_keyboard_focus_on_a_control() {
    let mut harness = crate::gpu::builder()
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
        playback.poll(std::time::Instant::now()).unwrap();
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
impl vibecut::services::playback::Sources for PanicOnce {
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
        if playback.poll(std::time::Instant::now()).is_err() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    playback.clear().unwrap();
    assert!(
        playback.poll(std::time::Instant::now()).is_ok(),
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
    let mut harness = crate::gpu::builder()
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
        harness.state().panels()["preview"].contains_rect(retry.rect),
        "Retry must be inside the player panel: {:?} versus {:?}",
        retry.rect,
        harness.state().panels()["preview"]
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
impl vibecut::services::playback::Sources for GatedFailure {
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
        let failed = matches!(
            editor.playback.poll(std::time::Instant::now()),
            Err(vibecut::Error::Worker(_))
        );
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
        editor.playback.poll(std::time::Instant::now()).unwrap();
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
        editor.playback.poll(std::time::Instant::now()).unwrap();
        if let Some(frame) = editor.playback.frame() {
            assert!(frame.generation > 1);
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
    impl vibecut::services::playback::Sources for Factory {
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
        testkit::source::asset(1, std::env::temp_dir().join("vibecut-audio-seek.wav"))
            .format("wav")
            .duration(duration)
            .content_hash(media::ContentHash::from_bytes(&[8]))
            .stream(
                testkit::source::audio()
                    .codec("pcm_f32le")
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(duration)
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            retime: Default::default(),
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
        playback.poll(std::time::Instant::now()).unwrap();
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
        playback.poll(std::time::Instant::now()).unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
}

// The source gates work before publication, so callbacks consume prefilled
// samples even if a superseded worker has not had a chance to run again.
struct PreviewWork {
    provider: u64,
    start: i64,
    frames: usize,
    release: std::sync::mpsc::SyncSender<bool>,
}
struct ControlledSources {
    audio: bool,
    work: std::sync::mpsc::Sender<PreviewWork>,
    providers: std::sync::atomic::AtomicU64,
}
struct ControlledSource {
    provider: u64,
    audio: bool,
    work: Option<std::sync::mpsc::Sender<PreviewWork>>,
}
impl vibecut::services::playback::Sources for ControlledSources {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(ControlledSource {
            provider: self
                .providers
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            audio: self.audio,
            work: Some(self.work.clone()),
        })
    }
}
impl ControlledSource {
    fn gate(&self, start: i64, frames: usize, hooks: &dyn render::ExecutionHooks) -> bool {
        if let Some(work) = &self.work {
            let (release, receive) = std::sync::mpsc::sync_channel(1);
            if work
                .send(PreviewWork {
                    provider: self.provider,
                    start,
                    frames,
                    release,
                })
                .is_ok()
            {
                while !hooks.cancelled() {
                    match receive.recv_timeout(Duration::from_millis(10)) {
                        Ok(accept) => return accept,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            }
        }
        true
    }
}
impl render::SourceProvider for ControlledSource {
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
        at: Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        if !self.audio {
            self.gate(at.0, 0, hooks);
        }
        // Deliberately finish stale work even after cancellation, as an adapter
        // already returning pixels may do. Publication must still fence it.
        render::Image::new(64, 64, [at.seconds() as f32 / 10.0, 0.0, 0.0, 1.0], &Hooks).map(Some)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        if self.audio && !self.gate(start, frames, hooks) {
            return Err(render::Error::Invalid("controlled audio failure"));
        }
        let mut block =
            render::AudioBlock::new(rate, 2, frames, render::MemoryKind::Decoded, hooks)?;
        for plane in block.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                // A loud opening drives the limiter's envelope, followed by a
                // quiet waveform whose recovery depends on retained DSP state.
                let position = start + n as i64;
                *sample = if position < 500 {
                    2.0
                } else {
                    0.2 + (position % 97) as f32 / 1000.0
                };
            }
        }
        Ok(block)
    }
}
fn controlled_preview(audio: bool) -> (Session, Playback, std::sync::mpsc::Receiver<PreviewWork>) {
    let duration = Tick(10 * TICKS_PER_SECOND);
    let mut data = Project::empty(ProjectId(91), "Controlled preview".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("controlled-preview"))
            .duration(duration)
            .stream(if audio {
                testkit::source::audio().duration(duration).build()
            } else {
                testkit::source::video().duration(duration).build()
            })
            .build(),
    );
    let mut session = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let outcome = session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "Source".into(),
            kind: if audio {
                TrackKind::Audio
            } else {
                TrackKind::Video
            },
        })
        .unwrap();
    let CreatedId::Track(track) = outcome.created[0] else {
        panic!("track")
    };
    session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                source_asset: Some(media::AssetId(1)),
                source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                retime: Default::default(),
                animations: Default::default(),
                content: if audio {
                    ItemParams::Audio(project::AudioParams {
                        volume: 0.0,
                        muted: false,
                        source: Default::default(),
                        fades: Default::default(),
                    })
                } else {
                    ItemParams::Video(project::VideoParams {
                        volume: 0.0,
                        muted: true,
                        source_audio_enabled: false,
                        crop: None,
                        fades: Default::default(),
                    })
                },
            },
        })
        .unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(ControlledSources {
            audio,
            work: send,
            providers: std::sync::atomic::AtomicU64::new(0),
        }),
    )
    .unwrap();
    playback.set_device_available(true);
    (session, playback, receive)
}
#[allow(
    clippy::expect_used,
    reason = "test helper waiting on the controlled source channel; a timeout is the test failure"
)]
fn next_preview_work(receive: &std::sync::mpsc::Receiver<PreviewWork>) -> PreviewWork {
    receive
        .recv_timeout(Duration::from_secs(10))
        .expect("controlled source work")
}
fn wait_preview(playback: &mut Playback, at: Tick) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        playback.poll(std::time::Instant::now()).unwrap();
        if let Some(frame) = playback.frame() {
            assert_eq!(frame.at, at, "a stale frame must never be shown");
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", playback.error());
        std::thread::yield_now();
    }
}
#[test]
fn preview_axis_playing_hover_preserves_prefilled_pcm_source_cursor_and_dsp() {
    let (session, mut playback, receive) = controlled_preview(true);
    let project = serde_json::to_vec(session.project().data()).unwrap();
    let history = session.history_len();
    playback
        .seek(Tick::ZERO, true, session.project().clone())
        .unwrap();
    let mut pending = next_preview_work(&receive);
    let provider = pending.provider;
    let latency = pending.frames as i64;
    let mut end = 0;
    // Entering the next source call proves the preceding block was mixed and
    // published into the real ring; hold work after at least two full blocks.
    while pending.start < 2400 {
        assert_eq!(pending.start, end);
        end += pending.frames as i64;
        pending.release.send(true).unwrap();
        pending = next_preview_work(&receive);
    }
    let port = playback.audio_port();
    let mut actual = Vec::new();
    for hover in [
        Some(Tick(TICKS_PER_SECOND)),
        Some(Tick(4 * TICKS_PER_SECOND)),
        None,
        Some(Tick(2 * TICKS_PER_SECOND)),
        None,
    ] {
        let position = playback.position().unwrap();
        playback.set_hover(hover).unwrap();
        assert_eq!(
            playback.position().unwrap(),
            position,
            "pointer changes cannot jump a playing transport"
        );
        let submitted = actual.len() / 2;
        let mut samples = [0.0; 256];
        port.fill(&mut samples, 0);
        assert_eq!(
            playback.position().unwrap(),
            Tick::from_units(submitted as i64, 48_000, time::Rounding::Nearest).unwrap()
        );
        assert!(
            samples.iter().all(|sample| *sample != 0.0),
            "hover invalidated prefilled PCM"
        );
        actual.extend_from_slice(&samples);
        assert!(playback.playing());
    }
    assert_eq!(pending.start, end);
    assert_eq!(pending.provider, provider);
    pending.release.send(true).unwrap();
    let continued = next_preview_work(&receive);
    assert_eq!(
        continued.provider, provider,
        "hover must retain the playing source/mixer"
    );
    assert_eq!(
        continued.start,
        end + pending.frames as i64,
        "source cursor must stay continuous"
    );
    let published_frames = (continued.start - latency) as usize;
    let mut after_hover = vec![0.0; published_frames * 2 - actual.len()];
    port.fill(&mut after_hover, 0);
    assert!(after_hover.iter().all(|sample| *sample != 0.0));
    actual.extend_from_slice(&after_hover);
    let mut reference = ControlledSource {
        provider: 0,
        audio: true,
        work: None,
    };
    let expected = render::mix_audio(
        session.project(),
        0,
        actual.len() / 2,
        2,
        &mut reference,
        &Hooks,
    )
    .unwrap();
    for (n, pair) in actual.as_chunks::<2>().0.iter().enumerate() {
        for (channel, sample) in pair.iter().enumerate() {
            assert!((*sample - expected.planes()[channel][n]).abs() < 1e-6);
        }
    }
    assert_eq!(port.underrun_frames(), 0);
    assert!(playback.position().unwrap() > Tick::ZERO);
    assert_eq!(
        serde_json::to_vec(session.project().data()).unwrap(),
        project
    );
    assert_eq!(session.history_len(), history);
    eprintln!(
        "preview axis: {} nonzero PCM samples, zero underruns",
        actual.len()
    );
}
#[test]
fn preview_axis_stale_pixels_are_fenced_after_rapid_hover_and_seek() {
    let (session, mut playback, receive) = controlled_preview(false);
    let stale = next_preview_work(&receive);
    for seconds in [1, 5, 2, 8] {
        playback
            .set_hover(Some(Tick(seconds * TICKS_PER_SECOND)))
            .unwrap();
    }
    stale.release.send(true).unwrap();
    let latest = next_preview_work(&receive);
    assert_eq!(latest.start, 8 * TICKS_PER_SECOND);
    latest.release.send(true).unwrap();
    wait_preview(&mut playback, Tick(8 * TICKS_PER_SECOND));
    let first = playback.frame().unwrap().clone();
    assert!(first.image.pixels.iter().all(|p| p.r() > 200 && p.g() == 0));
    let context = egui::Context::default();
    let mut texture = vibecut::services::preview::FrameTexture::default();
    texture.update(&context, "axis", &first);
    playback
        .set_hover(Some(Tick(6 * TICKS_PER_SECOND)))
        .unwrap();
    let stale = next_preview_work(&receive);
    playback
        .seek(Tick(3 * TICKS_PER_SECOND), false, session.project().clone())
        .unwrap();
    stale.release.send(true).unwrap();
    let replacement = next_preview_work(&receive);
    assert_eq!(replacement.start, 3 * TICKS_PER_SECOND);
    replacement.release.send(true).unwrap();
    wait_preview(&mut playback, Tick(3 * TICKS_PER_SECOND));
    let frame = playback.frame().unwrap();
    assert!(frame.generation > first.generation);
    assert!(
        frame
            .image
            .pixels
            .iter()
            .all(|p| p.r() < 180 && p.r() > 100 && p.g() == 0)
    );
    texture.update(&context, "axis", frame);
    assert_eq!(texture.presented(), Some(Tick(3 * TICKS_PER_SECOND)));
    let generation = frame.generation;
    let mut output = context.run_pass(Default::default(), |_| {});
    output.textures_delta.clear();
    let mut settings = render::RenderSettings::from_project(session.project());
    let item = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .next()
        .unwrap()
        .id;
    settings.placements.insert(
        item,
        render::Placement {
            opacity: 0.0,
            ..Default::default()
        },
    );
    playback.set_render_settings(settings).unwrap();
    wait_preview(&mut playback, Tick(3 * TICKS_PER_SECOND));
    let frame = playback.frame().unwrap();
    assert!(frame.generation > generation);
    assert!(
        frame
            .image
            .pixels
            .iter()
            .all(|p| *p == egui::Color32::BLACK)
    );
    let mut output = context.run_pass(Default::default(), |ctx| {
        texture.update(ctx, "axis", frame);
    });
    assert_eq!(
        output.textures_delta.set.len(),
        1,
        "same-tick replacement must upload new pixels"
    );
    output.textures_delta.clear();
}

#[test]
fn preview_axis_held_hover_does_not_keep_a_finished_transport_playing() {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    playback
        .seek(Tick(9 * TICKS_PER_SECOND), true, session.project().clone())
        .unwrap();
    let hover = Tick(4 * TICKS_PER_SECOND);
    playback.set_hover(Some(hover)).unwrap();
    let port = playback.audio_port();
    port.fill(&mut vec![0.0; 48_000 * 2], 0);
    port.fill(&mut [0.0; 2], 0);
    assert_eq!(playback.position().unwrap(), session.project().duration());
    playback.poll(std::time::Instant::now()).unwrap();
    assert!(
        !playback.playing(),
        "held hover must not suppress the normal end stop"
    );
    assert_eq!(playback.hover(), Some(hover));
    assert_eq!(playback.display_position().unwrap(), hover);
    wait_preview(&mut playback, hover);
    assert!(
        playback
            .frame()
            .unwrap()
            .image
            .pixels
            .iter()
            .all(|p| *p == egui::Color32::RED)
    );
    playback.set_hover(None).unwrap();
    let end = session.project().duration();
    assert_eq!(playback.display_position().unwrap(), end);
    wait_preview(&mut playback, end);
    assert!(
        playback
            .frame()
            .unwrap()
            .image
            .pixels
            .iter()
            .all(|p| *p == egui::Color32::BLACK)
    );
}

#[test]
fn preview_axis_changing_hover_cannot_refresh_a_stalled_callback_clock() {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    playback
        .seek(Tick::ZERO, true, session.project().clone())
        .unwrap();
    let port = playback.audio_port();
    port.fill(&mut [0.0; 960], 0);
    // A controlled monotonic timeline: callback progress establishes its health
    // baseline here; pointer movement cannot assign a different clock to it.
    let baseline = Instant::now() + Duration::from_secs(10);
    playback.poll(baseline).unwrap();
    let at = playback.position().unwrap();
    for millis in [200, 500, 1000] {
        playback
            .set_hover(Some(Tick(millis * TICKS_PER_SECOND / 100)))
            .unwrap();
        playback
            .poll(baseline + Duration::from_millis(millis as u64))
            .unwrap();
        assert!(
            playback.playing(),
            "a callback is healthy through one second"
        );
        assert_eq!(playback.position().unwrap(), at);
    }
    playback.set_hover(Some(Tick(TICKS_PER_SECOND))).unwrap();
    assert!(matches!(
        playback.poll(baseline + Duration::from_millis(1001)),
        Err(vibecut::Error::PreviewUnavailable("audio clock stalled"))
    ));
    assert!(!playback.playing());
    assert_eq!(playback.position().unwrap(), at);
    port.fill(&mut [0.0; 960], 0);
    playback.poll(baseline + Duration::from_secs(2)).unwrap();
    assert_eq!(
        playback.position().unwrap(),
        at,
        "no wall-clock substitution after stop"
    );
}

#[test]
fn preview_axis_hover_cannot_clear_a_current_audio_failure() {
    let (session, mut playback, receive) = controlled_preview(true);
    playback
        .seek(Tick::ZERO, true, session.project().clone())
        .unwrap();
    next_preview_work(&receive).release.send(false).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while playback.error().is_none() {
        assert!(Instant::now() < deadline);
        playback.poll(Instant::now()).unwrap();
        std::thread::yield_now();
    }
    let failure = playback.error().unwrap();
    assert!(matches!(
        failure.as_ref(),
        vibecut::Error::Render(render::Error::Invalid("controlled audio failure"))
    ));
    for hover in [Some(Tick(TICKS_PER_SECOND)), None] {
        playback.set_hover(hover).unwrap();
        assert!(Arc::ptr_eq(
            &playback
                .error()
                .expect("audio failure survives pointer changes"),
            &failure
        ));
    }
    playback
        .seek(Tick::ZERO, false, session.project().clone())
        .unwrap();
    assert!(
        playback.error().is_none(),
        "explicit transport reset clears the audio failure"
    );
}
