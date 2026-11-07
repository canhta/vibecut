#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use engine::{Command, Session};
use project::{Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId};
use serde_json::json;
use time::{SourceRange, Tick};

pub(crate) fn session(with_transition: bool) -> Session {
    let mut d = Project::empty(ProjectId(0), "transitions".into())
        .unwrap()
        .into_data();
    d.scene_mut().timeline.main.id = project::TrackId(1);
    d.scene_mut().timeline.main.items = [10, 11, 12]
        .into_iter()
        .enumerate()
        .map(|(n, id)| Item {
            hidden: false,
            retime: Default::default(),
            id: ItemId(id),
            source_asset: None,
            source_range: SourceRange::new(Tick(100), Tick(200)).unwrap(),
            start: Tick(n as i64 * 100),
            content: ItemParams::Overlay(OverlayParams {
                color: Color([255, 0, 0, 255]),
            }),
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
        })
        .collect();
    let mut s = Session::new(Project::new(d).unwrap(), 100).unwrap();
    if with_transition {
        add(&mut s, "center", 40).unwrap();
    }
    s
}
fn add(s: &mut Session, alignment: &str, duration: i64) -> engine::Result<engine::Outcome> {
    s.execute("transition.add", json!({"track":1,"from":10,"to":11,"kind":"cross_dissolve","duration":duration,"alignment":alignment}))
}
fn tr(s: &Session) -> &project::Transition {
    &s.project().data().scene().timeline.transitions[0]
}

#[test]
fn add_move_resize_remove_are_atomic_undoable_and_persisted() {
    let mut s = session(false);
    let before = s.project().clone();
    let result = add(&mut s, "center", 40).unwrap();
    let id = tr(&s).id.0;
    assert_eq!(
        result.created,
        vec![engine::CreatedId::Transition(project::TransitionId(id))]
    );
    let added = s.project().clone();
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    assert_eq!(s.project(), &added);
    for (cmd, params) in [
        ("transition.move", json!({"transition":id,"start":85})),
        (
            "transition.set_duration",
            json!({"transition":id,"duration":50}),
        ),
        ("transition.remove", json!({"transition":id})),
    ] {
        let before = s.project().clone();
        s.execute(cmd, params).unwrap();
        let after = s.project().clone();
        assert_ne!(before, after);
        assert_eq!(
            project::decode(&project::encode(&after, false).unwrap())
                .unwrap()
                .project,
            *after
        );
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
        s.redo().unwrap();
        assert_eq!(s.project(), &after);
    }
}

#[test]
fn spans_clamp_by_alignment_and_refuse_overlap_hostile_and_locked_edits() {
    let mut s = session(true);
    let id = tr(&s).id.0;
    for (cmd, params) in [
        (
            "transition.add",
            json!({"track":1,"from":10,"to":11,"kind":"cross_dissolve","duration":1,"alignment":"start"}),
        ),
        (
            "transition.add",
            json!({"track":1,"from":10,"to":12,"kind":"cross_dissolve","duration":10,"alignment":"center"}),
        ),
        (
            "transition.add",
            json!({"track":1,"from":11,"to":12,"kind":"spin_motion","duration":10,"alignment":"center"}),
        ),
        (
            "transition.set_duration",
            json!({"transition":id,"duration":0}),
        ),
        (
            "transition.set_duration",
            json!({"transition":id,"duration":-1}),
        ),
        ("transition.remove", json!({"transition":999})),
    ] {
        let before = s.project().clone();
        assert!(s.execute(cmd, params).is_err());
        assert_eq!(s.project(), &before);
    }
    for (start, expected) in [(i64::MIN, 60), (i64::MAX, 100)] {
        s.apply(Command::MoveTransition {
            transition: project::TransitionId(id),
            start: Tick(start),
        })
        .unwrap();
        assert_eq!(tr(&s).start, Tick(expected));
    }
    s.execute("track.set_locked", json!({"track":1,"locked":true}))
        .unwrap();
    assert!(
        s.execute("transition.remove", json!({"transition":id}))
            .is_err()
    );
    assert!(
        matches!(project::transition_definition("spin_motion"),Err(project::Error::UnsupportedTransition(kind)) if kind == "spin_motion")
    );
}

#[test]
fn clip_moves_trims_split_and_delete_revalidate_without_losing_history() {
    let mut s = session(true);
    s.execute("split_at", json!({"item":10,"at":50})).unwrap();
    let outgoing = tr(&s).from;
    assert_ne!(outgoing, ItemId(10));
    assert_eq!((tr(&s).start, tr(&s).duration), (Tick(80), Tick(40)));
    let split = s.project().clone();
    s.execute("trim_end", json!({"item":outgoing.0,"at":90}))
        .unwrap();
    assert!(s.project().data().scene().timeline.transitions.is_empty());
    s.undo().unwrap();
    assert_eq!(s.project(), &split);
    s.execute("trim_start", json!({"item":outgoing.0,"at":90}))
        .unwrap();
    assert_eq!(tr(&s).duration, Tick(21)); // odd centre: ten before, eleven after
    s.undo().unwrap();
    s.execute("move_item", json!({"item":11,"track":1,"start":400}))
        .unwrap();
    assert!(s.project().data().scene().timeline.transitions.is_empty());
    s.undo().unwrap();
    s.execute("delete", json!({"items":[11]})).unwrap();
    assert!(s.project().data().scene().timeline.transitions.is_empty());
}

#[test]
fn cancelled_gestures_failed_batches_and_reversal_restore_transitions() {
    let mut s = session(true);
    let before = s.project().clone();
    let id = tr(&s).id.0;
    s.begin_gesture().unwrap();
    s.execute("transition.move", json!({"transition":id,"start":90}))
        .unwrap();
    s.execute(
        "transition.set_duration",
        json!({"transition":id,"duration":80}),
    )
    .unwrap();
    s.cancel_gesture().unwrap();
    assert_eq!(s.project(), &before);
    assert!(
        s.execute(
            "batch",
            json!({"commands":[
                {"command":"transition.remove","params":{"transition":id}},
                {"command":"transition.remove","params":{"transition":id}}
            ]})
        )
        .is_err()
    );
    assert_eq!(s.project(), &before);
    s.begin_gesture().unwrap();
    s.execute("transition.move", json!({"transition":id,"start":95}))
        .unwrap();
    s.execute("transition.move", json!({"transition":id,"start":80}))
        .unwrap();
    s.commit_gesture().unwrap();
    assert_eq!(s.project(), &before);
}

#[test]
fn current_schema_roundtrip_rejects_old_and_unknown_kinds() {
    let s = session(true);
    let bytes = project::encode(s.project(), false).unwrap();
    let mut wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let current = project::SCHEMA_VERSION;
    assert_eq!(wire["schema_version"], current);
    wire["schema_version"] = json!(current - 1);
    assert!(matches!(
        project::decode(&serde_json::to_vec(&wire).unwrap()),
        Err(project::Error::TooOld {
            found,
            supported
        }) if found == current - 1 && supported == current
    ));
    wire["schema_version"] = json!(current);
    wire["project"]["scenes"][0]["timeline"]["transitions"][0]["kind"] = json!("unknown");
    assert!(
        matches!(project::decode(&serde_json::to_vec(&wire).unwrap()),Err(project::Error::UnsupportedTransition(kind)) if kind == "unknown")
    );
}

#[test]
fn joint_moves_incoming_splits_and_cancelled_additions_keep_identity_and_cut() {
    let mut s = session(true);
    // Move the third clip away, leaving space for a joint translation of the cut.
    s.execute("move_item", json!({"item":12,"track":1,"start":400}))
        .unwrap();
    s.execute(
        "move_items",
        json!({"moves":[{"item":10,"track":1,"start":50},{"item":11,"track":1,"start":150}]}),
    )
    .unwrap();
    assert_eq!((tr(&s).start, tr(&s).duration), (Tick(130), Tick(40)));
    s.execute("split_at", json!({"item":11,"at":160})).unwrap();
    assert_eq!(tr(&s).to, ItemId(11));
    assert_eq!((tr(&s).start, tr(&s).duration), (Tick(130), Tick(40)));
    let mut s = session(false);
    s.begin_gesture().unwrap();
    add(&mut s, "center", 40).unwrap();
    let retired = tr(&s).id;
    s.cancel_gesture().unwrap();
    add(&mut s, "center", 40).unwrap();
    assert!(tr(&s).id > retired);
    let loaded = project::decode(&project::encode(s.project(), false).unwrap())
        .unwrap()
        .project;
    let max_id = tr(&s).id;
    let mut s = Session::new(loaded, 100).unwrap();
    s.execute("transition.remove", json!({"transition":max_id.0}))
        .unwrap();
    add(&mut s, "center", 40).unwrap();
    assert!(tr(&s).id > max_id);
}

#[test]
fn ripple_preserves_each_transition_offset_through_exactly_one_shift() {
    let base = session(false);
    let mut data = base.project().data().clone();
    for item in &mut data.scene_mut().timeline.main.items {
        item.start += Tick(100);
    }
    let mut anchor = data.scene().timeline.main.items[0].clone();
    anchor.id = ItemId(9);
    anchor.start = Tick::ZERO;
    anchor.source_range = SourceRange::new(Tick(100), Tick(110)).unwrap();
    data.scene_mut().timeline.main.items.insert(0, anchor);
    let mut s = Session::new(Project::new(data).unwrap(), 100).unwrap();
    add(&mut s, "center", 40).unwrap();
    let before = s.project().clone();
    s.execute_with_ripple(
        "move_items",
        json!({"moves":[
            {"item":10,"track":1,"start":50},
            {"item":11,"track":1,"start":150},
            {"item":12,"track":1,"start":250}
        ]}),
        true,
    )
    .unwrap();
    assert_eq!(tr(&s).start, Tick(130));
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    let mut s = session(false);
    s.execute("transition.add", json!({"track":1,"from":11,"to":12,"kind":"cross_dissolve","duration":40,"alignment":"center"})).unwrap();
    s.execute_with_ripple("delete", json!({"items":[10]}), true)
        .unwrap();
    assert_eq!(tr(&s).start, Tick(80));
}
