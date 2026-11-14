// Grouping: `timeline.group` / `timeline.ungroup` and the group-aware edits.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "validated test fixtures and assertions"
)]
use crate::support::session::{get, new_item, project, session};
use engine::{
    Command, CreatedId, EditError, ItemPasteSource, Move, Placement, Session, TrackPlacement,
    TrimEdge,
};
use project::{GroupId, ItemId, Project, TrackId, TrackKind};
use std::sync::Arc;
use time::Tick;

/// Items 10 (0..40) and 11 (60..100) on one overlay track, plus item 12 (10..50) on the other.
fn fixture() -> (Session, TrackId, TrackId) {
    let mut s = session();
    let tracks = &s.project().data().scene().timeline.overlays;
    let (first, second) = (tracks[0].id, tracks[1].id);
    let out = s
        .apply(Command::InsertItem {
            placement: Placement::Track { track: second },
            item: new_item(10, 40),
        })
        .unwrap();
    assert_eq!(out.created, [CreatedId::Item(ItemId(12))]);
    (s, first, second)
}
fn group(s: &mut Session, items: &[u64]) -> GroupId {
    let out = s
        .apply(Command::TimelineGroup {
            items: items.iter().map(|id| ItemId(*id)).collect(),
        })
        .unwrap();
    let [CreatedId::Group(group)] = out.created.as_slice() else {
        panic!("one created group");
    };
    *group
}
fn groups(s: &Session) -> Vec<(u64, Option<GroupId>)> {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .map(|i| (i.id.0, i.group))
        .collect()
}
fn spans(s: &Session) -> Vec<(u64, i64, i64)> {
    let mut all: Vec<_> = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .map(|i| (i.id.0, i.start.0, i.end().unwrap().0))
        .collect();
    all.sort_unstable();
    all
}

#[test]
fn group_and_ungroup_are_one_undo_step_across_tracks() {
    let (mut s, _, _) = fixture();
    let before = s.project().clone();
    let g = group(&mut s, &[10, 12]);
    assert_eq!(get(&s, 10).group, Some(g));
    assert_eq!(get(&s, 12).group, Some(g));
    assert_eq!(get(&s, 11).group, None);
    let grouped = s.project().clone();
    // Ungrouping any member dissolves the whole group.
    s.apply(Command::TimelineUngroup {
        items: vec![ItemId(12)],
    })
    .unwrap();
    assert!(groups(&s).iter().all(|(_, g)| g.is_none()));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(s.project(), &grouped));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(s.project(), &before));
}

#[test]
fn group_refusals_are_typed_and_change_nothing() {
    let (mut s, first, _) = fixture();
    let g = group(&mut s, &[10, 12]);
    let before = s.project().clone();
    let cases: Vec<(Command, &str)> = vec![
        (
            Command::TimelineGroup {
                items: vec![ItemId(11)],
            },
            "GroupTooSmall",
        ),
        (
            Command::TimelineGroup {
                items: vec![ItemId(10), ItemId(10)],
            },
            "Invalid",
        ),
        (
            Command::TimelineGroup {
                items: vec![ItemId(10), ItemId(99)],
            },
            "NoItem",
        ),
        (
            Command::TimelineGroup {
                items: vec![ItemId(10), ItemId(12)],
            },
            "AlreadyGrouped",
        ),
        (
            Command::TimelineUngroup {
                items: vec![ItemId(11)],
            },
            "NotGrouped",
        ),
        (Command::TimelineUngroup { items: vec![] }, "Invalid"),
    ];
    for (command, expected) in cases {
        let error = s.apply(command).unwrap_err();
        let name = match error {
            EditError::GroupTooSmall => "GroupTooSmall",
            EditError::AlreadyGrouped => "AlreadyGrouped",
            EditError::NotGrouped(_) => "NotGrouped",
            EditError::NoItem(_) => "NoItem",
            EditError::Invalid(_) => "Invalid",
            other => panic!("{other:?}"),
        };
        assert_eq!(name, expected);
        assert!(Arc::ptr_eq(s.project(), &before), "{expected}");
    }
    // A locked track holding a member refuses both commands.
    s.apply(Command::SetTrackLocked {
        track: first,
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    for command in [
        Command::TimelineGroup {
            items: vec![ItemId(11), ItemId(12)],
        },
        Command::TimelineUngroup {
            items: vec![ItemId(12)],
        },
    ] {
        assert!(matches!(
            s.apply(command),
            Err(EditError::Locked(track)) if track == first
        ));
        assert!(Arc::ptr_eq(s.project(), &locked));
    }
    assert_eq!(get(&s, 12).group, Some(g));
}

#[test]
fn grouping_a_member_of_a_group_merges_the_groups() {
    let (mut s, _, _) = fixture();
    let old = group(&mut s, &[10, 12]);
    let merged = group(&mut s, &[10, 11]);
    assert_ne!(old, merged);
    assert!(groups(&s).iter().all(|(_, g)| *g == Some(merged)));
}

#[test]
fn moving_a_member_moves_the_group_and_keeps_each_track() {
    let (mut s, first, second) = fixture();
    group(&mut s, &[10, 12]);
    s.apply(Command::MoveItem {
        item: ItemId(10),
        track: first,
        start: Tick(5),
    })
    .unwrap();
    assert_eq!(spans(&s), [(10, 5, 45), (11, 60, 100), (12, 15, 55)]);
    s.apply(Command::MoveItems {
        moves: vec![Move {
            item: ItemId(12),
            track: second,
            start: Tick(20),
        }],
    })
    .unwrap();
    assert_eq!(spans(&s), [(10, 10, 50), (11, 60, 100), (12, 20, 60)]);
    s.apply(Command::TimelineMoveItems {
        items: vec![ItemId(10)],
        delta: Tick(3),
        placement: None,
    })
    .unwrap();
    assert_eq!(spans(&s), [(10, 13, 53), (11, 60, 100), (12, 23, 63)]);
    s.undo().unwrap();
    assert_eq!(spans(&s)[0], (10, 10, 50));
}

#[test]
fn trimming_a_member_trims_the_group_as_a_set() {
    let (mut s, _, _) = fixture();
    group(&mut s, &[10, 12]);
    s.apply(Command::TrimEnd {
        item: ItemId(10),
        at: Tick(30),
    })
    .unwrap();
    assert_eq!(spans(&s), [(10, 0, 30), (11, 60, 100), (12, 10, 40)]);
    s.apply(Command::TrimStart {
        item: ItemId(12),
        at: Tick(15),
    })
    .unwrap();
    assert_eq!(spans(&s), [(10, 5, 30), (11, 60, 100), (12, 15, 40)]);
    s.apply(Command::TimelineTrim {
        items: vec![ItemId(12)],
        edge: TrimEdge::End,
        delta: Tick(-5),
    })
    .unwrap();
    assert_eq!(spans(&s), [(10, 5, 25), (11, 60, 100), (12, 15, 35)]);
    // A delta one member cannot take refuses the whole set.
    let before = s.project().clone();
    assert!(
        s.apply(Command::TrimEnd {
            item: ItemId(10),
            at: Tick(5),
        })
        .is_err()
    );
    assert!(Arc::ptr_eq(s.project(), &before));
}

#[test]
fn splitting_a_member_splits_the_group_under_the_cut_and_keeps_the_group() {
    let (mut s, _, _) = fixture();
    let g = group(&mut s, &[10, 12]);
    // The cut at 5 reaches item 10 only: item 12 starts at 10.
    let out = s
        .apply(Command::SplitAt {
            item: ItemId(10),
            at: Tick(5),
        })
        .unwrap();
    assert_eq!(out.created.len(), 1);
    assert!(
        groups(&s)
            .iter()
            .all(|(id, group)| (*id == 11) == group.is_none() && group.is_none_or(|v| v == g))
    );
    // The cut at 20 reaches the right piece of 10 and item 12.
    let out = s
        .apply(Command::SplitAt {
            item: ItemId(12),
            at: Tick(20),
        })
        .unwrap();
    assert_eq!(out.created.len(), 2);
    let members = groups(&s)
        .into_iter()
        .filter(|(_, v)| *v == Some(g))
        .count();
    assert_eq!(members, 5, "every piece keeps the group");
    s.undo().unwrap();
    assert_eq!(
        groups(&s)
            .into_iter()
            .filter(|(_, v)| *v == Some(g))
            .count(),
        3
    );
}

#[test]
fn split_left_and_right_reach_every_member_under_the_cut() {
    for left in [true, false] {
        let (mut s, _, _) = fixture();
        group(&mut s, &[10, 12]);
        let items = vec![ItemId(10)];
        let command = if left {
            Command::TimelineSplitLeft {
                items,
                at: Tick(20),
            }
        } else {
            Command::TimelineSplitRight {
                items,
                at: Tick(20),
            }
        };
        s.apply(command).unwrap();
        let spans = spans(&s);
        let kept: Vec<_> = spans.iter().filter(|(id, _, _)| *id != 11).collect();
        assert_eq!(kept.len(), 2, "one piece of each member survives");
        for (_, start, end) in kept {
            assert!(if left { *start >= 20 } else { *end <= 20 });
        }
        // Each survivor is the only holder of its group piece pair, so the pair stays grouped.
        let survivors: Vec<_> = groups(&s).into_iter().filter(|(id, _)| *id != 11).collect();
        assert!(survivors.iter().all(|(_, g)| g.is_some()));
    }
}

#[test]
fn deleting_a_member_deletes_the_group_and_a_lone_survivor_is_ungrouped() {
    let (mut s, _, _) = fixture();
    group(&mut s, &[10, 11, 12]);
    s.apply(Command::Delete {
        items: vec![ItemId(12)],
    })
    .unwrap();
    assert!(spans(&s).is_empty());
    // A track removal can leave one member: it is ungrouped, not refused.
    let (mut s, _, second) = fixture();
    group(&mut s, &[10, 12]);
    s.apply(Command::RemoveTrack { track: second }).unwrap();
    assert_eq!(groups(&s), [(10, None), (11, None)]);
}

#[test]
fn ripple_deleting_a_member_ripple_deletes_the_group_on_each_track() {
    let (mut s, _, _) = fixture();
    group(&mut s, &[10, 12]);
    s.apply(Command::RippleDelete { item: ItemId(10) }).unwrap();
    // 12 goes with 10; 11 closes the 40-tick span 10 left on its own track.
    assert_eq!(spans(&s), [(11, 20, 60)]);
}

#[test]
fn duplicating_a_member_duplicates_the_group_under_a_fresh_id() {
    let (mut s, _, _) = fixture();
    let g = group(&mut s, &[10, 12]);
    let out = s
        .apply(Command::DuplicateItems {
            items: vec![ItemId(12)],
        })
        .unwrap();
    let copies: Vec<ItemId> = out
        .created
        .iter()
        .filter_map(|id| match id {
            CreatedId::Item(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(copies.len(), 2);
    let fresh = get(&s, copies[0].0).group.unwrap();
    assert_ne!(fresh, g);
    assert_eq!(get(&s, copies[1].0).group, Some(fresh));
    assert_eq!(get(&s, 10).group, Some(g));
    assert_eq!(get(&s, 12).group, Some(g));
    assert_eq!(
        out.created
            .iter()
            .filter(|id| matches!(id, CreatedId::Group(_)))
            .count(),
        1
    );
}

#[test]
fn copy_and_cut_carry_the_group_but_the_paste_is_ungrouped() {
    let (mut s, _, _) = fixture();
    let g = group(&mut s, &[10, 12]);
    s.apply(Command::CopyItems {
        items: vec![ItemId(10)],
    })
    .unwrap();
    let out = s
        .apply(Command::PasteItems {
            at: Tick(200),
            source: ItemPasteSource::Stored,
            track: None,
        })
        .unwrap();
    assert_eq!(out.created.len(), 2, "the whole group is copied");
    for id in out.created {
        let CreatedId::Item(id) = id else {
            panic!("item id")
        };
        assert_eq!(get(&s, id.0).group, None);
    }
    assert_eq!(get(&s, 10).group, Some(g));
}

#[test]
fn magnet_closes_each_track_the_group_touches() {
    // Main holds A (0..40) and B (40..80); an overlay holds C (0..40) and D (40..80).
    let build = |grouped: bool| {
        let mut data = project().into_data();
        let template = data.scene().timeline.overlays[0].items[0].clone();
        let make = |id: u64, start: i64| {
            let mut item = template.clone();
            item.id = ItemId(id);
            item.start = Tick(start);
            item.source_range = crate::support::session::range(100, 140);
            item
        };
        data.scene_mut().timeline.main.items = vec![make(20, 0), make(21, 40)];
        let overlay = &mut data.scene_mut().timeline.overlays[0];
        overlay.items = vec![make(22, 0), make(23, 40)];
        if grouped {
            for track in data.scene_mut().timeline.iter_mut() {
                for item in &mut track.items {
                    if item.id == ItemId(20) || item.id == ItemId(22) {
                        item.group = Some(GroupId(90));
                    }
                }
            }
        }
        Session::new(Project::new(data).unwrap(), 10).unwrap()
    };
    let mut s = build(true);
    s.apply(Command::Delete {
        items: vec![ItemId(20)],
    })
    .unwrap();
    assert_eq!(spans(&s), [(21, 0, 40), (23, 0, 40)]);
    let mut control = build(false);
    control
        .apply(Command::Delete {
            items: vec![ItemId(20)],
        })
        .unwrap();
    assert_eq!(spans(&control), [(21, 0, 40), (22, 0, 40), (23, 40, 80)]);
}

#[test]
fn group_ids_survive_save_and_reload_and_new_ids_do_not_collide() {
    let (mut s, _, _) = fixture();
    let g = group(&mut s, &[10, 12]);
    let bytes = project::encode(s.project(), false).unwrap();
    let mut loaded = Session::new(project::decode(&bytes).unwrap().project, 10).unwrap();
    assert_eq!(get(&loaded, 12).group, Some(g));
    let next = group(&mut loaded, &[11, 12]);
    assert!(next.0 > g.0);
    let out = loaded
        .apply(Command::AddTrack {
            placement: TrackPlacement::Auto,
            name: "after".into(),
            kind: TrackKind::Video,
        })
        .unwrap();
    assert!(matches!(out.created.as_slice(), &[CreatedId::Track(TrackId(id))] if id > next.0));
}
