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
        if scale == 1.0 {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/p1-48-scenes.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            h.render().unwrap().save(&path).unwrap();
            println!("scene screenshot: {}", path.display());
        }
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
