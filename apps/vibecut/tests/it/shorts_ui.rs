#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
#[path = "support/shorts.rs"]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{
    ByteBudget,
    providers::{CatalogOperation, ConsentPrompt, Cost, UploadRequest},
    transcript::Cancellation,
};
use fixture::{Once, Pixels, range, ready, source, tick, wait};
use std::sync::{Arc, Mutex, mpsc};
use vibecut::{
    catalog::Locale,
    egui,
    shorts::{
        Action, Context, Decision, GenerationRequest, Generator, ReviewSource, State as Shorts,
        TaskEvents,
    },
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
fn harness(app: Shorts) -> Harness<'static, Shorts> {
    OPENED.set(false);
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_ui_state(
            |ui, a: &mut Shorts| {
                let cx = Context {
                    editor_available: true,
                };
                if vibecut::shorts::show(a, &cx, ui).contains(&Action::OpenEditor) {
                    OPENED.set(true);
                }
            },
            app,
        )
}
#[test]
fn actions_and_play_order_reach_preview_and_single_step_editor_insertion() {
    let mut h = harness(ready());
    h.run_steps(3);
    open_first(&mut h);
    let preview = h.state().preview().unwrap().session.project();
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
            .preview()
            .unwrap()
            .session
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
            .preview()
            .unwrap()
            .session
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
    let budget = ByteBudget::new(512 << 20).unwrap();
    let mut playback =
        vibecut::playback::Playback::new(&session, budget, Arc::new(Pixels)).unwrap();
    playback.set_render_settings(settings.clone()).unwrap();
    let mut editor = vibecut::Editor::new(session, playback, Default::default());
    for command in ["edit.undo", "edit.redo"] {
        editor.activate(command).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while editor.playback.frame().is_none() && editor.playback.error().is_none() {
            editor.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(editor.playback.error().is_none());
    }
    if !testkit::oracle::require("Shorts editor crop export") {
        return;
    }
    // Export through the real editor dialog after handoff and edit undo/redo.
    // Small output keeps the encoded-frame oracle bounded; crop coordinates stay
    // source-normalized, identical to the full-size portrait project.
    let mut data = editor.session.project().data().clone();
    data.canvas.width = 64;
    data.canvas.height = 114;
    let session = engine::Session::new(project::Project::new(data).unwrap(), 100).unwrap();
    let mut playback = vibecut::playback::Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Spatial),
    )
    .unwrap();
    let mut settings = settings;
    settings.width = 64;
    settings.height = 114;
    playback.set_render_settings(settings).unwrap();
    let mut editor = vibecut::Editor::new(session, playback, Default::default());
    editor.preferences.export.include_audio = false;
    editor.activate("file.export").unwrap();
    let last = editor.session.project().data().scene().timeline.main.items[1].id;
    editor.selection = std::collections::BTreeSet::from([last]);
    editor.activate("timeline.delete").unwrap();
    editor.activate("edit.undo").unwrap();
    editor.activate("edit.redo").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while editor.playback.frame().is_none() {
        editor.tick();
        assert!(editor.playback.error().is_none());
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    editor
        .export
        .as_mut()
        .unwrap()
        .set_sources(Arc::new(Spatial))
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("off-center.mp4");
    let mut exported = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_ui_state(|ui, a: &mut vibecut::Editor| a.ui(ui), editor);
    exported.run_steps(3);
    exported.get(by("export.run")).click();
    exported.run_steps(2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        exported.state_mut().tick();
        let state = exported.state().export.as_ref().unwrap().inspect();
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
    exported
        .state_mut()
        .export_picked_file(path.clone())
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        exported.state_mut().tick();
        let state = exported.state().export.as_ref().unwrap().inspect();
        if !state.open {
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
        (tick(11), tick(14)),
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
    let mut h = harness(app);
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
        let mut app = Shorts::with_generator(
            vibecut::catalog::Preferences {
                locale,
                ..Default::default()
            },
            ByteBudget::new(512 << 20).unwrap(),
            Arc::new(Once(Mutex::new(Some(fixture::source_at(1000))))),
            Arc::new(Pixels),
        )
        .unwrap();
        app.captions = false;
        app.pick_video(std::env::temp_dir().join("long-video.mp4"))
            .unwrap();
        app.generate().unwrap();
        wait(&mut app);
        let mut h = harness(app);
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
    let mut h = harness(ready());
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
        assert!(vibecut::crop::portrait(width, height).is_err());
    }
}
#[test]
fn accepted_export_is_reachable_and_failed_jobs_can_be_retried_or_cancelled() {
    let mut h = harness(ready());
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
        h.query_by_label(vibecut::catalog::text(
            Locale::En,
            vibecut::catalog::Text::ShortsSourceUnavailable
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
struct ApprovalGenerator {
    entered: Mutex<mpsc::SyncSender<()>>,
    approved: Arc<std::sync::atomic::AtomicBool>,
}
impl Generator for ApprovalGenerator {
    fn run(
        &self,
        _: GenerationRequest,
        _: ByteBudget,
        events: &TaskEvents,
        _: &Cancellation,
    ) -> engine::Result<ReviewSource> {
        self.entered.lock().unwrap().send(()).unwrap();
        events.approve(&UploadRequest {
            operation: CatalogOperation::SuggestClips,
            provider: "injected".into(),
            model: "explicit-model".into(),
            languages: "vi".into(),
            audio_seconds: 0.0,
            upload_bytes: 123,
            cost: Cost::Free,
            terms: vec![],
        })?;
        self.approved
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(source())
    }
}
#[test]
fn exact_upload_requires_explicit_approval_and_cancellation_interrupts_the_prompt() {
    let queued = Arc::new(Once(Mutex::new(Some(source()))));
    let mut app = Shorts::with_generator(
        Default::default(),
        ByteBudget::new(512 << 20).unwrap(),
        queued.clone(),
        Arc::new(Pixels),
    )
    .unwrap();
    app.pick_video(std::env::temp_dir().join("queued.mp4"))
        .unwrap();
    app.generate().unwrap();
    app.cancel();
    wait(&mut app);
    assert!(app.source().is_none());
    assert!(
        queued.0.lock().unwrap().is_some(),
        "queued cancellation must not call the generator"
    );

    for decision in [Some(true), Some(false), None] {
        let (send, receive) = mpsc::sync_channel(1);
        let approved = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = Shorts::with_generator(
            Default::default(),
            ByteBudget::new(512 << 20).unwrap(),
            Arc::new(ApprovalGenerator {
                entered: Mutex::new(send),
                approved: approved.clone(),
            }),
            Arc::new(Pixels),
        )
        .unwrap();
        app.captions = false;
        app.pick_video(std::env::temp_dir().join("video.mp4"))
            .unwrap();
        let mut h = harness(app);
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
        assert!(h.query_by_label("Size: 123").is_some());
        assert!(h.query_by_label("injected · vi").is_some());
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

#[test]
fn home_card_opens_a_dedicated_viewport_and_native_drop_selects_one_video() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = vibecut::workspace::Workspace::new(
        directory.path().join("recovery"),
        Default::default(),
        None,
    )
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
            |ui, a: &mut vibecut::workspace::Workspace| {
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
    let mut h = harness(fixture::app());
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
impl vibecut::playback::Sources for Spatial {
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
