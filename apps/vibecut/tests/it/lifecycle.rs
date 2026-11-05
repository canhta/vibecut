#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use std::time::{Duration, Instant};
use vibecut::{catalog::Preferences, egui, workspace::Workspace};
fn idle(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.busy() {
        app.tick();
        assert!(Instant::now() < deadline, "worker timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn app(root: &std::path::Path) -> Workspace {
    let mut app = Workspace::new(root.join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    app
}
#[test]
fn create_save_exit_reopen_view_state_and_schema_refusal_preserve_disk() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("project.vibecut");
    let mut app = app(root.path());
    app.create().unwrap();
    idle(&mut app);
    assert_eq!(
        app.editor
            .as_ref()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "New project"
    );
    app.editor
        .as_mut()
        .unwrap()
        .session
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":2,"scroll":55,"playhead":0}),
        )
        .unwrap();
    app.persist(Some(path.clone()), true, false, false).unwrap();
    assert!(app.busy());
    idle(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert!(app.editor.is_none());
    app.open(path.clone()).unwrap();
    idle(&mut app);
    assert_eq!(
        app.editor.as_ref().unwrap().session.metadata().view.zoom,
        2.0
    );
    app.persist(None, true, false, false).unwrap();
    idle(&mut app);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = serde_json::json!(u32::MAX);
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    app.open(path.clone()).unwrap();
    idle(&mut app);
    assert!(app.error().is_some());
    assert!(app.editor.is_none());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
#[test]
fn missing_path_creates_once_but_other_load_errors_show_retry_and_back() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.open(root.path().join("missing.vibecut")).unwrap();
    idle(&mut app);
    let id = app.editor.as_ref().unwrap().session.project().data().id;
    assert_eq!(
        app.editor
            .as_ref()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "Untitled Project"
    );
    for _ in 0..10 {
        app.tick();
    }
    assert_eq!(app.editor.as_ref().unwrap().session.project().data().id, id);
    app.persist(None, true, false, true).unwrap();
    idle(&mut app);
    let bad = root.path().join("bad.vibecut");
    std::fs::write(&bad, b"{").unwrap();
    app.open(bad).unwrap();
    idle(&mut app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), app);
    h.run_steps(3);
    assert!(h.query_by_label("Retry").is_some());
    h.get(By::new().predicate(|n| n.author_id() == Some("project.error.back")))
        .click();
    h.run_steps(3);
    assert!(h.state().error().is_none());
    assert!(h.query_by_label("New project").is_some());
}
#[test]
fn inline_rename_commit_cancel_and_window_close_guard_use_real_widgets() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.create().unwrap();
    idle(&mut app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), app);
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.rename")))
        .click();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.rename.input")))
        .type_text(" changed");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let renamed = h
        .state()
        .editor
        .as_ref()
        .unwrap()
        .session
        .project()
        .data()
        .name
        .as_str()
        .to_owned();
    assert_ne!(renamed, "New project");
    h.get(By::new().predicate(|n| n.author_id() == Some("project.rename")))
        .click();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.rename.input")))
        .type_text(" cancel");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(
        h.state()
            .editor
            .as_ref()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        renamed
    );
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(1);
    assert!(!h.state().ready_to_close());
    assert!(
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .unwrap()
            .commands
            .contains(&egui::ViewportCommand::CancelClose)
    );
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .clear();
    h.run_steps(3);
    let snapping = h.state().editor.as_ref().unwrap().preferences.snapping;
    h.key_press(egui::Key::N);
    h.run_steps(3);
    assert_eq!(
        h.state().editor.as_ref().unwrap().preferences.snapping,
        snapping
    );
    h.get(By::new().predicate(|n| n.author_id() == Some("project.close.cancel")))
        .click();
    h.run_steps(3);
    assert!(!h.state().ready_to_close());
    h.key_press(egui::Key::N);
    h.run_steps(3);
    assert_ne!(
        h.state().editor.as_ref().unwrap().preferences.snapping,
        snapping
    );
}
#[test]
fn recovery_prompt_recovers_copy_and_requires_save_as() {
    let root = tempfile::tempdir().unwrap();
    let mut first = app(root.path());
    first.create().unwrap();
    idle(&mut first);
    first.persist(None, true, false, true).unwrap();
    idle(&mut first);
    drop(first);
    let second = app(root.path());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), second);
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("recovery.recover")))
        .click();
    h.run_steps(3);
    idle(h.state_mut());
    h.run_steps(3);
    assert!(
        h.state_mut()
            .editor
            .as_mut()
            .unwrap()
            .session
            .autosave()
            .unwrap()
            .saved_path()
            .is_none()
    );
    h.state_mut()
        .persist(Some(root.path().join("copy.vibecut")), true, false, false)
        .unwrap();
    idle(h.state_mut());
    assert!(h.state().error().is_none());
    assert!(
        engine::recovery::discover(&root.path().join("recovery"))
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn create_failure_is_recoverable_and_busy_save_as_retains_the_editor() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let recovery = root.path().join("recovery");
    std::fs::remove_dir(&recovery).unwrap();
    std::fs::write(&recovery, b"not a directory").unwrap();
    app.create().unwrap();
    idle(&mut app);
    assert!(app.editor.is_none());
    assert!(app.error().is_some());
    std::fs::remove_file(recovery).unwrap();
    app.create().unwrap();
    idle(&mut app);
    assert!(app.error().is_none());
    let path = root.path().join("busy.vibecut");
    let held = project::WriterLease::acquire(&path).unwrap();
    let id = app.editor.as_ref().unwrap().session.project().data().id;
    app.persist(Some(path.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_some());
    assert_eq!(app.editor.as_ref().unwrap().session.project().data().id, id);
    assert!(!path.exists());
    drop(held);
    app.persist(Some(path.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert!(path.exists());
}

#[test]
fn loading_and_exiting_indicators_cover_real_background_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), app);
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.create")))
        .click();
    h.run_steps(1);
    h.run_steps(1);
    assert!(h.query_by_label("Loading project…").is_some());
    idle(h.state_mut());
    h.run_steps(3);
    h.state_mut()
        .persist(Some(root.path().join("saved.vibecut")), true, false, false)
        .unwrap();
    h.run_steps(1);
    assert!(h.query_by_label("Exiting project…").is_some());
    idle(h.state_mut());
    h.run_steps(3);
    assert!(h.state().editor.is_none());
}

#[test]
fn corrupt_recovery_candidate_is_skipped_with_a_visible_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let mut first = app(root.path());
    first.create().unwrap();
    idle(&mut first);
    first.persist(None, true, false, true).unwrap();
    idle(&mut first);
    drop(first);
    let journal = std::fs::read_dir(root.path().join("recovery"))
        .unwrap()
        .map(|e| e.unwrap().path().join("journal.json"))
        .find(|p| p.exists())
        .unwrap();
    std::fs::write(journal, b"{").unwrap();
    let second = app(root.path());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), second);
    h.run_steps(3);
    assert!(
        h.query_by_label("A damaged recovery copy was skipped.")
            .is_some()
    );
    assert!(h.query_by_label("Recover a copy").is_none());
    assert!(h.query_by_label("New project").is_some());
}

#[test]
fn journal_only_exit_refreshes_first_frame_thumbnail_without_saving_edits() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("project.vibecut");
    let mut app = app(root.path());
    app.create().unwrap();
    idle(&mut app);
    app.persist(Some(path.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_none());
    let before = std::fs::read(&path).unwrap();
    let session = &mut app.editor.as_mut().unwrap().session;
    let frame = session.project().data().canvas.fps.tick_of(1).unwrap();
    session.execute("timeline.insert", serde_json::json!({
        "placement":{"mode":"auto"},
        "item":{"animations":{},"source_asset":null,"source_range":{"start":0,"end":frame.0},
        "start":0,"content":{"kind":"overlay","params":{"color":[255,0,0,255]}}}
    })).unwrap();
    session
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":1,"scroll":0,"playhead":frame.0*10}),
        )
        .unwrap();
    app.persist(None, false, true, true).unwrap();
    idle(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert!(app.ready_to_close());
    assert!(app.editor.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(matches!(project::read_metadata(&path).thumbnail,
        project::Thumbnail::Ready { ref rgba, .. } if rgba[0]>245 && rgba[1]<10 && rgba[2]<10));
}
