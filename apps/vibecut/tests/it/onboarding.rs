#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use project::{Project, ProjectId};
use std::sync::Arc;
use vibecut::{
    Editor, egui,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

fn build(preferences: Preferences) -> Harness<'static, Editor> {
    let session = Session::new(Project::empty(ProjectId(1), "Empty".into()).unwrap(), 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let editor = Editor::new(session, playback, preferences);
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    harness
}

fn has(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .query(By::new().predicate(|node| node.author_id() == Some(id)))
        .is_some()
}

/// Whether some painted text shape reads `expected` (the drop zone paints its words).
fn paints(harness: &Harness<Editor>, expected: &str) -> bool {
    let mut pending: Vec<_> = harness.output().shapes.iter().map(|s| &s.shape).collect();
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Vec(children) => pending.extend(children),
            egui::Shape::Text(text) if text.galley.text() == expected => return true,
            _ => {}
        }
    }
    false
}

#[test]
fn onboarding_reset_is_undoable_and_never_enters_project_history() {
    let mut harness = build(Preferences {
        welcome_seen: true,
        ..Preferences::default()
    });
    assert!(harness.state().preferences.welcome_seen);
    let project = harness.state().session.project().clone();
    harness.state_mut().activate("ui.onboarding.reset").unwrap();
    harness.run_steps(2);
    assert!(!harness.state().preferences.welcome_seen);
    assert_eq!(harness.state().session.project(), &project);
    assert!(!harness.state().session.can_undo());
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(harness.state().preferences.welcome_seen);
}

#[test]
fn empty_media_invites_a_drop_and_files_held_over_it_get_their_own_state() {
    let mut harness = build(Preferences::default());
    assert!(has(&harness, "file.import"));
    assert!(paints(&harness, "Import"));
    assert!(!paints(&harness, "Drop to import"));

    harness
        .input_mut()
        .hovered_files
        .push(egui::HoveredFile::default());
    harness.run_steps(1);
    assert!(paints(&harness, "Drop to import"), "the zone is the target");
    assert!(
        !paints(&harness, "Drop media here to import"),
        "the window-wide overlay stands down while the zone shows"
    );
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-08-media-drop.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&path).unwrap();

    harness.run_steps(2);
    assert!(paints(&harness, "Import"), "the state ends with the drag");
}
