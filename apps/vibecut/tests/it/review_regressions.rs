#![allow(clippy::unwrap_used, clippy::panic)]
use std::time::{Duration, Instant};
use vibecut::{catalog::Preferences, workspace::Workspace};
fn idle(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.busy() {
        app.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn prepublication_save_as_refusal_must_allow_a_different_destination() {
    let root = tempfile::tempdir().unwrap();
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    app.create().unwrap();
    idle(&mut app);
    let invalid = root.path().join("reserved.meta.json");
    app.persist(Some(invalid.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_some());
    assert!(!invalid.exists());
    let valid = root.path().join("valid.vibecut");
    app.persist(Some(valid.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(
        app.error().is_none(),
        "valid Save As stays poisoned: {:?}",
        app.error()
    );
    assert!(valid.exists());
}
#[test]
fn only_one_recovery_prompt_should_own_the_editor_at_a_time() {
    use egui_kittest::{
        Harness,
        kittest::{By, Queryable},
    };
    let root = tempfile::tempdir().unwrap();
    for _ in 0..2 {
        let mut app =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        idle(&mut app);
        // First instance has no candidate; second has one, but create is the public lifecycle entry.
        app.create().unwrap();
        idle(&mut app);
        app.persist(None, true, false, true).unwrap();
        idle(&mut app);
    }
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    let mut h = Harness::builder()
        .with_size(vibecut::egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), app);
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("recovery.recover")))
        .click();
    h.run_steps(3);
    idle(h.state_mut());
    h.run_steps(3);
    assert!(h.state().editor.is_some());
    assert!(
        h.query_by_label("Recover a copy").is_none(),
        "another startup candidate still blocks the recovered editor"
    );
}

#[test]
fn two_recovered_projects_can_be_saved_and_left_in_sequence_without_changing_originals() {
    use egui_kittest::{
        Harness,
        kittest::{By, Queryable},
    };
    let root = tempfile::tempdir().unwrap();
    let mut originals = Vec::new();
    for id in 1..=2 {
        let path = root.path().join(format!("original-{id}.vibecut"));
        project::WriterLease::acquire(&path)
            .unwrap()
            .save(
                &project::Project::empty(project::ProjectId(id), format!("original {id}").into())
                    .unwrap(),
            )
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut app =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        idle(&mut app);
        app.open(path.clone()).unwrap();
        idle(&mut app);
        app.editor
            .as_mut()
            .unwrap()
            .session
            .execute(
                "project.rename",
                serde_json::json!({"name":format!("recovered {id}")}),
            )
            .unwrap();
        app.persist(None, true, false, true).unwrap();
        idle(&mut app);
        originals.push((path, bytes));
    }
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    let mut h = Harness::builder()
        .with_size(vibecut::egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, a: &mut Workspace| a.ui(ui), app);
    h.run_steps(3);
    let mut recovered = std::collections::BTreeSet::new();
    for copy in 1..=2 {
        h.get(By::new().predicate(|n| n.author_id() == Some("recovery.recover")))
            .click();
        h.run_steps(3);
        idle(h.state_mut());
        h.run_steps(3);
        assert!(h.query_by_label("Recover a copy").is_none());
        let editor = h.state().editor.as_ref().unwrap();
        let id = editor.session.project().data().id;
        recovered.insert(id);
        let name = editor.session.project().data().name.clone();
        let destination = root.path().join(format!("copy-{copy}.vibecut"));
        h.state_mut()
            .persist(Some(destination.clone()), true, false, false)
            .unwrap();
        idle(h.state_mut());
        h.run_steps(3);
        assert!(h.state().error().is_none());
        assert!(h.state().editor.is_none());
        let saved = project::load(&destination).unwrap().project;
        assert_eq!(saved.data().id, id);
        assert_eq!(saved.data().name, name);
        for (original, bytes) in &originals {
            assert_eq!(&std::fs::read(original).unwrap(), bytes);
        }
    }
    assert_eq!(recovered.len(), 2);
    assert!(
        engine::recovery::discover(&root.path().join("recovery"))
            .unwrap()
            .candidates
            .is_empty()
    );
}
