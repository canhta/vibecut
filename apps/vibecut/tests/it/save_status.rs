//! The header save text follows the save job to its end on every path that writes the project.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::Harness;
use engine::{ByteBudget, Command, Session};
use project::{Project, ProjectId, TrackKind};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use vibecut::{
    Editor, egui,
    preferences::Preferences,
    services::lifecycle::EditorSession,
    services::playback::{Files, Playback},
};

const SAVING: &str = "Saving project…";

/// A persistent editor whose project is saved as `<root>/title.vibecut`.
fn saved_editor(root: &Path) -> Editor {
    let project = Project::empty(ProjectId(1), "Test".into()).unwrap();
    let mut saving = engine::recovery::Autosave::new(
        &root.join("recovery"),
        Session::new(project.clone(), 10).unwrap(),
        ByteBudget::new(64 << 20).unwrap(),
        engine::footage::uncorrected(),
        Arc::new(engine::jobs::SystemClock::default()),
        Default::default(),
    )
    .unwrap();
    saving.save_to(root.join("title.vibecut")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while saving.saved_path().is_none() {
        assert!(Instant::now() < deadline, "save never completed");
        std::thread::sleep(Duration::from_millis(5));
        saving.tick().unwrap();
    }
    let session = Session::new(project, 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.session = EditorSession::Persistent(Box::new(saving));
    editor
}

fn harness(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn edit(harness: &mut Harness<Editor>) {
    harness
        .state_mut()
        .session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "".into(),
            kind: TrackKind::Video,
        })
        .unwrap();
    harness.run_steps(2);
}

/// Paint frames until the header no longer says saving, within a bound.
fn settle(harness: &mut Harness<Editor>) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.run_steps(1);
        let text = harness.state().save_status_text();
        if text != SAVING {
            return text;
        }
        assert!(Instant::now() < deadline, "header still says {text}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn autosave<'a>(harness: &'a mut Harness<'_, Editor>) -> &'a mut engine::recovery::Autosave {
    harness.state_mut().session.autosave().unwrap()
}

#[test]
fn a_clean_saved_project_never_shows_saving() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = harness(saved_editor(root.path()));
    // The journal of a clean project is cleared, which leaves its revision at zero.
    let text = settle(&mut harness);
    assert_eq!(text, "Auto saved");
    harness.run_steps(30);
    assert_eq!(harness.state().save_status_text(), "Auto saved");
}

#[test]
fn autosave_shows_saving_then_the_journal_clock() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = harness(saved_editor(root.path()));
    settle(&mut harness);
    edit(&mut harness);
    assert_eq!(harness.state().save_status_text(), SAVING);
    let text = settle(&mut harness);
    assert!(text.starts_with("Auto saved: "), "{text}");
    harness.run_steps(30);
    assert_eq!(harness.state().save_status_text(), text);
}

#[test]
fn a_manual_save_ends_in_the_saved_state() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = harness(saved_editor(root.path()));
    settle(&mut harness);
    edit(&mut harness);
    autosave(&mut harness)
        .apply(Command::SaveProject {})
        .unwrap();
    assert_eq!(harness.state().save_status_text(), SAVING);
    let text = settle(&mut harness);
    assert!(text.starts_with("Auto saved"), "{text}");
    harness.run_steps(30);
    assert!(harness.state().save_status_text().starts_with("Auto saved"));
    assert!(!autosave(&mut harness).is_dirty());
}

#[test]
fn the_close_flush_ends_in_the_saved_state() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = harness(saved_editor(root.path()));
    settle(&mut harness);
    edit(&mut harness);
    // Close and project switches wait on the flush barrier before leaving.
    autosave(&mut harness).flush().unwrap();
    harness.run_steps(2);
    let text = settle(&mut harness);
    assert!(text.starts_with("Auto saved"), "{text}");
    harness.run_steps(30);
    assert!(harness.state().save_status_text().starts_with("Auto saved"));
}
