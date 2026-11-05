#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use project::{Project, ProjectId};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use vibecut::{
    Editor, PREFERENCE_COMMAND,
    catalog::{DEFAULT_PANELS, PreferenceStore, Preferences},
    egui,
    playback::{Files, Playback},
};

fn editor(preferences: Preferences) -> Editor {
    let session = Session::new(Project::empty(ProjectId(1), "Test".into()).unwrap(), 10).unwrap();
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
    Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
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
    assert_eq!(PREFERENCE_COMMAND, "ui.prefs.set");
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
fn ui_prefs_set_dispatches_typed_preference_params() {
    let mut editor = editor(Preferences::default());
    let project = editor.session.project().clone();
    editor
        .dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "dark", "value": false}),
        )
        .unwrap();
    assert!(!editor.preferences.dark);
    assert_eq!(editor.session.project(), &project);
    editor
        .dispatch(
            "ui.prefs.set",
            serde_json::json!({"key": "panels", "value": [0.30, 0.20, 0.45]}),
        )
        .unwrap();
    assert_eq!(editor.preferences.panels, [0.30, 0.20, 0.45]);
    assert_eq!(editor.session.project(), &project);
    assert!(
        editor
            .dispatch(
                "ui.prefs.set",
                serde_json::json!({"key": "panels", "value": [0.9, 0.9, 0.9]}),
            )
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
fn splitter_drags_clamp_boundaries_and_restore_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut harness = build(editor(Preferences::default()));
    harness.run_steps(3);
    let full_width = harness.state().panels["media"].width()
        + harness.state().panels["preview"].width()
        + harness.state().panels["inspector"].width();
    drag(&mut harness, "layout.media", egui::vec2(10_000.0, 0.0));
    let wide = harness.state().preferences.panels;
    assert!((wide[0] - 0.40).abs() < f32::EPSILON);
    assert!(0.30 <= 1.0 - wide[0] - wide[1] + f32::EPSILON);
    click(&mut harness, "edit.undo");
    assert_eq!(harness.state().preferences.panels, DEFAULT_PANELS);
    click(&mut harness, "edit.redo");
    assert_eq!(harness.state().preferences.panels, wide);
    drag(&mut harness, "layout.timeline", egui::vec2(0.0, -10_000.0));
    assert!((harness.state().preferences.panels[2] - 0.70).abs() < f32::EPSILON);
    drag(&mut harness, "layout.timeline", egui::vec2(0.0, 10_000.0));
    assert!((harness.state().preferences.panels[2] - 0.15).abs() < f32::EPSILON);
    drag(&mut harness, "layout.media", egui::vec2(-10_000.0, 0.0));
    assert!((harness.state().preferences.panels[0] - 0.15).abs() < f32::EPSILON);

    harness.state_mut().preferences.panels = [0.30, 0.20, 0.45];
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load().unwrap();
    let mut restarted = build(editor(restored.clone()));
    restarted.run_steps(3);
    let media = restarted.state().panels["media"].width();
    assert!(
        (media - full_width * restored.panels[0]).abs() < 1.0,
        "restored left width {media} should match {}",
        full_width * restored.panels[0]
    );
    assert_eq!(restarted.state().preferences.panels, [0.30, 0.20, 0.45]);
}

#[test]
fn property_sections_collapse_expand_and_persist_per_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut harness = build(editor(Preferences::default()));
    harness.run_steps(3);
    click(&mut harness, "inspector.details");
    assert!(
        harness
            .state()
            .preferences
            .section_open("inspector.canvas", true)
    );
    click(&mut harness, "inspector.section.canvas");
    assert!(
        !harness
            .state()
            .preferences
            .section_open("inspector.canvas", true)
    );
    click(&mut harness, "inspector.section.canvas");
    assert!(
        harness
            .state()
            .preferences
            .section_open("inspector.canvas", false)
    );
    click(&mut harness, "inspector.section.canvas");
    click(&mut harness, "edit.undo");
    assert!(
        harness
            .state()
            .preferences
            .section_open("inspector.canvas", false)
    );
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load().unwrap();
    assert!(restored.section_open("inspector.canvas", false));
    assert!(!restored.section_open("inspector.metadata", false));
}

#[test]
fn hostile_window_sizes_keep_finite_nonnegative_panel_geometry() {
    for size in [
        egui::vec2(1.0, 1.0),
        egui::vec2(100.0, 50.0),
        egui::vec2(200.0, 100.0),
        egui::vec2(1280.0, 800.0),
    ] {
        let mut harness = Harness::builder().with_size(size).build_ui_state(
            |ui, editor: &mut Editor| editor.ui(ui),
            editor(Preferences::default()),
        );
        harness.run_steps(3);
        if size.y >= 100.0 {
            drag(&mut harness, "layout.media", egui::vec2(10_000.0, 0.0));
            drag(&mut harness, "layout.timeline", egui::vec2(0.0, -10_000.0));
        }
        for (id, rect) in &harness.state().panels {
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
        assert_eq!(store.load().unwrap(), editor.preferences);
    }
}

#[test]
fn splitter_commit_refuses_an_edit_that_would_exceed_the_store_budget() {
    let mut preferences = Preferences::default();
    for index in 0..128 {
        let mut next = preferences.clone();
        next.sections.insert(format!("section.{index:032}"), false);
        next.sections.insert("x".to_owned(), false);
        if serde_json::to_vec(&next).unwrap().len() > 4096 {
            break;
        }
        next.sections.remove("x");
        preferences = next;
    }
    let base = preferences.clone();
    for length in 1..=64 {
        preferences = base.clone();
        preferences.sections.insert("x".repeat(length), false);
        if serde_json::to_vec(&preferences).unwrap().len() == 4096 {
            break;
        }
    }
    assert_eq!(serde_json::to_vec(&preferences).unwrap().len(), 4096);
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
