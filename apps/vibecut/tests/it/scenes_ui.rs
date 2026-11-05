#![allow(clippy::unwrap_used, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, CreatedId, Session};
use project::{Project, ProjectId, SceneId};
use std::sync::Arc;
use vibecut::{
    Editor,
    catalog::Preferences,
    egui,
    playback::{Files, Playback},
};
#[test]
fn scene_sheet_switches_and_multideletes_with_main_protected() {
    for scale in [1.0, 2.0] {
        let mut session =
            Session::new(Project::empty(ProjectId(1), "Scenes".into()).unwrap(), 30).unwrap();
        let mut ids = Vec::new();
        for name in ["Opening", "Closing"] {
            let created = session
                .execute("scene.create", serde_json::json!({"name":name}))
                .unwrap();
            match created.created.first() {
                Some(CreatedId::Scene(id)) => ids.push(*id),
                _ => panic!("scene"),
            }
        }
        let playback = Playback::new(
            &session,
            ByteBudget::new(64 << 20).unwrap(),
            Arc::new(Files),
        )
        .unwrap();
        let editor = Editor::new(session, playback, Preferences::default());
        let mut h = Harness::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
        h.run_steps(3);
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.open")))
            .click();
        h.run_steps(3);
        let first = ids.first().unwrap();
        let row = format!("scene.row.{}", first.0);
        h.get(By::new().predicate(|n| n.author_id() == Some(row.as_str())))
            .click();
        h.run_steps(3);
        assert_eq!(h.state().session.project().data().current_scene_id, *first);
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.select")))
            .click();
        h.run_steps(3);
        let inspect = h.state().inspect().unwrap();
        assert!(
            !inspect
                .widgets
                .iter()
                .find(|w| w.id == "scene.row.0")
                .unwrap()
                .enabled
        );
        assert!(
            !inspect
                .widgets
                .iter()
                .find(|w| w.id == "scene.delete")
                .unwrap()
                .enabled
        );
        for id in &ids {
            let row = format!("scene.row.{}", id.0);
            h.get(By::new().predicate(|n| n.author_id() == Some(row.as_str())))
                .click();
            h.run_steps(3);
        }
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.delete")))
            .click();
        h.run_steps(3);
        assert!(
            h.query_by_label("Delete selected scenes? You can undo this action.")
                .is_some()
        );
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.cancel_delete")))
            .click();
        h.run_steps(3);
        assert_eq!(h.state().session.project().data().scenes.iter().count(), 3);
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.delete")))
            .click();
        h.run_steps(3);
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.confirm_delete")))
            .click();
        h.run_steps(3);
        assert_eq!(h.state().session.project().data().scenes.iter().count(), 1);
        assert_eq!(
            h.state().session.project().data().current_scene_id,
            SceneId(0)
        );
        h.state_mut().activate("scene.close").unwrap();
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.project().data().scenes.iter().count(), 3);
    }
}

fn editor_with_scene_session() -> Editor {
    let session = Session::new(
        Project::empty(ProjectId(72), "Scene project".into()).unwrap(),
        30,
    )
    .unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Editor::new(session, playback, Preferences::default())
}
fn click(h: &mut Harness<'_, Editor>, id: &str) {
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some(id)))
        .click();
    h.run_steps(3);
}
fn scenes_len(h: &Harness<'_, Editor>) -> usize {
    h.state().session.project().data().scenes.iter().count()
}
#[test]
fn deleting_a_scene_with_a_rename_draft_preserves_escape_dismissal() {
    for (cancel_confirmation, press_enter) in [(false, false), (true, false), (true, true)] {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor_with_scene_session());
        click(&mut h, "scene.open");
        click(&mut h, "scene.create");
        let created = h.state().session.project().data().current_scene_id;
        let row = format!("scene.row.{}", created.0);
        click(&mut h, "scene.select");
        click(&mut h, &row);
        h.get(By::new().predicate(|n| n.author_id() == Some(row.as_str())))
            .click_secondary();
        h.run_steps(3);
        click(&mut h, &format!("scene.rename.{}", created.0));
        h.get(By::new().predicate(|n| n.author_id() == Some("scene.rename.input")))
            .type_text(" draft");
        click(&mut h, "scene.delete");
        if cancel_confirmation {
            if press_enter {
                h.key_press(egui::Key::Enter);
                h.run_steps(3);
                assert!(
                    h.state()
                        .inspect()
                        .unwrap()
                        .widgets
                        .iter()
                        .any(|w| w.id == "scene.confirm_delete")
                );
            }
            h.key_press(egui::Key::Escape);
            h.run_steps(3);
            let inspect = h.state().inspect().unwrap();
            assert!(
                !inspect
                    .widgets
                    .iter()
                    .any(|w| w.id == "scene.confirm_delete")
            );
            assert!(inspect.widgets.iter().any(|w| w.id == "scene.rename.input"));
            assert_eq!(
                h.state()
                    .session
                    .project()
                    .data()
                    .scenes
                    .get(created)
                    .unwrap()
                    .name
                    .as_str(),
                "Scene 2"
            );
            click(&mut h, "scene.delete");
        }
        click(&mut h, "scene.confirm_delete");
        assert_eq!(scenes_len(&h), 1);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(
            !h.state()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|w| w.id == "scene.create")
        );
    }
}
#[test]
fn create_switches_to_an_empty_scene_and_history_keeps_the_project_exact() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor_with_scene_session());
    h.run_steps(3);
    let before = h.state().session.project().clone();
    click(&mut h, "scene.open");
    click(&mut h, "scene.create");
    let created = h.state().session.project().data().current_scene_id;
    assert_ne!(created, SceneId(0));
    assert_eq!(scenes_len(&h), 2);
    {
        let data = h.state().session.project().data();
        let scene = data.scenes.get(created).unwrap();
        assert_eq!(scene.name.as_str(), "Scene 2");
        assert_eq!(scene.timeline.track_count(), 1);
        assert!(scene.timeline.main.items.is_empty());
        assert!(scene.timeline.overlays.is_empty());
        assert!(scene.timeline.audio.is_empty());
        assert_eq!(data.canvas, before.data().canvas);
    }
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.project().data(), before.data());
    h.state_mut().activate("edit.redo").unwrap();
    h.run_steps(3);
    assert_eq!(scenes_len(&h), 2);
    click(&mut h, "scene.row.0");
    let after_create = h.state().session.project().clone();
    click(&mut h, "scene.select");
    click(&mut h, &format!("scene.row.{}", created.0));
    click(&mut h, "scene.delete");
    click(&mut h, "scene.confirm_delete");
    assert_eq!(scenes_len(&h), 1);
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.project().data(), after_create.data());
}
#[test]
fn rename_commits_cancels_renames_main_and_survives_persistence() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor_with_scene_session());
    h.run_steps(3);
    click(&mut h, "scene.open");
    click(&mut h, "scene.create");
    let created = h.state().session.project().data().current_scene_id;
    h.get(
        By::new().predicate(|n| n.author_id() == Some(format!("scene.row.{}", created.0).as_str())),
    )
    .click_secondary();
    h.run_steps(3);
    h.get(
        By::new()
            .predicate(|n| n.author_id() == Some(format!("scene.rename.{}", created.0).as_str())),
    )
    .click();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("scene.rename.input")))
        .type_text(" Opening");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scenes
            .get(created)
            .unwrap()
            .name
            .as_str(),
        "Scene 2 Opening"
    );
    h.state_mut()
        .activate(&format!("scene.rename.{}", created.0))
        .unwrap();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("scene.rename.input")))
        .type_text(" cancelled");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scenes
            .get(created)
            .unwrap()
            .name
            .as_str(),
        "Scene 2 Opening"
    );
    h.get(By::new().predicate(|n| n.author_id() == Some("scene.row.0")))
        .click_secondary();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("scene.rename.0")))
        .click();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some("scene.rename.input")))
        .type_text(" Main");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scenes
            .main()
            .name
            .as_str(),
        "Main scene Main"
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scenes
            .main()
            .name
            .as_str(),
        "Main scene"
    );
    h.state_mut().activate("edit.redo").unwrap();
    h.run_steps(3);
    let bytes = project::encode(h.state().session.project(), false).unwrap();
    let decoded = project::decode(&bytes).unwrap().project;
    assert_eq!(&decoded, h.state().session.project().as_ref());
    assert_eq!(
        decoded.data().scenes.get(created).unwrap().name.as_str(),
        "Scene 2 Opening"
    );
    assert_eq!(
        decoded.data().scenes.main().name.as_str(),
        "Main scene Main"
    );
}
