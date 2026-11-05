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

fn track_editor() -> (Editor, project::TrackId, project::ItemId) {
    use engine::{NewItem, Placement};
    use project::{Color, ItemParams, TextParams};
    use time::{SourceRange, Tick};
    let mut s = Session::new(
        Project::empty(ProjectId(2), "Track management".into()).unwrap(),
        100,
    )
    .unwrap();
    for kind in [TrackKind::Video, TrackKind::Text, TrackKind::Audio] {
        s.apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: format!("{kind:?}").into(),
            kind,
        })
        .unwrap();
    }
    let text_track = s
        .project()
        .data()
        .scene()
        .timeline
        .overlays
        .iter()
        .find(|track| track.kind == TrackKind::Text)
        .unwrap()
        .id;
    let outcome = s
        .apply(Command::InsertItem {
            placement: Placement::Track { track: text_track },
            item: NewItem {
                animations: Default::default(),
                source_asset: None,
                source_range: SourceRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND * 4))
                    .unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Text(TextParams {
                    content: "Title".into(),
                    font_size: 48.0,
                    color: Color([255, 255, 255, 255]),
                }),
            },
        })
        .unwrap();
    let engine::CreatedId::Item(item) = outcome.created[0] else {
        panic!("item id")
    };
    let playback = Playback::new(&s, ByteBudget::new(64 << 20).unwrap(), Arc::new(Files)).unwrap();
    (
        Editor::new(s, playback, Preferences::default()),
        text_track,
        item,
    )
}

fn header_rect(h: &Harness<'_, Editor>, id: project::TrackId) -> egui::Rect {
    h.state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == format!("track.{}.header", id.0))
        .unwrap()
        .rect
}

fn drag(h: &mut Harness<'_, Editor>, from: egui::Pos2, to: egui::Pos2) {
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(2);
    h.drop_at(to);
    h.run_steps(3);
}

#[test]
fn rename_commits_on_enter_and_esc_keeps_the_old_name() {
    let (editor, track, _) = track_editor();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
    h.run_steps(4);
    let id = track.0;
    h.get(By::new().predicate(|n| n.author_id() == Some(format!("track.{id}.header").as_str())))
        .click_secondary();
    h.run_steps(2);
    h.get(
        By::new()
            .predicate(|n| n.author_id() == Some(format!("track.{id}.rename_action").as_str())),
    )
    .click();
    h.run_steps(2);
    assert!(
        h.state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == format!("track.{id}.rename"))
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("Đổi tên rãnh".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .find(|track| track.id.0 == id)
            .unwrap()
            .name
            .as_str(),
        "Đổi tên rãnh"
    );

    h.get(By::new().predicate(|n| n.author_id() == Some(format!("track.{id}.header").as_str())))
        .click_secondary();
    h.run_steps(2);
    h.get(
        By::new()
            .predicate(|n| n.author_id() == Some(format!("track.{id}.rename_action").as_str())),
    )
    .click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("discarded".into()));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .find(|track| track.id.0 == id)
            .unwrap()
            .name
            .as_str(),
        "Đổi tên rãnh"
    );
}

#[test]
fn drag_reorder_stays_inside_the_overlay_section() {
    let (editor, _, _) = track_editor();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
    h.run_steps(4);
    let before = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .overlays
        .clone();
    let from = header_rect(&h, before[0].id).center();
    let to = header_rect(&h, before[1].id).center();
    drag(&mut h, from, to);
    let after = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .overlays
        .clone();
    assert_eq!(after[0].id, before[1].id);
    assert_eq!(after[1].id, before[0].id);
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(
        h.state().session.project().data().scene().timeline.overlays,
        before
    );
    // Reversal, section boundaries and Escape must leave order/history intact.
    let history = h.state().session.history_len();
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(2);
    h.drop_at(from);
    h.run_steps(3);
    let audio = h.state().session.project().data().scene().timeline.audio[0].id;
    let audio_pos = header_rect(&h, audio).center();
    drag(&mut h, from, audio_pos);
    let main_pos = header_rect(&h, project::TrackId(0)).center();
    drag(&mut h, main_pos, to);
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(h.state().session.history_len(), history);
    assert_eq!(
        h.state().session.project().data().scene().timeline.overlays,
        before
    );
}

#[test]
fn leaving_or_removing_a_renamed_track_restores_keyboard_shortcuts() {
    for remove in [false, true] {
        let (editor, track, _) = track_editor();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
        h.run_steps(4);
        h.get(
            By::new()
                .predicate(|n| n.author_id() == Some(format!("track.{}.header", track.0).as_str())),
        )
        .click_secondary();
        h.run_steps(2);
        h.get(By::new().predicate(|n| {
            n.author_id() == Some(format!("track.{}.rename_action", track.0).as_str())
        }))
        .click();
        h.run_steps(2);
        h.event(egui::Event::Text("discarded draft".into()));
        h.run_steps(2);
        if remove {
            h.state_mut()
                .session
                .apply(Command::RemoveTrack { track })
                .unwrap();
        } else {
            let result = h
                .state_mut()
                .session
                .execute("scene.create", serde_json::json!({"name":"Other"}))
                .unwrap();
            let engine::CreatedId::Scene(scene) = result.created[0] else {
                panic!("scene")
            };
            h.state_mut()
                .activate(&format!("scene.row.{}", scene.0))
                .unwrap();
        }
        h.run_steps(3);
        let before = h.state().snapping;
        h.key_press(egui::Key::N);
        h.run_steps(3);
        assert_ne!(
            h.state().snapping,
            before,
            "orphan rename must not capture shortcuts"
        );
        assert!(
            !h.state()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|widget| widget.id == format!("track.{}.rename", track.0))
        );
    }
}

#[test]
fn lock_refuses_structural_edits_and_add_track_uses_the_active_kind() {
    let (mut editor, text_track, item) = track_editor();
    editor.selection.insert(item);
    editor
        .playback
        .seek(
            time::Tick(time::TICKS_PER_SECOND * 2),
            false,
            editor.session.project().clone(),
        )
        .unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
    h.run_steps(4);
    let id = text_track.0;
    h.get(By::new().predicate(|n| n.author_id() == Some(format!("track.{id}.header").as_str())))
        .click_secondary();
    h.run_steps(2);
    h.get(By::new().predicate(|n| n.author_id() == Some(format!("track.{id}.lock").as_str())))
        .click();
    h.run_steps(3);
    assert!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .find(|track| track.id == text_track)
            .unwrap()
            .locked
    );
    let items = |h: &Harness<'_, Editor>| {
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .count()
    };
    let history = h.state().session.history_len().0;
    assert!(matches!(
        h.state_mut().activate("timeline.split"),
        Err(vibecut::Error::Engine(engine::EditError::Locked(track))) if track == text_track
    ));
    h.get(By::new().predicate(|n| n.author_id() == Some("timeline.split")))
        .click();
    h.run_steps(3);
    assert_eq!(items(&h), 1);
    assert_eq!(h.state().session.history_len().0, history);

    let text_tracks = |h: &Harness<'_, Editor>| {
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .filter(|track| track.kind == TrackKind::Text)
            .count()
    };
    let before = text_tracks(&h);
    h.get(By::new().predicate(|n| n.author_id() == Some("timeline.add_track")))
        .click();
    h.run_steps(3);
    assert_eq!(text_tracks(&h), before + 1);
    let added_above = &h.state().session.project().data().scene().timeline.overlays;
    let active_index = added_above
        .iter()
        .position(|track| track.id == text_track)
        .unwrap();
    assert_eq!(added_above[active_index - 1].kind, TrackKind::Text);

    h.state_mut().selection.clear();
    let old_ids: Vec<_> = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .overlays
        .iter()
        .map(|track| track.id)
        .collect();
    h.get(By::new().predicate(|n| n.author_id() == Some("timeline.add_track")))
        .click();
    h.run_steps(3);
    let tracks = &h.state().session.project().data().scene().timeline.overlays;
    assert_eq!(tracks.len(), old_ids.len() + 1);
    assert_eq!(tracks.last().unwrap().kind, TrackKind::Video);
    assert!(!old_ids.contains(&tracks.last().unwrap().id));
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-54-track-mgmt.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    h.render().unwrap().save(&path).unwrap();
    println!("track management screenshot: {}", path.display());
}
