#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, Session};
use project::{Project, ProjectId, TrackKind};
use std::sync::Arc;
use vibecut::{
    Editor,
    catalog::Preferences,
    egui,
    playback::{Files, Playback},
};
fn editor() -> Editor {
    let mut s = Session::new(Project::empty(ProjectId(1), "Tracks".into()).unwrap(), 100).unwrap();
    // Lowest overlay first so each newly inserted kind appears above it.
    for kind in [
        TrackKind::Video,
        TrackKind::Text,
        TrackKind::Sticker,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
        TrackKind::Audio,
    ] {
        s.apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: format!("Private {kind:?} name").into(),
            kind,
        })
        .unwrap();
    }
    let playback = Playback::new(&s, ByteBudget::new(64 << 20).unwrap(), Arc::new(Files)).unwrap();
    Editor::new(s, playback, Preferences::default())
}
#[test]
fn track_headers_remain_operable_at_display_density() {
    for scale in [1.0, 2.0] {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor());
        h.run_steps(4);
        let inspect = h.state().inspect().unwrap();
        let main = inspect
            .widgets
            .iter()
            .find(|w| w.id == "track.0.header")
            .unwrap();
        assert_eq!(main.rect.width(), 112.0);
        assert_eq!(main.rect.height(), 65.0);
        for track in h.state().session.project().data().scene().timeline.iter() {
            let w = inspect
                .widgets
                .iter()
                .find(|w| w.id == format!("track.{}.header", track.id.0))
                .unwrap();
            let expected = match track.kind {
                TrackKind::Video => 65.0,
                TrackKind::Audio => 50.0,
                _ => 25.0,
            };
            assert_eq!(w.rect.height(), expected);
            assert!(
                h.query_all(By::new().predicate(
                    |n| n.author_id() == Some(format!("track.{}.header", track.id.0).as_str())
                ))
                .count()
                    == 1
            );
        }
        for pair in [6, 5, 4, 3, 2, 1, 0, 7].windows(2) {
            let row = |id| {
                inspect
                    .widgets
                    .iter()
                    .find(|widget| widget.id == format!("track.{id}.header"))
                    .unwrap()
                    .rect
            };
            assert!(row(pair[0]).bottom() < row(pair[1]).top());
        }
        // Main visibility and mute are independent real registry edits.
        h.get(By::new().predicate(|n| n.author_id() == Some("track.0.muted")))
            .click();
        h.run_steps(3);
        assert!(
            h.state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .muted
        );
        h.get(By::new().predicate(|n| n.author_id() == Some("track.0.hidden")))
            .click();
        h.run_steps(3);
        assert!(
            h.state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .hidden
        );
        h.get(By::new().predicate(|n| n.author_id() == Some("track.0.magnet")))
            .click();
        h.run_steps(3);
        assert!(!h.state().session.project().data().scene().timeline.magnet);
        h.get(By::new().predicate(|n| n.author_id() == Some("timeline.magnet")))
            .click();
        h.run_steps(3);
        assert!(h.state().session.project().data().scene().timeline.magnet);
        assert!(
            !h.state()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|w| w.id == "track.7.hidden")
        );
        let id = h.state().session.project().data().scene().timeline.overlays[0].id;
        h.get(
            By::new()
                .predicate(|n| n.author_id() == Some(format!("track.{}.header", id.0).as_str())),
        )
        .click_secondary();
        h.run_steps(3);
        h.get(
            By::new()
                .predicate(|n| n.author_id() == Some(format!("track.{}.delete", id.0).as_str())),
        )
        .click();
        h.run_steps(3);
        assert!(
            !h.state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|t| t.id == id)
        );
        h.get(By::new().predicate(|n| n.author_id() == Some("edit.undo")))
            .click();
        h.run_steps(3);
        assert!(
            h.state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|t| t.id == id)
        );
    }
}
