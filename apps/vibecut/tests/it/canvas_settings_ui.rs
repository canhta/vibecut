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
use project::{AspectPreset, CanvasSize, Project, ProjectId};
use std::sync::Arc;
use vibecut::{
    Editor, egui,
    preferences::Preferences,
    services::color_picker::palette,
    services::playback::{Files, Playback},
};

fn editor() -> Editor {
    let mut data = Project::empty(ProjectId(1), "Canvas".into())
        .unwrap()
        .into_data();
    data.original_canvas_size = Some(CanvasSize {
        width: 1080,
        height: 1920,
    });
    let session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn enter(harness: &mut Harness<Editor>, id: &str, value: &str) {
    click(harness, id);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(value.to_owned()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
}

fn has_widget(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}

fn field_value(harness: &Harness<Editor>, id: &str) -> String {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .value()
        .unwrap()
        .to_owned()
}

fn canvas(harness: &Harness<Editor>) -> project::Canvas {
    harness.state().session.project().data().canvas.clone()
}

#[test]
fn canvas_settings_pick_preset_custom_fps_swatch_reset_and_persist() {
    let mut harness = build(editor());
    harness.run_steps(3);
    assert_eq!(canvas(&harness).preset, Some(AspectPreset::Landscape));
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1920, 1080)
    );

    // A preset commits one history step.
    let before = canvas(&harness);
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.ratio");
    click(&mut harness, "inspector.canvas.ratio.9x16");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1080, 1920)
    );
    assert_eq!(canvas(&harness).preset, Some(AspectPreset::Portrait));
    assert_eq!(harness.state().session.history_len().0, history + 1);
    assert_ne!(before, canvas(&harness));

    // A custom dimension marks custom mode, remembers the last custom size and
    // commits exactly one step.
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.width", "1000");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1000, 1920)
    );
    assert_eq!(canvas(&harness).preset, None);
    assert_eq!(
        canvas(&harness).last_custom_size,
        CanvasSize {
            width: 1000,
            height: 1920
        }
    );
    assert_eq!(harness.state().session.history_len().0, history + 1);

    // A zero dimension is refused without a history step or project change.
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.height", "0");
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(harness.state().session.history_len().0, history);
    enter(&mut harness, "inspector.canvas.height", "700");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1000, 700)
    );

    // Odd custom sizes commit the nearest even value (ties up), one at a time,
    // and each field shows the committed value.
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.width", "1081");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1082, 700)
    );
    assert_eq!(field_value(&harness, "inspector.canvas.width"), "1082");
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.height", "721");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1082, 722)
    );
    assert_eq!(field_value(&harness, "inspector.canvas.height"), "722");
    assert_eq!(harness.state().session.history_len().0, history + 1);
    assert_eq!(
        canvas(&harness).last_custom_size,
        CanvasSize {
            width: 1082,
            height: 722
        }
    );

    // The integer select writes the exact rational, one step per pick.
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.fps");
    click(&mut harness, "inspector.canvas.fps.60_1");
    assert_eq!(canvas(&harness).fps, time::FrameRate::FPS_60);
    assert_eq!(harness.state().session.history_len().0, history + 1);

    // A fractional source rate keeps its exact rational and its own selected
    // row; picking it neither rounds nor adds history.
    harness
        .state_mut()
        .session
        .apply(Command::SetFps {
            num: 24000,
            den: 1001,
        })
        .unwrap();
    harness.run_steps(3);
    assert_eq!(canvas(&harness).fps, time::FrameRate::FPS_23_976);
    click(&mut harness, "inspector.canvas.fps");
    assert!(has_widget(&harness, "inspector.canvas.fps.current"));
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.fps.current");
    assert_eq!(canvas(&harness).fps, time::FrameRate::FPS_23_976);
    assert_eq!(harness.state().session.history_len().0, history);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    // The background section exposes the whole solid palette and commits one step.
    let history = harness.state().session.history_len().0;
    let swatch = palette::solid(5).unwrap();
    click(&mut harness, "inspector.canvas_swatch.5");
    assert_eq!(canvas(&harness).background, swatch);
    assert_eq!(harness.state().session.history_len().0, history + 1);

    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-46-canvas.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&path).unwrap();

    // Reset to the retained original size is one step and keeps the preset row.
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.reset_original");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1080, 1920)
    );
    assert_eq!(canvas(&harness).preset, Some(AspectPreset::Original));
    assert_eq!(harness.state().session.history_len().0, history + 1);

    // Undo/redo restores the pre-reset canvas exactly.
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1082, 722)
    );
    harness.state_mut().session.redo().unwrap();
    harness.run_steps(3);
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1080, 1920)
    );

    // Settings round-trip through persistence with their custom memory.
    let encoded = project::encode(harness.state().session.project(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap().project;
    assert_eq!(&loaded, harness.state().session.project().as_ref());
    assert_eq!(
        loaded.data().canvas.last_custom_size,
        CanvasSize {
            width: 1082,
            height: 722
        }
    );
}

#[test]
fn timeline_cache_settings_share_two_stores_clear_refresh_and_preserve_projects() {
    use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
    use vibecut::app::Workspace;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    let budget = ByteBudget::new(64 << 20).unwrap();
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Waveform,
        source: [1; 32],
        provider: "test",
        model: "test",
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    for store in ["waveform", "thumbnails"] {
        let (mut cache, _) = ArtifactCache::open(&root.join(store), budget.clone(), 128).unwrap();
        cache.put(key, &[7; 28]).unwrap();
    }
    let source = dir.path().join("source-and-project-sentinel");
    std::fs::write(&source, b"personal source").unwrap();
    let mut workspace =
        Workspace::new(dir.path().join("recovery"), Preferences::default(), None).unwrap();
    let id = workspace.register_editor(Box::new(editor()));
    let mut second = editor();
    let mut data = second.session.project().as_ref().clone().into_data();
    data.id = ProjectId(2);
    second.session = vibecut::services::lifecycle::EditorSession::Memory(Box::new(
        Session::new(Project::new(data).unwrap(), 10).unwrap(),
    ));
    let second_id = workspace.register_editor(Box::new(second));
    let before = workspace.editor_mut(id).unwrap().session.project().clone();
    let history = workspace.editor_mut(id).unwrap().session.history_len();
    let position = workspace
        .editor_mut(id)
        .unwrap()
        .playback
        .position()
        .unwrap();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1727.0, 1029.0))
        .build_ui_state(
            move |ui, app: &mut Workspace| app.window_ui(id, ui),
            workspace,
        );
    harness.run_steps(3);
    for name in ["inspector.section.canvas", "inspector.section.background"] {
        harness
            .get(By::new().predicate(|node| node.author_id() == Some(name)))
            .click();
        harness.run_steps(2);
    }
    let wait = |h: &mut Harness<Workspace>| {
        h.run_steps(3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while h
            .query_all(
                By::new().predicate(|node| node.author_id() == Some("inspector.cache.cancel")),
            )
            .next()
            .is_some()
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
            h.run_steps(2);
        }
    };
    wait(&mut harness);
    assert!(
        harness
            .get(
                By::new().predicate(|n| n.author_id() == Some("inspector.cache.size")
                    && n.value().as_deref() == Some("200 B"))
            )
            .rect()
            .is_positive()
    );
    let capture = |h: &mut Harness<Workspace>, state: &str| {
        for (locale, language) in [
            (vibecut::i18n::Locale::En, "en"),
            (vibecut::i18n::Locale::Vi, "vi"),
        ] {
            h.state_mut().preferences.locale = locale;
            for scale in [1.0, 2.0] {
                h.input_mut()
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                h.ctx.set_zoom_factor(1.0);
                h.run_steps(3);
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "../../target/398-settings-{state}-{language}-{scale}x.png"
                ));
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let image = h.render().unwrap();
                assert_eq!(
                    image.dimensions(),
                    ((1727.0 * scale) as u32, (1029.0 * scale) as u32)
                );
                assert_eq!(h.ctx.content_rect().size(), egui::vec2(1727.0, 1029.0));
                image.save(path).unwrap();
            }
        }
        h.input_mut()
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(1.0);
        h.ctx.set_zoom_factor(1.0);
        h.state_mut().preferences.locale = vibecut::i18n::Locale::En;
        h.run_steps(3);
    };
    capture(&mut harness, "populated");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.cache.clear")))
        .click();
    harness.run_steps(2);
    wait(&mut harness);
    assert!(
        harness
            .get(
                By::new().predicate(|n| n.author_id() == Some("inspector.cache.size")
                    && n.value().as_deref() == Some("0 B"))
            )
            .rect()
            .is_positive()
    );
    capture(&mut harness, "cleared");
    let access = harness.state().media().unwrap();
    let reports = access.lock().unwrap().cache_maintenance().unwrap();
    assert!(reports.into_iter().all(|snapshot| {
        snapshot
            .run(engine::cache::CacheOperation::Inspect, || false)
            .disk_bytes
            == 0
    }));
    // The second editor uses the same process stores and observes the removal.
    harness.state_mut().editor_mut(second_id).unwrap().tick();
    assert!(Arc::ptr_eq(
        &before,
        harness
            .state_mut()
            .editor_mut(id)
            .unwrap()
            .session
            .project()
    ));
    assert_eq!(
        harness
            .state_mut()
            .editor_mut(id)
            .unwrap()
            .session
            .history_len(),
        history
    );
    assert_eq!(
        harness
            .state_mut()
            .editor_mut(id)
            .unwrap()
            .playback
            .position()
            .unwrap(),
        position
    );
    assert_eq!(std::fs::read(source).unwrap(), b"personal source");
    std::fs::remove_dir_all(root.join("thumbnails")).unwrap(); // external deletion, not maintenance
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.cache.refresh")))
        .click();
    harness.run_steps(2);
    wait(&mut harness);
    assert!(
        harness
            .get(
                By::new().predicate(|n| n.author_id() == Some("inspector.cache.error")
                    && n.value().as_deref()
                        == Some("Preview storage was removed or replaced; restart VibeCut."))
            )
            .rect()
            .is_positive()
    );
    capture(&mut harness, "root-removed");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.cache.retry")))
        .click();
    harness.run_steps(2);
    wait(&mut harness);
    assert!(!root.join("thumbnails").exists());
    assert_eq!(
        project::encode(
            harness
                .state_mut()
                .editor_mut(id)
                .unwrap()
                .session
                .project(),
            false
        )
        .unwrap(),
        project::encode(&before, false).unwrap()
    );
}
