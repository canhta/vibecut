#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
#[path = "support/shorts.rs"]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{
    ByteBudget,
    providers::{CatalogOperation, ConsentPrompt, Cost, UploadRequest},
};
use fixture::{Pixels, once, range, ready, source, staged, state, tick, wait};
use std::sync::{Arc, Mutex, mpsc};
use vibecut::{
    egui,
    i18n::Locale,
    shorts::{Action, Context, Decision, State as Shorts},
};
fn by(id: &str) -> By<'_> {
    By::new().predicate(move |n| n.author_id() == Some(id))
}
/// The results grid opens first; a card click enters the quick edit.
fn open_first(h: &mut Harness<'static, Shorts>) {
    h.get(by("shorts.candidate.0")).click();
    h.run_steps(2);
}
thread_local! {
    static OPENED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn harness(app: Shorts, scale: f32) -> Harness<'static, Shorts> {
    OPENED.set(false);
    crate::gpu::builder()
        .with_pixels_per_point(scale)
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_ui_state(
            |ui, a: &mut Shorts| {
                let cx = Context {
                    editor_available: true,
                };
                if vibecut::shorts::show(a, cx, ui).contains(&Action::OpenEditor) {
                    OPENED.set(true);
                }
            },
            app,
        )
}
#[test]
fn actions_and_play_order_reach_preview_and_single_step_editor_insertion() {
    let mut h = harness(ready(), 1.0);
    h.run_steps(3);
    open_first(&mut h);
    let preview = h.state().preview_session().unwrap().project();
    let items = &preview.data().scene().timeline.main.items;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].source_range, range(10, 13));
    assert_eq!(items[1].source_range, range(0, 4));
    assert_eq!(items[1].start, tick(3));
    h.get(by("shorts.accept")).click();
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0].decision, Decision::Accepted);
    assert_eq!(h.state().history_len(), (1, 0));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0].decision, Decision::Pending);
    h.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0].decision, Decision::Accepted);
    h.get(by("shorts.reject")).click();
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0].decision, Decision::Rejected);
    h.get(by("shorts.undo")).click();
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0].decision, Decision::Accepted);
    h.get(by("shorts.redo")).click();
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0].decision, Decision::Rejected);
    h.get(by("shorts.candidate.1")).click();
    h.run_steps(2);
    assert_eq!(h.state().selected(), 1);
    h.get(by("shorts.candidate.0")).click();
    h.run_steps(2);
    assert!(
        h.state()
            .preview_session()
            .unwrap()
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .all(|track| track.kind != project::TrackKind::Text)
    );
    h.get(by("shorts.captions")).click();
    h.run_steps(2);
    assert!(
        h.state()
            .preview_session()
            .unwrap()
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .any(|track| track.kind == project::TrackKind::Text)
    );
    let mut off_center = h.state().reviews()[0].crop;
    off_center.x = 0.0;
    h.state_mut().set_crop(off_center).unwrap();
    h.run_steps(2);
    let row = h.state().reviews()[0].clone();
    h.get(by("shorts.edit")).click();
    h.run_steps(2);
    assert!(OPENED.get());
    let (mut session, settings) = h.state().editor_session().unwrap();
    assert_eq!(session.project().data().canvas.width, 1080);
    assert_eq!(session.project().data().canvas.height, 1920);
    let original = Some(project::CanvasSize {
        width: 320,
        height: 180,
    });
    assert_eq!(session.project().data().original_canvas_size, original);
    assert_eq!(session.history_len(), (1, 0));
    let main = &session.project().data().scene().timeline.main;
    assert_eq!(
        main.items
            .iter()
            .map(|i| i.source_range)
            .collect::<Vec<_>>(),
        row.segments.iter().map(|s| s.range).collect::<Vec<_>>()
    );
    assert!(
        main.items
            .iter()
            .all(|i| settings.placements.get(&i.id).unwrap().crop == row.crop)
    );
    session.undo().unwrap();
    assert!(
        session
            .project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .is_empty()
    );
    assert_eq!(session.project().data().canvas.width, 1080);
    assert_eq!(session.project().data().canvas.height, 1920);
    assert_eq!(session.project().data().original_canvas_size, original);
    session.redo().unwrap();
    assert_eq!(session.project().data().canvas.width, 1080);
    assert_eq!(session.project().data().canvas.height, 1920);
    assert_eq!(session.project().data().original_canvas_size, original);
    assert_eq!(
        session.project().data().scene().timeline.main.items.len(),
        2
    );
    // Typed session edits after handoff, then the services preview still renders.
    let budget = ByteBudget::new(512 << 20).unwrap();
    let mut playback =
        vibecut::services::playback::Playback::new(&session, budget, Arc::new(Pixels)).unwrap();
    playback.set_render_settings(settings.clone()).unwrap();
    for undo in [true, false] {
        if undo {
            session.undo().unwrap();
        } else {
            session.redo().unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while playback.frame().is_none() && playback.error().is_none() {
            playback.poll(std::time::Instant::now()).unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(playback.error().is_none());
    }
    if !testkit::oracle::require("Shorts editor crop export") {
        return;
    }
    // Export through the shared export job after handoff and typed edits. Small
    // output keeps the encoded-frame oracle bounded; crop coordinates stay
    // source-normalized, identical to the full-size portrait project.
    let mut data = session.project().data().clone();
    data.canvas.width = 64;
    data.canvas.height = 114;
    let mut session = engine::Session::new(project::Project::new(data).unwrap(), 100).unwrap();
    let mut settings = settings;
    settings.width = 64;
    settings.height = 114;
    let mut playback = vibecut::services::playback::Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Spatial),
    )
    .unwrap();
    playback.set_render_settings(settings).unwrap();
    // Delete, undo and redo are typed commands on the handed-off session.
    let last = session.project().data().scene().timeline.main.items[1].id;
    session
        .apply(engine::Command::Delete { items: vec![last] })
        .unwrap();
    session.undo().unwrap();
    session.redo().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while playback.frame().is_none() {
        playback.poll(std::time::Instant::now()).unwrap();
        assert!(playback.error().is_none());
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let options = vibecut::export::ExportOptions {
        include_audio: false,
        ..Default::default()
    };
    let mut dialog = vibecut::export::ExportDialog::new(
        playback.budget().clone(),
        playback.render_state().unwrap(),
    )
    .unwrap();
    dialog.set_sources(Arc::new(Spatial)).unwrap();
    let project = session.project().clone();
    dialog
        .start(project.clone(), options, media::EncodePreference::default())
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("off-center.mp4");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        dialog.tick(&project, options).unwrap();
        let state = dialog.inspect();
        if state
            .job
            .as_ref()
            .is_some_and(|j| j.state == engine::jobs::JobState::Completed)
        {
            break;
        }
        assert!(state.error.is_none(), "{:?}", state.error);
        assert!(
            !state
                .job
                .as_ref()
                .is_some_and(|j| j.state == engine::jobs::JobState::Failed),
            "export failed"
        );
        assert!(std::time::Instant::now() < deadline, "crop export timeout");
        std::thread::yield_now();
    }
    dialog.save_to(path.clone(), Vec::new()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        dialog.tick(&project, options).unwrap();
        let state = dialog.inspect();
        if state.saved.is_some() {
            break;
        }
        assert_ne!(
            state.phase,
            vibecut::export::ExportPhase::Failed,
            "{state:?}"
        );
        assert!(std::time::Instant::now() < deadline, "crop save timeout");
        std::thread::yield_now();
    }
    // Independent FFmpeg decoding: average of the encoded first frame must be
    // red (the left crop), rather than the blue default centered placement.
    let decoded = testkit::oracle::command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=1:1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success(), "{:?}", decoded.stderr);
    assert_eq!(decoded.stdout.len(), 3);
    assert!(
        decoded.stdout[0] > 200 && decoded.stdout[2] < 40,
        "wrong crop: {:?}",
        decoded.stdout
    );
    h.state_mut().preferences.locale = Locale::Vi;
    h.run_steps(2);
    assert!(h.query_all(by("shorts.accept")).count() == 1);
}
#[test]
fn safe_trim_and_invalid_bounds_preserve_history_and_source_ranks() {
    let mut app = ready();
    app.trim(0, tick(10), tick(14)).unwrap();
    assert_eq!(app.reviews()[0].segments[0].range, range(10, 14));
    assert_eq!(app.reviews()[0].segments[0].source_order, 1);
    assert_eq!(app.reviews()[0].segments[1].source_order, 0);
    assert_eq!(app.history_len(), (1, 0));
    let before = app.reviews()[0].clone();
    let history = app.history_len();
    for (start, end) in [
        (tick(14), tick(10)),
        (tick(-1), tick(4)),
        (tick(0), tick(11)),
    ] {
        assert!(app.trim(0, start, end).is_err());
        assert_eq!(app.reviews()[0], before);
        assert_eq!(app.history_len(), history);
    }
    app.undo();
    assert_eq!(app.reviews()[0].segments[0].range, range(10, 13));
    app.redo();
    assert_eq!(app.reviews()[0], before);
    // A start inside a spoken unit is within the one-second tolerance of its edge,
    // so the trim snaps to that safe frame instead of refusing.
    app.trim(0, tick(11), tick(14)).unwrap();
    assert_eq!(app.reviews()[0].segments[0].range, range(10, 14));
    let mut h = harness(app, 1.0);
    h.run_steps(3);
    open_first(&mut h);
    h.get(by("shorts.tool.trim")).click();
    h.run_steps(2);
    h.get(by("shorts.segment.0.in")).click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get(by("shorts.segment.0.in")).type_text("NaN");
    h.run_steps(2);
    h.get(by("shorts.segment.0.trim")).click();
    h.run_steps(2);
    assert!(h.state().error().is_some());
    assert_eq!(h.state().reviews()[0], before);
    for (locale, out) in [(Locale::En, "1,014.000"), (Locale::Vi, "1.014,000")] {
        let mut app = state(
            vibecut::Look {
                locale,
                ..Default::default()
            },
            once(fixture::source_at(1000)).0,
            1000,
        );
        app.captions = false;
        app.pick_video(std::env::temp_dir().join("long-video.mp4"))
            .unwrap();
        app.generate().unwrap();
        wait(&mut app);
        let mut h = harness(app, 1.0);
        h.run_steps(3);
        open_first(&mut h);
        h.get(by("shorts.tool.trim")).click();
        h.run_steps(2);
        // Apply catalog-formatted grouped values without editing them first.
        h.get(by("shorts.segment.0.trim")).click();
        h.run_steps(2);
        assert!(h.state().error().is_none());
        h.get(by("shorts.segment.0.out")).click();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get(by("shorts.segment.0.out")).type_text(out);
        h.run_steps(2);
        h.get(by("shorts.segment.0.trim")).click();
        h.run_steps(2);
        assert!(h.state().error().is_none());
        assert_eq!(h.state().reviews()[0].segments[0].range, range(1010, 1014));
        assert_eq!(h.state().history_len(), (1, 0));
        h.get(by("shorts.locale")).click();
        h.run_steps(3);
        h.get(by("shorts.segment.0.trim")).click();
        h.run_steps(2);
        assert!(
            h.state().error().is_none(),
            "drafts must follow the new locale"
        );
        assert_eq!(h.state().reviews()[0].segments[0].range, range(1010, 1014));
        h.get(by("shorts.segment.0.out")).click();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get(by("shorts.segment.0.out")).type_text("1.01,4");
        h.run_steps(2);
        h.get(by("shorts.segment.0.trim")).click();
        h.run_steps(2);
        assert!(h.state().error().is_some());
        assert_eq!(h.state().reviews()[0].segments[0].range, range(1010, 1014));
    }
}
#[test]
fn crop_gesture_reversal_and_bounds_are_one_review_undo_step() {
    let mut h = harness(ready(), 1.0);
    h.run_steps(3);
    open_first(&mut h);
    h.get(by("shorts.tool.layout")).click();
    h.run_steps(2);
    let initial = h.state().reviews()[0].clone();
    let from = h.get(by("shorts.crop")).rect().center();
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(from + egui::vec2(50.0, 0.0));
    h.run_steps(2);
    h.hover_at(from - egui::vec2(30.0, 0.0));
    h.run_steps(2);
    h.drop_at(from - egui::vec2(30.0, 0.0));
    h.run_steps(2);
    assert!(h.state().reviews()[0].crop.x < initial.crop.x);
    assert_eq!(h.state().history_len(), (1, 0));
    h.get(by("shorts.undo")).click();
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0], initial);
    let history = h.state().history_len();
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(from + egui::vec2(40.0, 0.0));
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    h.drop_at(from + egui::vec2(50.0, 0.0));
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0], initial);
    assert_eq!(h.state().history_len(), history);
    h.get(by("shorts.crop")).click();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    assert!(h.state().reviews()[0].crop.x > initial.crop.x);
    h.get(by("shorts.undo")).click();
    h.run_steps(2);
    assert_eq!(h.state().reviews()[0], initial);
    let mut invalid = initial.crop;
    invalid.x = 1.0;
    assert!(h.state_mut().set_crop(invalid).is_err());
    assert_eq!(h.state().reviews()[0], initial);
    for (width, height) in [
        (0.0, 10.0),
        (f64::NAN, 10.0),
        (10.0, f64::INFINITY),
        (f64::MAX, f64::MIN_POSITIVE),
    ] {
        assert!(vibecut::services::crop::portrait(width, height).is_err());
    }
    // A corner drag keeps the portrait crop's 9:16 source shape.
    let crop = h.state().reviews()[0].crop;
    let rect = h.get(by("shorts.crop")).rect();
    let corner = egui::pos2(
        rect.left() + rect.width() * (crop.x + crop.width) as f32,
        rect.top() + rect.height() * (crop.y + crop.height) as f32,
    );
    h.hover_at(corner);
    h.run_steps(1);
    h.drag_at(corner);
    h.run_steps(1);
    h.hover_at(corner - egui::vec2(30.0, 30.0));
    h.run_steps(2);
    h.drop_at(corner - egui::vec2(30.0, 30.0));
    h.run_steps(2);
    let resized = h.state().reviews()[0].crop;
    assert!(resized.width < crop.width);
    assert!(
        (resized.width / resized.height * (320.0 / 180.0) - 9.0 / 16.0).abs() <= 1e-6,
        "{resized:?}"
    );
    assert!(resized.valid());
}
#[test]
fn accepted_export_is_reachable_and_failed_jobs_can_be_retried_or_cancelled() {
    let mut h = harness(ready(), 1.0);
    h.run_steps(3);
    open_first(&mut h);
    h.get(by("shorts.accept")).click();
    h.run_steps(2);
    let output = tempfile::tempdir().unwrap();
    h.state_mut().export_to(output.path().to_owned()).unwrap();
    wait(h.state_mut());
    h.run_steps(3);
    assert_eq!(h.state().batch().unwrap().len(), 1);
    assert!(
        h.query_by_label(vibecut::i18n::text(
            Locale::En,
            vibecut::i18n::Text::ShortsSourceUnavailable
        ))
        .is_some()
    );
    assert_eq!(
        h.state().batch_snapshot(0).unwrap().state,
        engine::jobs::JobState::Failed
    );
    assert!(!h.state().batch().unwrap().paths(0).unwrap().video.exists());
    h.get(by("shorts.export.0.retry")).click();
    h.run_steps(1);
    h.state_mut().cancel();
    wait(h.state_mut());
    assert!(matches!(
        h.state().batch_snapshot(0).unwrap().state,
        engine::jobs::JobState::Cancelled | engine::jobs::JobState::Failed
    ));
}
/// A suggestion that asks the run's consent for one exact upload before it finishes.
fn approval(
    unknown_price: bool,
    entered: mpsc::SyncSender<()>,
    approved: Arc<std::sync::atomic::AtomicBool>,
) -> vibecut::services::ai_access::Opener {
    let entered = Mutex::new(entered);
    staged(move |consent: &dyn ConsentPrompt, _| {
        entered.lock().unwrap().send(()).unwrap();
        let request = UploadRequest {
            config: ai::ProviderConfig::new(
                "injected".into(),
                "explicit-model".into(),
                "fixture".into(),
                ai::Location::Remote {
                    backend: "fixture".into(),
                    base_url: "https://fixture.invalid".into(),
                    auth_ref: "fixture".into(),
                },
            )
            .unwrap(),
            operation: CatalogOperation::SuggestClips,
            provider: "injected".into(),
            model: "explicit-model".into(),
            languages: "vi".into(),
            audio_seconds: 0.0,
            upload_bytes: 123,
            cost: if unknown_price {
                Cost::Unknown
            } else {
                Cost::Free
            },
            terms: vec![],
        };
        if unknown_price {
            consent.approve_unknown_price(&request)?;
        } else {
            consent.approve(&request)?;
        }
        approved.store(true, std::sync::atomic::Ordering::Release);
        Ok(source())
    })
}
#[test]
fn exact_upload_requires_explicit_approval_and_cancellation_interrupts_the_prompt() {
    let (opener, queued) = once(source());
    let mut app = state(Default::default(), opener, 0);
    app.pick_video(std::env::temp_dir().join("queued.mp4"))
        .unwrap();
    app.generate().unwrap();
    app.cancel();
    wait(&mut app);
    assert!(app.source().is_none());
    assert!(
        queued.lock().unwrap().is_some(),
        "queued cancellation must not call the generator"
    );

    for (locale, unknown_price, scale) in [
        (Locale::En, false, 1.0),
        (Locale::En, true, 1.0),
        (Locale::Vi, true, 1.0),
        (Locale::En, true, 2.0),
        (Locale::Vi, true, 2.0),
    ] {
        for decision in [Some(true), Some(false), None] {
            let (send, receive) = mpsc::sync_channel(1);
            let approved = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let mut app = state(
                Default::default(),
                approval(unknown_price, send, approved.clone()),
                0,
            );
            app.preferences.locale = locale;
            app.captions = false;
            app.pick_video(std::env::temp_dir().join("video.mp4"))
                .unwrap();
            let mut h = harness(app, scale);
            h.run_steps(3);
            h.get(by("shorts.generate")).click();
            h.run_steps(2);
            receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while h.query_all(by("shorts.consent.approve")).count() == 0 {
                h.run_steps(1);
                assert!(std::time::Instant::now() < deadline);
            }
            assert!(!approved.load(std::sync::atomic::Ordering::Acquire));
            if locale == Locale::En {
                assert!(h.query_by_label("Size: 123").is_some());
            }
            assert!(h.query_by_label("injected · explicit-model · vi").is_some());
            if unknown_price {
                assert!(
                    h.query_by_label(vibecut::i18n::Text::ShortsUnknownCost.text(locale))
                        .is_some()
                );
                assert!(
                    h.get(by("shorts.consent.approve"))
                        .accesskit_node()
                        .is_disabled()
                );
                if decision == Some(true) {
                    let output = std::env::var_os("CARGO_TARGET_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| {
                            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
                        })
                        .join("ui-audit");
                    std::fs::create_dir_all(&output).unwrap();
                    h.render()
                        .unwrap()
                        .save(output.join(format!("shorts-unknown-price-{locale:?}@{scale}x.png")))
                        .unwrap();
                    h.get(by("shorts.consent.unknown_price")).click();
                    h.run_steps(2);
                    assert!(
                        !h.get(by("shorts.consent.approve"))
                            .accesskit_node()
                            .is_disabled()
                    );
                }
            }
            match decision {
                Some(true) => h.get(by("shorts.consent.approve")).click(),
                Some(false) => h.get(by("shorts.consent.deny")).click(),
                None => h.get(by("shorts.cancel")).click(),
            };
            h.run_steps(1);
            wait(h.state_mut());
            h.run_steps(2);
            assert_eq!(
                approved.load(std::sync::atomic::Ordering::Acquire),
                decision == Some(true)
            );
            assert_eq!(h.state().source().is_some(), decision == Some(true));
        }
    }
}

#[test]
fn home_card_opens_a_dedicated_viewport_and_native_drop_selects_one_video() {
    let directory = tempfile::tempdir().unwrap();
    let mut app =
        vibecut::app::Workspace::new(directory.path().join("recovery"), Default::default(), None)
            .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.busy() {
        app.tick();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_ui_state(
            |ui, a: &mut vibecut::app::Workspace| {
                a.tick();
                if let Some(id) = a.focused_project() {
                    a.window_ui(id, ui);
                } else {
                    a.ui(ui);
                }
            },
            app,
        );
    h.run_steps(3);
    h.get(by("home.shorts")).click();
    h.run_steps(2);
    assert!(h.state().shorts.is_some());
    assert!(h.state().focused_editor().is_none());
    assert!(h.query_all(by("shorts.pick")).count() == 1);
    h.state_mut().shorts = Some(ready());
    h.run_steps(3);
    h.get(by("shorts.candidate.0")).click();
    h.run_steps(2);
    h.get(by("shorts.edit")).click();
    h.run_steps(2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while h.state().busy() {
        h.run_steps(1);
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        h.state()
            .focused_editor()
            .unwrap()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .len(),
        2
    );
    assert!(
        h.state_mut()
            .focused_editor_mut()
            .unwrap()
            .session
            .autosave()
            .is_some()
    );
    #[derive(Debug)]
    struct DropFile(std::path::PathBuf);
    impl egui::DroppedFile for DropFile {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> std::result::Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }
    let path = directory.path().join("cuộc trò chuyện.mp4");
    let mut h = harness(fixture::app(), 1.0);
    h.state_mut().request.transcription_provider = "unavailable".into();
    h.run_steps(2);
    assert_eq!(h.state().request.transcription_provider, "unavailable");
    h.state_mut().request.transcription_provider = "whisper.cpp".into();
    h.input_mut()
        .dropped_files
        .push(Arc::new(DropFile(path.clone())));
    h.run_steps(1);
    assert_eq!(h.state().request.path, path);
    assert!(h.state().source().is_none());
    h.state_mut().generate().unwrap();
    assert!(
        h.state_mut()
            .pick_video(directory.path().join("replacement.mp4"))
            .is_err()
    );
    wait(h.state_mut());
    assert_eq!(h.state().request.path, path);
    for invalid in [
        std::path::PathBuf::new(),
        std::path::PathBuf::from("x".repeat(32769)),
    ] {
        assert!(h.state_mut().pick_video(invalid).is_err());
        assert_eq!(h.state().request.path, path);
    }
}

struct Spatial;
impl vibecut::services::playback::Sources for Spatial {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Spatial)
    }
}
impl render::SourceProvider for Spatial {
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
        _: time::Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        Ok(Some(render::Image::from_working_pixels(
            320,
            180,
            (0..320 * 180).map(|n| {
                if n % 320 < 120 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                }
            }),
            hooks,
        )?))
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
        render::AudioBlock::new(rate, 2, frames, render::MemoryKind::Decoded, hooks)
    }
}

#[test]
fn workspace_shorts_raw_text_paste_reaches_the_real_trim_field() {
    let root = tempfile::tempdir().unwrap();
    let mut app =
        vibecut::app::Workspace::new(root.path().join("recovery"), Default::default(), None)
            .unwrap();
    app.open_shorts().unwrap();
    app.request_close();
    app.shorts = Some(ready());
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_ui_state(|ui, a: &mut vibecut::app::Workspace| a.ui(ui), app);
    h.run_steps(3);
    h.get(by("shorts.candidate.0")).click();
    h.run_steps(2);
    h.get(by("shorts.tool.trim")).click();
    h.run_steps(2);
    h.get(by("shorts.segment.0.out")).click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.state_mut().text_clipboard =
        vibecut::services::text_clipboard::TextClipboard::new(|| Some("14.000".into()));
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::V,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.step();
    assert!(
        !h.output()
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
    );
    h.run_steps(2);
    assert_eq!(
        h.state().shorts.as_ref().unwrap().reviews()[0].segments[0].range,
        range(10, 13)
    );
    h.get(by("shorts.segment.0.trim")).click();
    h.run_steps(2);
    assert_eq!(
        h.state().shorts.as_ref().unwrap().reviews()[0].segments[0].range,
        range(10, 14)
    );
}

/// A scripted analysis: the first answer is a running job, the next the shots of two
/// people on screen together over the whole fixture.
fn two_people(
    calls: Arc<std::sync::atomic::AtomicUsize>,
    gate: Arc<std::sync::atomic::AtomicBool>,
) -> vibecut::shorts::ShotSource {
    use engine::control::ClipShotsDiagnostic::Pending;
    use std::sync::atomic::Ordering;
    Arc::new(move |runner, _, _, _| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let key = engine::cache::CacheKey::new(engine::cache::KeyInputs {
                kind: engine::cache::ArtifactKind::Preview,
                source: [0; 32],
                provider: "fixture",
                model: "shots",
                version: "1",
                settings: &[],
                transcript: [0; 32],
                cues: [0; 32],
                clip_range: [0; 32],
                translation: [0; 32],
            })?;
            let gate = gate.clone();
            let (job, _) = runner.submit::<engine::control::Error>(
                engine::jobs::JobSpec {
                    kind: engine::jobs::JobKind::AnalyzeShots,
                    class: engine::jobs::ResourceClass::Cpu,
                    input: key,
                    bytes: Default::default(),
                },
                move |context| {
                    while !gate.load(Ordering::SeqCst) {
                        context.check_cancelled()?;
                        std::thread::yield_now();
                    }
                    Ok(Vec::new())
                },
            )?;
            return Ok((Vec::new(), Some(Pending { job })));
        }
        let person = |id: u32, x: f32| engine::FaceTrack {
            identity: None,
            id,
            bounds: [x, 70.0, 20.0, 40.0],
            observed_frames: 20,
        };
        Ok((
            vec![engine::ShotAnalysis {
                range: range(0, 20),
                people_count: 2,
                tracks: vec![person(0, 70.0), person(1, 230.0)],
                speaker: engine::SpeakerState::Unknown,
                continuity_hint: None,
                scene: engine::Scene::Camera,
                seconds: Vec::new(),
            }],
            None,
        ))
    })
}

#[test]
fn a_pending_analysis_refreshes_the_preview_to_a_split_layout_when_its_job_finishes() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (calls, gate) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let mut app = fixture::app();
    app.with_shot_source(two_people(calls.clone(), gate.clone()));
    app.generate().unwrap();
    wait(&mut app);
    // While the analysis job runs the preview has no evidence: the centred Fill.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let tracks = |app: &Shorts| {
        app.preview_session()
            .unwrap()
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == project::TrackKind::Video)
            .count()
    };
    assert_eq!(tracks(&app), 1);
    gate.store(true, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while calls.load(Ordering::SeqCst) < 2 {
        app.tick();
        assert!(std::time::Instant::now() < deadline, "analysis timeout");
        std::thread::yield_now();
    }
    app.tick();
    // The refreshed preview lays the two people out as a Split: a second video track.
    assert_eq!(tracks(&app), 2);
    let preview = app.preview_session().unwrap().project();
    let items = &preview.data().scene().timeline.main.items;
    assert_eq!(items[0].source_range, range(10, 13));
    assert!(app.error().is_none());
}

#[test]
fn a_lent_detector_without_installed_weights_reports_the_face_data_missing() {
    let root = tempfile::tempdir().unwrap();
    let media =
        engine::control::MediaAccess::open_in(root.path(), ByteBudget::new(64 << 20).unwrap())
            .unwrap();
    // The loader every composition root lends refuses a missing bundle with this typed error.
    let missing = engine::control::VisionAccess::new(|| {
        Err(engine::control::Error::Providers(
            engine::providers::Error::ModelMissing("yunet-2026may"),
        ))
    });
    let mut app = fixture::app();
    app.lend_analysis(Some(Arc::new(Mutex::new(media))), Some(Arc::new(missing)));
    app.generate().unwrap();
    wait(&mut app);
    let mut h = harness(app, 1.0);
    h.run_steps(3);
    let said = |text| {
        h.query_by_label(vibecut::i18n::text(Locale::En, text))
            .is_some()
    };
    assert!(said(vibecut::i18n::Text::ShortsShotsModels));
    assert!(!said(vibecut::i18n::Text::ShortsShotsUnavailable));
}

#[test]
fn an_unavailable_detector_is_reported_once_and_can_be_dismissed() {
    let mut app = fixture::app();
    app.lend_analysis(None, None);
    app.generate().unwrap();
    wait(&mut app);
    let mut h = harness(app, 1.0);
    h.run_steps(3);
    let count = |h: &Harness<'static, Shorts>| h.query_all(by("shorts.shots.dismiss")).count();
    assert_eq!(count(&h), 1);
    h.get(by("shorts.shots.dismiss")).click();
    h.run_steps(3);
    assert_eq!(count(&h), 0);
}

#[test]
fn a_busy_model_cache_is_asked_again_until_the_answer_clears() {
    use engine::control::ClipShotsDiagnostic;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut app = fixture::app();
    // The download finishes after the second ask: busy, busy, then an answer without evidence.
    app.with_shot_source(Arc::new(move |_, _, _, _| {
        let ask = seen.fetch_add(1, Ordering::SeqCst);
        Ok((
            Vec::new(),
            (ask < 2).then_some(ClipShotsDiagnostic::ModelsBusy),
        ))
    }));
    app.generate().unwrap();
    wait(&mut app);
    // The first answer is busy: the toast says so and no thread waits.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut h = harness(app, 1.0);
    h.run_steps(3);
    let busy = vibecut::i18n::text(Locale::En, vibecut::i18n::Text::ShortsShotsBusy);
    assert!(h.query_by_label(busy).is_some());
    // The bounded re-ask runs from `tick` until the busy answer clears, then the notice goes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while calls.load(Ordering::SeqCst) < 3 {
        h.run_steps(2);
        assert!(std::time::Instant::now() < deadline, "re-ask timeout");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    h.run_steps(3);
    assert!(h.query_by_label(busy).is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

/// The managed library's stand-in: runs kept in memory, newest first.
#[derive(Default)]
struct MemoryRuns(Mutex<Vec<(project::ProjectId, project::Project)>>);
impl vibecut::shorts::RunStore for MemoryRuns {
    fn list(&self) -> engine::control::Result<Vec<vibecut::shorts::RunEntry>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(|(id, project)| {
                let run = project.data().shorts_run.as_ref().unwrap();
                vibecut::shorts::RunEntry {
                    id: *id,
                    source_name: run.source_name.as_str().to_owned(),
                    clips: run.clips.len(),
                    poster: project::Thumbnail::Unavailable,
                    modified_unix_ms: 0,
                }
            })
            .collect())
    }
    fn save(
        &self,
        project: project::Project,
        _: project::Thumbnail,
    ) -> engine::control::Result<project::ProjectId> {
        assert!(project.data().shorts_run.is_some());
        let id = project.data().id;
        self.0.lock().unwrap().push((id, project));
        Ok(id)
    }
    fn load(&self, id: project::ProjectId) -> engine::control::Result<project::Project> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(kept, _)| *kept == id)
            .unwrap()
            .1
            .clone())
    }
    fn remove(&self, id: project::ProjectId) -> engine::control::Result<()> {
        self.0.lock().unwrap().retain(|(kept, _)| *kept != id);
        Ok(())
    }
}

fn run_window(
    path: &std::path::Path,
    runs: Arc<MemoryRuns>,
    opener: vibecut::services::ai_access::Opener,
) -> Shorts {
    let path = path.to_owned();
    let mut app = Shorts::with_access(
        vibecut::preferences::Preferences::default().look(),
        ByteBudget::new(512 << 20).unwrap(),
        opener,
        Arc::new(move |_| Ok(fixture::asset_in(0, path.clone()))),
        Arc::new(fixture::Pixels),
    )
    .unwrap();
    app.request.min_seconds = 1;
    app.request.max_seconds = 20;
    app.captions = false;
    app.lend_runs(runs);
    app
}

fn tick_until(app: &mut Shorts, done: impl Fn(&Shorts) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !done(app) {
        app.tick();
        assert!(std::time::Instant::now() < deadline, "runs timeout");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a_finished_run_is_kept_and_its_recent_entry_reopens_the_results() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("interview.mp4");
    std::fs::write(&path, b"video").unwrap();
    let runs = Arc::new(MemoryRuns::default());
    let mut first = run_window(&path, runs.clone(), once(fixture::source()).0);
    first.pick_video(path.clone()).unwrap();
    first.generate().unwrap();
    wait(&mut first);
    tick_until(&mut first, |app| app.runs().len() == 1);
    assert_eq!(first.runs()[0].source_name, "interview.mp4");
    assert_eq!(first.runs()[0].clips, 2);
    // A new window finds the run in the library and reopens its results without ASR or ranking.
    let mut second = run_window(&path, runs.clone(), once(fixture::source()).0);
    tick_until(&mut second, |app| app.runs().len() == 1);
    let id = second.runs()[0].id;
    second.open_run(id).unwrap();
    tick_until(&mut second, |app| app.source().is_some());
    assert!(second.error().is_none());
    assert_eq!(second.reviews().len(), 2);
    let kept = &second.source().unwrap().candidates;
    let made = &fixture::source().candidates;
    assert_eq!(kept[0].proposal.title, made[0].proposal.title);
    assert_eq!(kept[0].segments[0].range, made[0].segments[0].range);
    assert_eq!(kept[1].proposal.score, made[1].proposal.score);
    // The marker stays in the library: the in-memory source is an ordinary project.
    assert!(second.source().unwrap().project.data().shorts_run.is_none());
    // Removing the entry forgets the run and leaves the video where it is.
    second.remove_run(id);
    tick_until(&mut second, |app| app.runs().is_empty());
    assert!(path.is_file());
}

#[test]
fn a_run_whose_video_moved_reports_it_instead_of_opening() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("interview.mp4");
    std::fs::write(&path, b"video").unwrap();
    let runs = Arc::new(MemoryRuns::default());
    let mut first = run_window(&path, runs.clone(), once(fixture::source()).0);
    first.pick_video(path.clone()).unwrap();
    first.generate().unwrap();
    wait(&mut first);
    tick_until(&mut first, |app| app.runs().len() == 1);
    std::fs::remove_file(&path).unwrap();
    let mut second = run_window(&path, runs, once(fixture::source()).0);
    tick_until(&mut second, |app| app.runs().len() == 1);
    let id = second.runs()[0].id;
    second.open_run(id).unwrap();
    tick_until(&mut second, |app| app.error().is_some());
    assert!(second.source().is_none());
}

#[test]
fn the_back_arrow_returns_to_the_entry_and_the_run_keeps_going() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let gate = Arc::new(AtomicBool::new(false));
    let hold = gate.clone();
    let opener = fixture::staged(move |_, hooks| {
        while !hold.load(Ordering::SeqCst) {
            hooks.check_cancelled()?;
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(fixture::source())
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("interview.mp4");
    std::fs::write(&path, b"video").unwrap();
    let mut app = run_window(&path, Arc::new(MemoryRuns::default()), opener);
    app.pick_video(path).unwrap();
    app.generate().unwrap();
    let mut h = harness(app, 1.0);
    h.run_steps(3);
    // The working page: the skeleton grid with a live count of zero, the arrow and Cancel.
    assert!(h.query_by(by("shorts.back")).is_some());
    assert!(h.query_by(by("shorts.cancel")).is_some());
    assert_eq!(h.state().generation_count(), Some(0));
    h.get(by("shorts.back")).click();
    h.run_steps(3);
    // The entry lists the run in progress and still runs it.
    assert!(h.query_by(by("shorts.back")).is_none());
    assert!(h.state().busy());
    h.get(by("shorts.run.0")).click();
    h.run_steps(3);
    assert!(h.query_by(by("shorts.back")).is_some());
    // Leaving again and finishing while away lands on the results.
    h.get(by("shorts.back")).click();
    h.run_steps(2);
    gate.store(true, Ordering::SeqCst);
    wait(h.state_mut());
    h.run_steps(3);
    assert_eq!(h.state().reviews().len(), 2);
    assert!(h.query_by(by("shorts.candidate.0")).is_some());
}
