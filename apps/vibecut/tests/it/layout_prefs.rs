#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use project::{AspectPreset, CanvasSize, Project, ProjectId};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::FrameRate;
use vibecut::{
    Editor, egui,
    preferences::{
        DEFAULT_PANELS, MEDIA_MIN_PX, PLAYER_MIN_PX, PreferenceStore, Preferences, TIMELINE_MAX,
        TIMELINE_MIN,
    },
    services::playback::{Files, Playback},
};

fn editor(preferences: Preferences) -> Editor {
    editor_with(
        Project::empty(ProjectId(1), "Test".into()).unwrap(),
        preferences,
    )
}

fn editor_with(project: Project, preferences: Preferences) -> Editor {
    let session = Session::new(project, 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, preferences)
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    if matches!(
        id,
        "appearance.theme" | "appearance.locale" | "layout.reset"
    ) {
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
            .click();
        harness.run_steps(2);
    }
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn widget(harness: &Harness<Editor>, id: &str) -> vibecut::WidgetState {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
}

fn drag(harness: &mut Harness<Editor>, id: &str, delta: egui::Vec2) {
    let start = widget(harness, id).rect.center();
    harness.hover_at(start);
    harness.run_steps(1);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(start + delta);
    harness.run_steps(2);
    harness.drop_at(start + delta);
    harness.run_steps(2);
}

#[test]
fn theme_toggle_uses_app_history_and_leaves_the_project_untouched() {
    let mut harness = build(editor(Preferences::default()));
    harness.run_steps(3);
    let project = harness.state().session.project().clone();
    assert!(harness.state().preferences.dark);
    click(&mut harness, "appearance.theme");
    assert!(!harness.state().preferences.dark);
    assert_eq!(harness.state().session.project(), &project);
    assert!(!harness.state().session.can_undo());
    click(&mut harness, "edit.undo");
    assert!(harness.state().preferences.dark);
    assert_eq!(harness.state().session.project(), &project);
    click(&mut harness, "edit.redo");
    assert!(!harness.state().preferences.dark);
    assert_eq!(harness.state().session.project(), &project);
}

#[test]
fn preference_edits_set_typed_values_and_leave_project_history_alone() {
    let mut editor = editor(Preferences::default());
    let project = editor.session.project().clone();
    editor
        .set_preference("dark", serde_json::json!(false))
        .unwrap();
    assert!(!editor.preferences.dark);
    assert_eq!(editor.session.project(), &project);
    editor
        .set_preference("panels", serde_json::json!([0.30, 0.20, 0.45]))
        .unwrap();
    assert_eq!(editor.preferences.panels, [0.30, 0.20, 0.45]);
    assert_eq!(editor.session.project(), &project);
    assert!(
        editor
            .set_preference("panels", serde_json::json!([0.9, 0.9, 0.9]))
            .is_err()
    );
    assert_eq!(editor.preferences.panels, [0.30, 0.20, 0.45]);
}

#[test]
fn layout_reset_is_undoable_and_never_enters_project_history() {
    let preferences = Preferences {
        panels: [0.30, 0.20, 0.55],
        ..Default::default()
    };
    let mut harness = build(editor(preferences));
    harness.run_steps(3);
    let project = harness.state().session.project().clone();
    click(&mut harness, "layout.reset");
    assert_eq!(harness.state().preferences.panels, DEFAULT_PANELS);
    assert_eq!(harness.state().session.project(), &project);
    assert!(!harness.state().session.can_undo());
    click(&mut harness, "edit.undo");
    assert_eq!(harness.state().preferences.panels, [0.30, 0.20, 0.55]);
    click(&mut harness, "edit.redo");
    assert_eq!(harness.state().preferences.panels, DEFAULT_PANELS);
    assert_eq!(harness.state().session.project(), &project);
}

#[test]
fn frame_keeps_chrome_compact_and_window_actions_in_the_menu() {
    for (width, height, scale, locale, dark) in [
        (1727.0, 1029.0, 1.0, vibecut::i18n::Locale::En, true),
        (1727.0, 1029.0, 2.0, vibecut::i18n::Locale::Vi, false),
        (1280.0, 800.0, 1.0, vibecut::i18n::Locale::Vi, true),
        (1280.0, 800.0, 2.0, vibecut::i18n::Locale::En, false),
    ] {
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(width, height))
            .with_pixels_per_point(scale)
            .build_ui_state(
                |ui, editor: &mut Editor| editor.ui(ui),
                editor(Preferences {
                    locale,
                    dark,
                    ..Default::default()
                }),
            );
        harness.run_steps(3);
        let header = harness.state().panels()["header"];
        let export = widget(&harness, "file.export").rect;
        assert_eq!(header.height(), 36.0);
        assert_eq!(export.size(), egui::vec2(67.0, 22.0));
        assert_eq!(export.top() - header.top(), 7.0);
        assert_eq!(header.right() - export.right(), 11.0);
        assert!(
            (widget(&harness, "project.rename").rect.center().x - header.center().x).abs() < 0.5
        );
        // The keyboard-shortcuts button is a header control beside Feedback, never in the menu.
        let shortcuts = widget(&harness, "shortcuts.open");
        assert!(shortcuts.enabled);
        assert!(shortcuts.rect.right() <= widget(&harness, "app.feedback").rect.left());
        assert!(shortcuts.rect.left() > widget(&harness, "project.rename").rect.right());
        let panels = &harness.state().panels();
        assert_eq!(panels["media"].left() - header.left(), 9.0);
        assert_eq!(panels["media"].top(), header.bottom());
        assert_eq!(panels["preview"].left() - panels["media"].right(), 7.0);
        assert_eq!(panels["inspector"].left() - panels["preview"].right(), 7.0);
        assert_eq!(panels["timeline"].top() - panels["media"].bottom(), 6.0);
        let initial = harness.state().session.project().clone();
        for id in [
            "window.home",
            "project.save",
            "project.save_as",
            "models.open",
            "layout.reset",
            "appearance.locale",
            "appearance.theme",
        ] {
            assert!(
                !harness
                    .state()
                    .inspect()
                    .unwrap()
                    .widgets
                    .iter()
                    .any(|widget| widget.id == id),
                "{id} must be behind the menu"
            );
        }
        click(&mut harness, "window.menu");
        for id in [
            "window.home",
            "project.save",
            "project.save_as",
            "models.open",
            "layout.reset",
            "appearance.locale",
            "appearance.theme",
        ] {
            assert!(widget(&harness, id).enabled, "{id} stays reachable");
        }
        assert_eq!(harness.state().session.project(), &initial);
    }
}

#[test]
fn splitter_drags_clamp_boundaries_and_restore_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut harness = build(editor(Preferences::default()));
    harness.run_steps(3);
    let full_width = harness.state().panels()["media"].width()
        + harness.state().panels()["preview"].width()
        + harness.state().panels()["inspector"].width();
    drag(&mut harness, "layout.media", egui::vec2(10_000.0, 0.0));
    let wide = harness.state().preferences.panels;
    // The media column takes what the player and properties leave at their minimums.
    let properties = harness.state().panels()["inspector"].width();
    assert!((wide[0] * full_width - (full_width - PLAYER_MIN_PX - properties)).abs() < 1.0);
    click(&mut harness, "edit.undo");
    assert_eq!(harness.state().preferences.panels, DEFAULT_PANELS);
    click(&mut harness, "edit.redo");
    assert_eq!(harness.state().preferences.panels, wide);
    drag(&mut harness, "layout.timeline", egui::vec2(0.0, -10_000.0));
    assert!((harness.state().preferences.panels[2] - TIMELINE_MAX).abs() < f32::EPSILON);
    drag(&mut harness, "layout.timeline", egui::vec2(0.0, 10_000.0));
    assert!((harness.state().preferences.panels[2] - TIMELINE_MIN).abs() < f32::EPSILON);
    drag(&mut harness, "layout.media", egui::vec2(-10_000.0, 0.0));
    assert!((harness.state().preferences.panels[0] * full_width - MEDIA_MIN_PX).abs() < 1.0);

    harness.state_mut().preferences.panels = [0.35, 0.30, 0.45];
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load_with_warning().0;
    let mut restarted = build(editor(restored.clone()));
    restarted.run_steps(3);
    let media = restarted.state().panels()["media"].width();
    assert!(
        (media - full_width * restored.panels[0]).abs() < 1.0,
        "restored left width {media} should match {}",
        full_width * restored.panels[0]
    );
    assert_eq!(restarted.state().preferences.panels, [0.35, 0.30, 0.45]);
}

#[test]
fn details_rows_and_info_section_collapse_and_persist() {
    // Aspect ratio, resolution and frame rate for each canvas state. An unsaved
    // project shows no Path row.
    let rows = [
        (
            Some(AspectPreset::Landscape),
            None,
            (1920, 1080),
            FrameRate::FPS_30,
            "16:9",
            "1,920 × 1,080",
            "30.00fps",
        ),
        (
            None,
            None,
            (1000, 1000),
            FrameRate::FPS_29_97,
            "Custom",
            "1,000 × 1,000",
            "29.97fps",
        ),
        (
            Some(AspectPreset::Original),
            Some((1080, 1920)),
            (1080, 1920),
            FrameRate::FPS_30,
            "Original",
            "Adapted",
            "30.00fps",
        ),
        (
            Some(AspectPreset::Landscape),
            Some((1920, 1080)),
            (1920, 1080),
            FrameRate::FPS_24,
            "Original",
            "1,920 × 1,080",
            "24.00fps",
        ),
    ];
    for (preset, original, (width, height), fps, aspect, resolution, rate) in rows {
        let mut data = Project::empty(ProjectId(1), "Test".into())
            .unwrap()
            .into_data();
        data.canvas.preset = preset;
        data.canvas.width = width;
        data.canvas.height = height;
        data.canvas.fps = fps;
        data.original_canvas_size = original.map(|(width, height)| CanvasSize { width, height });
        let mut harness = build(editor_with(
            Project::new(data).unwrap(),
            Preferences::default(),
        ));
        harness.run_steps(3);
        click(&mut harness, "inspector.details");
        assert!(harness.query_by_label("Path:").is_none());
        let top = |text: &str| harness.get_by_label(text).rect().top();
        // The top bar's title button also carries the project name; the Details row is a label.
        let name = harness
            .get(By::new().label("Test").role(egui::accesskit::Role::Label))
            .rect()
            .top();
        let (imported_row, aspect_row, resolution_row, rate_row) = (
            top("Imported media:"),
            top(aspect),
            top(resolution),
            top(rate),
        );
        assert!(
            name < imported_row
                && imported_row < aspect_row
                && aspect_row < resolution_row
                && resolution_row < rate_row
        );
        harness.get_by_label("Name:");
        harness.get_by_label("Aspect ratio:");
        harness.get_by_label("Resolution:");
        harness.get_by_label("Frame rate:");
        harness.get_by_label("Modify");
        // No user media yet: the value is the empty dash, not the stay-in-place text.
        assert!(
            harness
                .query_by_label("Stay in original location")
                .is_none()
        );
    }

    // Imported media reads `Stay in original location` for user assets, which are referenced
    // by path and never copied; ephemeral assets are internal and do not count.
    for (ephemeral, stays) in [(false, true), (true, false)] {
        let mut data = Project::empty(ProjectId(1), "Test".into())
            .unwrap()
            .into_data();
        data.assets.push(
            SourceAsset::try_from(AssetData {
                ephemeral,
                id: AssetId(1),
                path: std::env::temp_dir().join("vibecut-details-media.mp4"),
                content_hash: ContentHash::from_bytes(&[1; 8]),
                probe: SourceProbe {
                    format: "mov,mp4".into(),
                    duration: None,
                    streams: Vec::new(),
                },
            })
            .unwrap(),
        );
        let mut harness = build(editor_with(
            Project::new(data).unwrap(),
            Preferences::default(),
        ));
        harness.run_steps(3);
        click(&mut harness, "inspector.details");
        harness.get_by_label("Imported media:");
        assert_eq!(
            harness
                .query_by_label("Stay in original location")
                .is_some(),
            stays,
            "stay-in-place text for ephemeral={ephemeral}"
        );
    }

    // A saved project shows its path row; the unsaved one above does not.
    let root = tempfile::tempdir().unwrap();
    let project = Project::empty(ProjectId(1), "Test".into()).unwrap();
    let mut saving = engine::recovery::Autosave::new(
        &root.path().join("recovery"),
        Session::new(project.clone(), 10).unwrap(),
        ByteBudget::new(64 << 20).unwrap(),
        engine::footage::uncorrected(),
        Arc::new(engine::jobs::SystemClock::default()),
        Default::default(),
    )
    .unwrap();
    let saved_path = root.path().join("title.vibecut");
    saving.save_to(saved_path.clone()).unwrap();
    // Save As publishes on a worker; the path is the saved one once tick acknowledges it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while saving.saved_path().is_none() {
        assert!(std::time::Instant::now() < deadline, "save never completed");
        std::thread::sleep(std::time::Duration::from_millis(5));
        saving.tick().unwrap();
    }
    let mut saved = editor_with(project, Preferences::default());
    saved.session = vibecut::services::lifecycle::EditorSession::Persistent(Box::new(saving));
    let mut harness = build(saved);
    harness.run_steps(3);
    click(&mut harness, "inspector.details");
    harness.get_by_label("Path:");
    harness.get_by_label(&saved_path.display().to_string());

    // The light and dark screenshots of the Details view; dark also goes to the ticket name.
    for (dark, name) in [(true, "dark"), (false, "light")] {
        let mut harness = build(editor(Preferences {
            dark,
            ..Default::default()
        }));
        harness.run_steps(3);
        click(&mut harness, "inspector.details");
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        std::fs::create_dir_all(&target).unwrap();
        let image = harness.render().unwrap();
        image
            .save(target.join(format!("p1-320-details-{name}.png")))
            .unwrap();
        if dark {
            image.save(target.join("p1-320-details.png")).unwrap();
        }
    }

    // Modify opens the Settings tab and focuses the canvas width, expanding a collapsed
    // canvas section first.
    for canvas_open in [true, false] {
        let mut preferences = Preferences::default();
        if !canvas_open {
            preferences
                .sections
                .insert("inspector.canvas".into(), false);
        }
        let mut harness = build(editor(preferences));
        harness.run_steps(3);
        click(&mut harness, "inspector.details");
        click(&mut harness, "inspector.details.modify");
        assert!(
            harness
                .state()
                .preferences
                .section_open("inspector.canvas", true),
            "canvas section open after Modify (was open: {canvas_open})"
        );
        assert!(has_widget(&harness, "inspector.canvas.fps"));
        assert!(
            harness
                .get(By::new().predicate(|node| node.author_id() == Some("inspector.canvas.width")))
                .is_focused(),
            "canvas width focused (section was open: {canvas_open})"
        );
    }

    // Info section: collapsed by default, toggles and persists per key.
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut harness = build(editor(Preferences::default()));
    harness.run_steps(3);
    click(&mut harness, "inspector.details");
    assert!(
        !harness
            .state()
            .preferences
            .section_open("inspector.metadata", false)
    );
    click(&mut harness, "inspector.section.metadata");
    assert!(
        harness
            .state()
            .preferences
            .section_open("inspector.metadata", false)
    );
    click(&mut harness, "inspector.section.metadata");
    assert!(
        !harness
            .state()
            .preferences
            .section_open("inspector.metadata", false)
    );
    click(&mut harness, "inspector.section.metadata");
    assert!(
        harness
            .state()
            .preferences
            .section_open("inspector.metadata", false)
    );
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load_with_warning().0;
    assert!(restored.section_open("inspector.metadata", false));
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

#[test]
fn hostile_window_sizes_keep_finite_nonnegative_panel_geometry() {
    for size in [
        egui::vec2(1.0, 1.0),
        egui::vec2(100.0, 50.0),
        egui::vec2(200.0, 100.0),
        egui::vec2(1280.0, 800.0),
    ] {
        let mut harness = crate::gpu::builder().with_size(size).build_ui_state(
            |ui, editor: &mut Editor| editor.ui(ui),
            editor(Preferences::default()),
        );
        harness.run_steps(3);
        if size.y >= 100.0 {
            drag(&mut harness, "layout.media", egui::vec2(10_000.0, 0.0));
            drag(&mut harness, "layout.timeline", egui::vec2(0.0, -10_000.0));
        }
        for (id, rect) in harness.state().panels() {
            assert!(
                rect.is_finite() && rect.width() >= 0.0 && rect.height() >= 0.0,
                "{size:?} {id}: {rect:?}"
            );
        }
        assert!(harness.state().preferences.validate().is_ok());
    }
}

#[test]
fn invalid_section_edits_are_atomic_and_do_not_enter_history() {
    let mut editor = editor(Preferences::default());
    let before = editor.preferences.clone();
    assert!(editor.set_section_open("", false).is_err());
    assert_eq!(editor.preferences, before);
    editor.activate("appearance.theme").unwrap();
    assert!(editor.set_section_open(&"x".repeat(65), false).is_err());
    editor.activate("edit.undo").unwrap();
    assert_eq!(editor.preferences, before);
    for index in 0..128 {
        let key = format!("{index:064}");
        if editor.set_section_open(&key, false).is_err() {
            break;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(dir.path().join("prefs.json"));
        store.save(&editor.preferences).unwrap();
        assert_eq!(store.load_with_warning().0, editor.preferences);
    }
}

#[test]
fn splitter_commit_refuses_an_edit_that_would_exceed_the_store_budget() {
    let mut preferences = Preferences {
        // This regression needs a drag that grows the serialized ratio.
        panels: [0.25, 0.25, 0.50],
        ..Default::default()
    };
    let ceiling = vibecut::preferences::MAX_PREFERENCES_BYTES as usize;
    for choice in &mut preferences.ai_models.choices {
        choice.recent = (0..4)
            .map(|index| format!("{index}{}", "x".repeat(ai::defaults::MAX_MODEL_BYTES - 1)))
            .collect();
        choice.model = choice.recent[0].clone();
    }
    preferences.resolve_models();
    for index in 0..64 {
        preferences.windows.insert(
            format!("{index:064}"),
            vibecut::preferences::WindowGeometry {
                size: [12345.678, 12345.678],
                position: Some([-12345.678, -23_456.79]),
                maximized: false,
            },
        );
    }
    for index in 0..128 {
        let mut next = preferences.clone();
        next.sections.insert(format!("section.{index:032}"), false);
        next.sections.insert("x".to_owned(), false);
        if serde_json::to_vec(&next).unwrap().len() > ceiling {
            break;
        }
        next.sections.remove("x");
        preferences = next;
    }
    let base = preferences.clone();
    for length in 1..=64 {
        preferences = base.clone();
        preferences.sections.insert("x".repeat(length), false);
        if serde_json::to_vec(&preferences).unwrap().len() == ceiling {
            break;
        }
    }
    assert_eq!(serde_json::to_vec(&preferences).unwrap().len(), ceiling);
    preferences.validate().unwrap();
    let mut harness = build(editor(preferences.clone()));
    harness.run_steps(3);
    drag(&mut harness, "layout.media", egui::vec2(137.0, 0.0));
    assert_eq!(harness.state().preferences, preferences);
}

/// Offscreen screenshot evidence for the ticket; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-05-*.png"]
fn layout_preferences_render_light_and_dark_screenshots() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .unwrap_or_else(|| root.join("target"));
    for (dark, name) in [(true, "dark"), (false, "light")] {
        let mut harness = build(editor(Preferences {
            dark,
            panels: [0.30, 0.20, 0.55],
            ..Default::default()
        }));
        harness.run_steps(3);
        click(&mut harness, "inspector.details");
        let image = harness.render().expect("offscreen render");
        std::fs::create_dir_all(&target).unwrap();
        image
            .save(target.join(format!("p1-05-{name}.png")))
            .unwrap();
        assert!(image.width() > 0 && image.height() > 0);
    }
}

#[test]
fn window_geometry_is_independent_bounded_and_persisted_outside_project_history() {
    use vibecut::preferences::WindowGeometry;
    let root = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(root.path().join("preferences.json"));
    let mut preferences = Preferences::default();
    let first = WindowGeometry {
        size: [1440.0, 900.0],
        position: Some([20.0, 30.0]),
        maximized: false,
    };
    let second = WindowGeometry {
        size: [1728.0, 1030.0],
        position: Some([-400.0, 10.0]),
        maximized: true,
    };
    preferences.windows.insert("editor.1".into(), first.clone());
    preferences
        .windows
        .insert("editor.2".into(), second.clone());
    store.save(&preferences).unwrap();
    let loaded = store.load_with_warning().0;
    assert_eq!(loaded.windows.get("editor.1"), Some(&first));
    assert_eq!(loaded.windows.get("editor.2"), Some(&second));
    for bad in [f32::NAN, f32::INFINITY, 0.0, 16385.0] {
        preferences.windows.get_mut("editor.1").unwrap().size[0] = bad;
        assert!(preferences.validate().is_err());
        assert!(store.save(&preferences).is_err());
        assert_eq!(store.load_with_warning().0, loaded);
    }
}
