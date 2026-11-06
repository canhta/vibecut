#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::kittest::{By, Queryable};
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
        app.focused_editor()
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "New project"
    );
    app.focused_editor_mut()
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
    assert!(app.focused_editor().is_none());
    app.open(path.clone()).unwrap();
    idle(&mut app);
    assert_eq!(
        app.focused_editor().unwrap().session.metadata().view.zoom,
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
    assert!(app.focused_editor().is_none());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
#[test]
fn missing_path_creates_once_but_other_load_errors_show_retry_and_back() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.open(root.path().join("missing.vibecut")).unwrap();
    idle(&mut app);
    let id = app.focused_editor().unwrap().session.project().data().id;
    assert_eq!(
        app.focused_editor()
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
    assert_eq!(
        app.focused_editor().unwrap().session.project().data().id,
        id
    );
    app.persist(None, true, false, true).unwrap();
    idle(&mut app);
    let bad = root.path().join("bad.vibecut");
    std::fs::write(&bad, b"{").unwrap();
    app.open(bad).unwrap();
    idle(&mut app);
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, a: &mut Workspace| {
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
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, a: &mut Workspace| {
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
    h.get(By::new().predicate(|n| n.author_id() == Some("project.rename")))
        .click();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("project.rename.input")))
        .type_text(" changed");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let renamed = h
        .state()
        .focused_editor()
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
            .focused_editor()
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
    let snapping = h.state().focused_editor().unwrap().preferences.snapping;
    h.key_press(egui::Key::N);
    h.run_steps(3);
    assert_eq!(
        h.state().focused_editor().unwrap().preferences.snapping,
        snapping
    );
    h.get(By::new().predicate(|n| n.author_id() == Some("project.close.cancel")))
        .click();
    h.run_steps(3);
    assert!(!h.state().ready_to_close());
    h.key_press(egui::Key::N);
    h.run_steps(3);
    assert_ne!(
        h.state().focused_editor().unwrap().preferences.snapping,
        snapping
    );
    let id = h.state().focused_project().unwrap();
    h.state_mut()
        .persist(
            Some(root.path().join("during-save.vibecut")),
            false,
            false,
            false,
        )
        .unwrap();
    h.state_mut().windows.get_mut(&id).unwrap().request_close();
    idle(h.state_mut());
    h.run_steps(3);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("project.close.cancel")))
            .count()
            == 1,
        "a close request during save must survive the save acknowledgement"
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
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, a: &mut Workspace| {
                a.tick();
                if let Some(id) = a.focused_project() {
                    a.window_ui(id, ui);
                } else {
                    a.ui(ui);
                }
            },
            second,
        );
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("recovery.recover")))
        .click();
    h.run_steps(3);
    idle(h.state_mut());
    h.run_steps(3);
    assert!(
        h.state_mut()
            .focused_editor_mut()
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
    assert!(app.focused_editor().is_none());
    assert!(app.error().is_some());
    std::fs::remove_file(recovery).unwrap();
    app.create().unwrap();
    idle(&mut app);
    assert!(app.error().is_none());
    let path = root.path().join("busy.vibecut");
    let held = project::WriterLease::acquire(&path).unwrap();
    let id = app.focused_editor().unwrap().session.project().data().id;
    app.persist(Some(path.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_some());
    assert_eq!(
        app.focused_editor().unwrap().session.project().data().id,
        id
    );
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
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, a: &mut Workspace| {
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
    assert!(h.state().focused_editor().is_none());
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
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, a: &mut Workspace| {
                a.tick();
                if let Some(id) = a.focused_project() {
                    a.window_ui(id, ui);
                } else {
                    a.ui(ui);
                }
            },
            second,
        );
    h.run_steps(3);
    assert!(h.query_by_label("A damaged backup was skipped.").is_some());
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
    let session = &mut app.focused_editor_mut().unwrap().session;
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
    assert!(!app.ready_to_close(), "Home remains open");
    assert!(app.focused_editor().is_none());
    app.request_close();
    assert!(app.ready_to_close());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(matches!(project::read_metadata(&path).thumbnail,
        project::Thumbnail::Ready { ref rgba, .. } if rgba[0]>245 && rgba[1]<10 && rgba[2]<10));
}

#[test]
fn final_close_is_not_cancelled_when_the_root_viewport_echoes_it_back() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.request_close();
    assert!(app.ready_to_close());
    let ctx = egui::Context::default();
    // A `ViewportCommand::Close` reaches the root viewport as `close_requested`
    // on the next pass; cancelling it would leave a hidden process running.
    let mut input = egui::RawInput::default();
    input.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            events: vec![egui::ViewportEvent::Close],
            ..Default::default()
        },
    );
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    output.textures_delta.clear();
    let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands.contains(&egui::ViewportCommand::Close));
    assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
}

#[test]
fn project_windows_keep_independent_edits_undo_and_playback_and_reopen_focuses_one_writer() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let mut ids = Vec::new();
    let mut paths = Vec::new();
    for number in 1..=2 {
        app.create().unwrap();
        idle(&mut app);
        let path = root.path().join(format!("project-{number}.vibecut"));
        app.persist(Some(path.clone()), false, false, false)
            .unwrap();
        idle(&mut app);
        assert!(app.error().is_none(), "{:?}", app.error());
        ids.push(app.focused_project().unwrap());
        paths.push(path);
    }
    assert!(app.home_open());
    assert_eq!(app.windows.len(), 2);
    let mut first = ids[0];
    let second = ids[1];
    for (id, name) in [(first, "first edit"), (second, "second edit")] {
        app.editor_mut(id)
            .unwrap()
            .session
            .execute("project.rename", serde_json::json!({"name":name}))
            .unwrap();
    }
    app.editor_mut(first)
        .unwrap()
        .activate("edit.undo")
        .unwrap();
    assert_eq!(
        app.editor_mut(first)
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "New project"
    );
    assert_eq!(
        app.editor_mut(second)
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "second edit"
    );
    app.editor_mut(first)
        .unwrap()
        .activate("edit.redo")
        .unwrap();
    assert_eq!(
        app.editor_mut(first)
            .unwrap()
            .session
            .project()
            .data()
            .name
            .as_str(),
        "first edit"
    );
    let first_port = app.editor_mut(first).unwrap().playback.audio_port();
    let second_port = app.editor_mut(second).unwrap().playback.audio_port();
    assert!(!std::sync::Arc::ptr_eq(&first_port, &second_port));
    app.tick();
    assert_eq!(app.windows.get(&first).unwrap().title(), "first edit");
    assert_eq!(app.windows.get(&second).unwrap().title(), "second edit");
    let revision = app.editor_mut(first).unwrap().session.revision();
    // Canonical path aliases select the original editor without loading a second writer.
    app.open(root.path().join(".").join(paths[0].file_name().unwrap()))
        .unwrap();
    assert_eq!(app.focused_project(), Some(first));
    assert_eq!(app.windows.len(), 2);
    assert_eq!(app.editor_mut(first).unwrap().session.revision(), revision);
    assert!(matches!(
        project::WriterLease::acquire(&paths[0]),
        Err(project::Error::Busy)
    ));
    let original_id = first;
    let original_bytes = std::fs::read(&paths[0]).unwrap();
    let original_viewport = app.viewport(first).unwrap();
    let copy = root.path().join("copy.vibecut");
    let lanes = std::collections::BTreeSet::from([7, 9]);
    let mut lane_map = app
        .editor_mut(first)
        .unwrap()
        .preferences
        .timeline_lanes
        .clone();
    lane_map.insert(original_id, lanes.clone());
    app.editor_mut(first)
        .unwrap()
        .set_preference("timeline_lanes", serde_json::to_value(&lane_map).unwrap())
        .unwrap();
    app.preferences.timeline_lanes = lane_map;
    // A refused Save As changes nothing: the issuing project keeps its lanes
    // and no entry appears for a destination that was never adopted.
    let refused = root.path().join("refused.vibecut");
    let held = project::WriterLease::acquire(&refused).unwrap();
    app.focus(first).unwrap();
    app.persist(Some(refused.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_some(), "a busy destination must be refused");
    assert_eq!(
        app.preferences.timeline_lanes.get(&original_id),
        Some(&lanes),
        "a failed Save As leaves the issuing project's lanes unchanged"
    );
    assert_eq!(
        app.preferences.timeline_lanes.len(),
        1,
        "a failed Save As mints no lane entry"
    );
    drop(held);
    app.focus(first).unwrap();
    app.persist(Some(copy.clone()), false, false, false)
        .unwrap();
    // A window UI pass may finish its save before the registry's next pump.
    // Reopening must reconcile that identity before selecting an existing window.
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.busy() {
        app.focused_window_mut().unwrap().tick();
        assert!(Instant::now() < deadline, "worker timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
    app.open(copy.clone()).unwrap();
    app.tick();
    assert!(app.error().is_none(), "{:?}", app.error());
    first = app.focused_project().unwrap();
    assert_ne!(first, original_id);
    assert_eq!(
        app.preferences.timeline_lanes.get(&first),
        Some(&lanes),
        "Save As carries the issuing project's open lanes to the copy"
    );
    assert_eq!(
        app.preferences.timeline_lanes.get(&original_id),
        Some(&lanes),
        "the original keeps its own open lanes after Save As"
    );
    assert_eq!(app.viewport(first), Some(original_viewport));
    assert_eq!(app.windows.len(), 2);
    assert!(!app.windows.contains_key(&original_id));
    assert!(std::sync::Arc::ptr_eq(
        &first_port,
        &app.editor_mut(first).unwrap().playback.audio_port()
    ));
    assert_eq!(std::fs::read(&paths[0]).unwrap(), original_bytes);
    assert_eq!(project::load(&copy).unwrap().project.data().id, first);
    app.editor_mut(first)
        .unwrap()
        .activate("edit.undo")
        .unwrap();
    assert_eq!(
        app.editor_mut(first).unwrap().session.project().data().id,
        first
    );
    app.editor_mut(first)
        .unwrap()
        .activate("edit.redo")
        .unwrap();
    app.open(copy.clone()).unwrap();
    assert_eq!(app.focused_project(), Some(first));
    app.open(paths[0].clone()).unwrap();
    idle(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert_eq!(app.focused_project(), Some(original_id));
    assert_eq!(app.windows.len(), 3);
    assert_ne!(app.viewport(original_id), app.viewport(first));
    app.windows
        .get_mut(&original_id)
        .unwrap()
        .persist(None, true, true, true)
        .unwrap();
    idle(&mut app);
    assert_eq!(app.windows.len(), 2);
    paths[0] = copy;
    // Explicit window harness targets preserve the same automation ids in either window.
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            move |ui, a: &mut Workspace| {
                a.tick();
                let id = a.focused_project().unwrap();
                a.window_ui(id, ui);
            },
            app,
        );
    for id in [first, second] {
        harness.state_mut().focus(id).unwrap();
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label(if id == first {
                    "first edit"
                } else {
                    "second edit"
                })
                .is_some()
        );
        assert!(
            harness
                .query_all(By::new().predicate(|n| n.author_id() == Some("project.rename")))
                .count()
                == 1
        );
        assert!(
            harness
                .query_all(By::new().predicate(|n| n.author_id() == Some("project.back")))
                .count()
                == 0
        );
    }
    harness.state_mut().request_close();
    assert!(!harness.state().home_open());
    assert!(!harness.state().ready_to_close());
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("window.menu")))
        .click();
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("window.home")))
        .click();
    harness.run_steps(3);
    assert!(harness.state().home_open());
    assert_eq!(harness.state().windows.len(), 2);
    harness.state_mut().request_close();
    for id in [first, second] {
        let window = harness.state_mut().windows.get_mut(&id).unwrap();
        window.persist(None, true, true, true).unwrap();
        assert_eq!(
            window.title(),
            if id == first {
                "first edit"
            } else {
                "second edit"
            }
        );
        idle(harness.state_mut());
    }
    assert!(harness.state().ready_to_close());
    assert!(harness.state().windows.is_empty());
    assert!(project::WriterLease::acquire(&paths[0]).is_ok());
    assert!(project::WriterLease::acquire(&paths[1]).is_ok());
}

#[test]
fn export_memory_pressure_cannot_consume_another_windows_autosave_allowance() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.create().unwrap();
    idle(&mut app);
    let first = app.focused_project().unwrap();
    app.create().unwrap();
    idle(&mut app);
    let second = app.focused_project().unwrap();
    let budget = app.editor_mut(first).unwrap().playback.budget().clone();
    let export = budget
        .reserve(engine::ByteOwner::Export, 256 << 20)
        .unwrap();
    let app_budget = app.control_budget();
    let snapshot = app_budget.snapshot().unwrap();
    let caches = app_budget
        .reserve(
            engine::ByteOwner::PersistentCache,
            snapshot.limit - snapshot.used,
        )
        .unwrap();
    assert_eq!(app.control_budget().snapshot().unwrap().used, 768 << 20);
    let path = root.path().join("autosave.vibecut");
    app.windows
        .get_mut(&second)
        .unwrap()
        .persist(Some(path.clone()), false, false, false)
        .unwrap();
    idle(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert!(path.is_file());
    let saved_id = project::load(&path).unwrap().project.data().id;
    assert_ne!(saved_id, second);
    assert!(
        !app.editor_mut(saved_id)
            .unwrap()
            .session
            .autosave()
            .unwrap()
            .is_dirty()
    );
    assert!(budget.snapshot().unwrap().used >= 256 << 20);
    drop(caches);
    drop(export);
}
