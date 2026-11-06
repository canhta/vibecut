//! Verifies the frame applies the actions the timeline returns.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use super::*;
use crate::{
    catalog::Preferences,
    playback::{Files, Playback},
};
use engine::CreatedId;
use engine::{ByteBudget, NewItem};
use project::{Project, ProjectId, TrackId, TrackKind};
use std::{collections::BTreeSet, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND};

const SECOND: i64 = TICKS_PER_SECOND;

struct Rig {
    editor: Editor,
    first: ItemId,
    second: ItemId,
    track: TrackId,
}

fn rig() -> Rig {
    let mut session =
        engine::Session::new(Project::empty(ProjectId(1), "Routing".into()).unwrap(), 50).unwrap();
    let track = match session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "Text".into(),
            kind: TrackKind::Text,
        })
        .unwrap()
        .created
        .first()
    {
        Some(CreatedId::Track(id)) => *id,
        _ => panic!("track"),
    };
    let mut insert = |start: i64| match session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: None,
                source_range: SourceRange::new(Tick(start), Tick(start + SECOND)).unwrap(),
                start: Tick(start),
                content: project::ItemParams::Text(project::TextParams {
                    content: "Hello".into(),
                    font_size: 48.0,
                    ..Default::default()
                }),
            },
        })
        .unwrap()
        .created
        .first()
    {
        Some(CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    };
    let first = insert(SECOND);
    let second = insert(4 * SECOND);
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Rig {
        editor: Editor::new(session, playback, Preferences::default()),
        first,
        second,
        track,
    }
}

fn shift(item: ItemId, track: TrackId, start: i64) -> timeline::Action {
    timeline::Action::Preview(Box::new(Command::MoveItems {
        moves: vec![engine::Move {
            item,
            track,
            start: Tick(start),
        }],
    }))
}

fn start_of(editor: &Editor, item: ItemId) -> Tick {
    editor
        .session
        .preview_project()
        .unwrap_or(editor.session.project())
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .unwrap()
        .start
}

#[test]
fn previews_and_drops_outside_an_open_gesture_edit_nothing() {
    let Rig {
        mut editor,
        first,
        track,
        ..
    } = rig();
    let project = editor.session.project().clone();
    let history = editor.session.history_len();
    editor.route_timeline(vec![
        shift(first, track, 9 * SECOND),
        timeline::Action::MoveToTrack {
            item: first,
            start: Tick(9 * SECOND),
            target: timeline::DropTarget::Track(track),
        },
        timeline::Action::EndGesture,
    ]);
    assert!(Arc::ptr_eq(&project, editor.session.project()));
    assert_eq!(editor.session.history_len(), history);
    assert!(editor.error.is_none());
}

#[test]
fn a_gesture_publishes_one_undo_step_from_many_previews() {
    let Rig {
        mut editor,
        first,
        track,
        ..
    } = rig();
    let history = editor.session.history_len().0;
    editor.route_timeline(vec![
        timeline::Action::BeginGesture {
            select: Some(first),
        },
        shift(first, track, 2 * SECOND),
        shift(first, track, 3 * SECOND),
    ]);
    assert!(editor.session.gesture_active());
    assert_eq!(
        editor.selection,
        BTreeSet::from([first]),
        "the grab selects"
    );
    assert_eq!(start_of(&editor, first), Tick(3 * SECOND));
    assert_eq!(
        editor.session.history_len().0,
        history,
        "previews are not history"
    );
    editor.route_timeline(vec![timeline::Action::EndGesture]);
    assert!(!editor.session.gesture_active());
    assert_eq!(editor.session.history_len().0, history + 1);
    assert_eq!(start_of(&editor, first), Tick(3 * SECOND));
}

#[test]
fn a_refused_track_drop_rolls_the_whole_gesture_back() {
    let Rig {
        mut editor,
        first,
        track,
        ..
    } = rig();
    let before = editor.session.project().clone();
    let history = editor.session.history_len();
    editor.route_timeline(vec![
        timeline::Action::BeginGesture { select: None },
        shift(first, track, 2 * SECOND),
        timeline::Action::MoveToTrack {
            item: first,
            start: Tick(2 * SECOND),
            target: timeline::DropTarget::Track(TrackId(999)),
        },
        // The release also ends the gesture; there is nothing left to publish.
        timeline::Action::EndGesture,
    ]);
    assert!(editor.error.is_some(), "the refusal is reported");
    assert!(!editor.session.gesture_active());
    assert_eq!(editor.session.project(), &before);
    assert_eq!(editor.session.history_len(), history);
}

#[test]
fn selection_requests_update_the_selection_and_the_session_copy() {
    let Rig {
        mut editor,
        first,
        second,
        ..
    } = rig();
    editor.route_timeline(vec![timeline::Action::Select {
        item: first,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(editor.selection, BTreeSet::from([first]));
    assert_eq!(editor.session.selection(), &BTreeSet::from([first]));
    editor.route_timeline(vec![
        timeline::Action::Marquee(BTreeSet::from([first, second])),
        timeline::Action::SettleSelection,
    ]);
    assert_eq!(editor.selection, BTreeSet::from([first, second]));
    assert_eq!(editor.session.selection(), &editor.selection);
    editor.route_timeline(vec![timeline::Action::ClearSelection]);
    assert!(editor.selection.is_empty() && editor.session.selection().is_empty());
}

#[test]
fn commands_seeks_activations_scene_renames_and_failures_reach_their_owners() {
    let Rig {
        mut editor, track, ..
    } = rig();
    editor.route_timeline(vec![timeline::Action::Command {
        id: "track.set_locked",
        params: serde_json::json!({"track":track,"locked":true}),
    }]);
    assert!(
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .any(|candidate| candidate.id == track && candidate.locked)
    );
    editor.route_timeline(vec![timeline::Action::Seek(Tick(2 * SECOND))]);
    assert_eq!(editor.playback.position().unwrap(), Tick(2 * SECOND));
    assert!(!editor.preferences.ripple);
    editor.route_timeline(vec![timeline::Action::Activate("timeline.ripple".into())]);
    assert!(
        editor.preferences.ripple,
        "activation goes through the action registry"
    );
    // A rename the engine refuses is reported and its draft stays open.
    editor.route_timeline(vec![timeline::Action::RenameScene {
        scene: project::SceneId(99),
        name: "Nowhere".into(),
    }]);
    assert!(editor.error.take().is_some());
    assert!(editor.timeline.scenes.is_renaming());
    let scene = editor.session.project().data().current_scene_id;
    editor.route_timeline(vec![timeline::Action::RenameScene {
        scene,
        name: "Renamed".into(),
    }]);
    assert!(editor.error.is_none());
    assert_eq!(
        editor.session.project().data().scene().name.as_str(),
        "Renamed"
    );
    editor.route_timeline(vec![timeline::Action::Fail("nope".into())]);
    assert_eq!(editor.error.as_deref(), Some("nope"));
}
