#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{Wire, new_item, project, range};
use engine::{Command, EditError, Move, Session, TrimEdge};
use project::{ItemId, Project, TrackId};
use proptest::prelude::*;
use serde_json::json;
use time::Tick;

fn ripple_fixture(lengths: &[i64], gap: i64) -> (Session, TrackId, TrackId) {
    let mut data = project().into_data();
    let track = data.scene().timeline.overlays.first().unwrap().id;
    let other = data.scene().timeline.overlays.get(1).unwrap().id;
    let template = data
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .clone();
    let mut start = gap;
    data.scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items = lengths
        .iter()
        .enumerate()
        .map(|(i, length)| {
            let mut item = template.clone();
            item.id = ItemId(10 + i as u64);
            item.start = Tick(start);
            item.source_range = range(100, 100 + length);
            start += length;
            item
        })
        .collect();
    let mut marker = template;
    marker.id = ItemId(30);
    marker.start = Tick(100_000);
    data.scene_mut().timeline.overlays.get_mut(1).unwrap().items = vec![marker];
    (
        Session::new(Project::new(data).unwrap(), 100).unwrap(),
        track,
        other,
    )
}
fn ripple_layout(s: &Session) -> Vec<(TrackId, ItemId, Tick, Tick)> {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| {
            t.items
                .iter()
                .map(move |i| (t.id, i.id, i.start, i.end().unwrap()))
        })
        .collect()
}
proptest! {
    #[test]
    fn ripple_edit_properties_preserve_adjacency_scope_and_history(
        lengths in prop::collection::vec(2i64..1000, 5),
        gap in 1i64..1000, index in 0usize..5, operation in 0u8..6,
        enabled in any::<bool>(),
    ) {
        let (mut s, track, other) = ripple_fixture(&lengths, gap);
        let before = project::encode(s.project(), false).unwrap();
        let before_layout = ripple_layout(&s);
        let start = gap + lengths.iter().take(index).sum::<i64>();
        let command = match operation {
            0 => Command::Delete { items: vec![ItemId(10 + index as u64)] },
            1 => Command::TrimEnd { item: ItemId(10 + index as u64), at: Tick(start + lengths[index] - 1) },
            2 => Command::MoveItem { item: ItemId(10), track, start: Tick(0) },
            3 => Command::MoveItem { item: ItemId(10), track: other, start: Tick::ZERO },
            4 => Command::SplitAt { item: ItemId(10 + index as u64), at: Tick(start + 1) },
            _ => Command::SetTrackFlags { track, hidden: true, muted: true, locked: false },
        };
        // Off is exactly the pre-existing edit semantics, including allocated ids.
        let mut baseline = Session::new(s.project().as_ref().clone(), 100).unwrap();
        baseline.apply(command.clone()).unwrap();
        s.apply(Command::TimelineEdit { commands: vec![command], ripple: enabled }).unwrap();
        let after_layout = ripple_layout(&s);
        if !enabled || operation >= 3 {
            prop_assert_eq!(s.project(), baseline.project());
        } else {
            let items: Vec<_> = after_layout.iter().filter(|row| row.0 == track).collect();
            for pair in items.windows(2) {
                prop_assert_eq!(pair[0].3, pair[1].2);
            }
        }
        prop_assert_eq!(after_layout.iter().find(|row| row.1 == ItemId(30)), before_layout.iter().find(|row| row.1 == ItemId(30)));
        let after = project::encode(s.project(), false).unwrap();
        prop_assert_eq!(s.history_len(), (1, 0));
        s.apply(Command::Undo {}).unwrap();
        prop_assert_eq!(project::encode(s.project(), false).unwrap(), before);
        s.apply(Command::Redo {}).unwrap();
        prop_assert_eq!(project::encode(s.project(), false).unwrap(), after);
    }
}
#[test]
fn ripple_preview_reversal_discard_and_refusal_preserve_the_baseline() {
    let (mut s, track, _) = ripple_fixture(&[40, 40, 40], 20);
    let before = project::encode(s.project(), false).unwrap();
    s.apply(Command::TimelineSetRipple { enabled: true })
        .unwrap();
    let preview = |at| Command::TimelinePreview {
        commands: vec![Command::TrimEnd {
            item: ItemId(10),
            at: Tick(at),
        }],
    };
    // The desktop starts a gesture before its first replacement preview.
    s.begin_gesture().unwrap();
    s.apply(Command::TimelineEdit {
        commands: vec![preview(40)],
        ripple: true,
    })
    .unwrap();
    assert_eq!(
        s.preview_project()
            .unwrap()
            .data()
            .scene()
            .timeline
            .overlays[0]
            .items[1]
            .start,
        Tick(40)
    );
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    assert!(matches!(
        s.apply(Command::TimelineSetRipple { enabled: false }),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    s.apply(Command::TimelineEdit {
        commands: vec![preview(60)],
        ripple: true,
    })
    .unwrap();
    assert_eq!(s.preview_project().unwrap().as_ref(), s.project().as_ref());
    s.apply(Command::TimelineEdit {
        commands: vec![preview(40)],
        ripple: true,
    })
    .unwrap();
    assert!(
        s.apply(Command::TimelineEdit {
            commands: vec![preview(-1)],
            ripple: true
        })
        .is_err()
    );
    // Typed entries may wrap one preview in the policy; the wire still refuses
    // a session control inside an edit batch.
    assert!(
        s.wire(
            "timeline.edit",
            json!({"commands":[{"command":"timeline.preview","params":{"commands":[]}}],"ripple":true})
        )
        .is_err()
    );
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    assert_eq!(s.history_len(), (0, 0));
    s.apply(Command::Delete {
        items: vec![ItemId(10)],
    })
    .unwrap();
    assert_eq!(
        ripple_layout(&s)
            .iter()
            .find(|row| row.1 == ItemId(11))
            .unwrap()
            .2,
        Tick(20)
    );
    s.apply(Command::Undo {}).unwrap();
    s.begin_gesture().unwrap();
    s.apply(Command::TrimEnd {
        item: ItemId(10),
        at: Tick(40),
    })
    .unwrap();
    let staged = s.project().clone();
    assert!(matches!(
        s.apply(Command::TimelineEdit {
            commands: vec![preview(60)],
            ripple: true
        }),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    assert_eq!(s.project(), &staged);
    s.cancel_gesture().unwrap();
    s.apply(Command::SetTrackFlags {
        track,
        hidden: false,
        muted: false,
        locked: true,
    })
    .unwrap();
    let locked = project::encode(s.project(), false).unwrap();
    assert!(matches!(
        s.apply(Command::Delete {
            items: vec![ItemId(10)]
        }),
        Err(EditError::Locked(_))
    ));
    assert_eq!(project::encode(s.project(), false).unwrap(), locked);
    assert!(crate::wire::run(&mut s, "timeline.set_ripple", json!({"enabled":"yes"})).is_err());
    assert!(
        s.apply(Command::Batch {
            commands: vec![Command::TimelineSetRipple { enabled: false }]
        })
        .is_err()
    );
}
#[test]
fn ripple_batch_subtracts_occupied_spans_and_never_closes_an_old_gap_twice() {
    let (mut s, track, _) = ripple_fixture(&[10, 10, 10], 20);
    s.apply(Command::TimelineSetRipple { enabled: true })
        .unwrap();
    s.apply(Command::Batch {
        commands: vec![
            Command::RippleDelete { item: ItemId(10) },
            Command::RippleDelete { item: ItemId(12) },
        ],
    })
    .unwrap();
    assert_eq!(
        ripple_layout(&s)
            .iter()
            .find(|row| row.1 == ItemId(11))
            .unwrap()
            .2,
        Tick(20)
    );
    s.apply(Command::Undo {}).unwrap();
    // Material inserted into a deletion's old span must not be shifted into overlap.
    s.apply(Command::Batch {
        commands: vec![
            Command::Delete {
                items: vec![ItemId(10)],
            },
            Command::InsertItem {
                placement: engine::Placement::Track { track },
                item: new_item(20, 5),
            },
        ],
    })
    .unwrap();
    assert_eq!(
        ripple_layout(&s)
            .iter()
            .find(|row| row.1 == ItemId(11))
            .unwrap()
            .2,
        Tick(25)
    );
    // A move wholly before its original span still keeps its old follower adjacent.
    let (mut s, track, _) = ripple_fixture(&[10, 10], 50);
    s.apply(Command::TimelineEdit {
        commands: vec![Command::MoveItem {
            item: ItemId(10),
            track,
            start: Tick(0),
        }],
        ripple: true,
    })
    .unwrap();
    assert_eq!(
        ripple_layout(&s)
            .iter()
            .find(|row| row.1 == ItemId(11))
            .unwrap()
            .2,
        Tick(10)
    );
    // Overlapping group tails are a union, not two independently counted gaps.
    let (mut s, track, _) = ripple_fixture(&[10, 10, 10], 50);
    s.apply(Command::TimelineEdit {
        commands: vec![Command::MoveItems {
            moves: vec![
                Move {
                    item: ItemId(10),
                    track,
                    start: Tick(0),
                },
                Move {
                    item: ItemId(11),
                    track,
                    start: Tick(10),
                },
            ],
        }],
        ripple: true,
    })
    .unwrap();
    assert_eq!(
        ripple_layout(&s)
            .iter()
            .find(|row| row.1 == ItemId(12))
            .unwrap()
            .2,
        Tick(20)
    );
}
#[test]
fn ripple_json_entries_use_one_diff_and_reject_hostile_edits_atomically() {
    for (operation, expected) in [
        (0, 20),
        (1, 20),
        (2, 40),
        (3, 60),
        (4, 40),
        (5, 50),
        (6, 50),
        (7, 50),
    ] {
        let (mut s, track, _) = ripple_fixture(&[40, 40], 20);
        let command = match operation {
            0 => Command::Delete {
                items: vec![ItemId(10)],
            },
            1 => Command::RippleDelete { item: ItemId(10) },
            2 => Command::TrimEnd {
                item: ItemId(10),
                at: Tick(40),
            },
            3 => Command::TrimStart {
                item: ItemId(10),
                at: Tick(40),
            },
            4 => Command::TimelineTrim {
                items: vec![ItemId(10)],
                edge: TrimEdge::End,
                delta: Tick(-20),
            },
            5 => Command::MoveItem {
                item: ItemId(10),
                track,
                start: Tick(10),
            },
            6 => Command::MoveItems {
                moves: vec![Move {
                    item: ItemId(10),
                    track,
                    start: Tick(10),
                }],
            },
            _ => Command::TimelineMoveItems {
                items: vec![ItemId(10)],
                delta: Tick(-10),
                placement: None,
            },
        };
        let before = project::encode(s.project(), false).unwrap();
        s.apply(Command::TimelineEdit {
            commands: vec![command],
            ripple: true,
        })
        .unwrap();
        assert_eq!(
            ripple_layout(&s)
                .iter()
                .find(|row| row.1 == ItemId(11))
                .unwrap()
                .2,
            Tick(expected),
            "operation {operation}"
        );
        s.apply(Command::Undo {}).unwrap();
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
        assert!(
            s.apply(Command::TimelineEdit {
                commands: vec![Command::MoveItem {
                    item: ItemId(10),
                    track,
                    start: Tick(i64::MAX)
                }],
                ripple: true
            })
            .is_err()
        );
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
    }
    let (mut s, _, _) = ripple_fixture(&[], 0);
    assert!(
        !s.apply(Command::TimelineEdit {
            commands: vec![Command::Delete { items: vec![] }],
            ripple: true
        })
        .unwrap()
        .changed
    );
    let (mut s, _, _) = ripple_fixture(&[1], 0);
    s.apply(Command::TimelineEdit {
        commands: vec![Command::Delete {
            items: vec![ItemId(10)],
        }],
        ripple: true,
    })
    .unwrap();
    assert!(!ripple_layout(&s).iter().any(|row| row.1 == ItemId(10)));
}
#[test]
fn ripple_wire_policy_is_atomic_explicit_and_cannot_nest() {
    for (default, explicit) in [(false, true), (true, false)] {
        let (mut s, _, _) = ripple_fixture(&[40, 40], 20);
        s.apply(Command::TimelineSetRipple { enabled: default })
            .unwrap();
        s.apply(Command::TimelineEdit {
            commands: vec![Command::Delete {
                items: vec![ItemId(10)],
            }],
            ripple: explicit,
        })
        .unwrap();
        assert_eq!(
            ripple_layout(&s)
                .iter()
                .find(|row| row.1 == ItemId(11))
                .unwrap()
                .2,
            Tick(if explicit { 20 } else { 60 })
        );
        s.apply(Command::Undo {}).unwrap();
        let before = project::encode(s.project(), false).unwrap();
        let history = s.history_len();
        for params in [
            json!({"commands":[]}),
            json!({"commands":[],"ripple":"yes"}),
            json!({"commands":[{"command":"undo","params":{}}],"ripple":true}),
        ] {
            assert!(crate::wire::run(&mut s, "timeline.edit", params).is_err());
            assert_eq!(project::encode(s.project(), false).unwrap(), before);
            assert_eq!(s.history_len(), history);
        }
        assert!(matches!(
            s.apply(Command::TimelineEdit {
                commands: vec![
                    Command::Delete { items: vec![] };
                    engine::control::MAX_BATCH_COMMANDS + 1
                ],
                ripple: true,
            }),
            Err(EditError::Control(engine::control::ControlError::Limit))
        ));
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
        for command in ["batch", "timeline.edit"] {
            let mut params = json!({"commands":[{"command":"delete","params":{"items":[10]}},
                {"command":"timeline.edit","params":{"commands":[],"ripple":true}}]});
            if command == "timeline.edit" {
                params["ripple"] = json!(true);
            }
            assert!(crate::wire::run(&mut s, command, params).is_err());
            assert_eq!(project::encode(s.project(), false).unwrap(), before);
            assert_eq!(s.history_len(), history);
        }
    }
}
/// Main track 20 @0, 21 @40, 22 @80 (each 40 long) beside `project()`'s overlay items 10 @0
/// and 11 @60, with the main-track magnet set as given.
fn main_magnet_fixture(magnet: bool) -> Session {
    let mut data = project().into_data();
    let template = data
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .clone();
    data.scene_mut().timeline.main.items = [20, 21, 22]
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let index = index as i64;
            let mut item = template.clone();
            item.id = ItemId(id);
            item.start = Tick(index * 40);
            item.source_range = range(100 + index * 100, 140 + index * 100);
            item
        })
        .collect();
    data.scene_mut().timeline.magnet = magnet;
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
fn main_starts(s: &Session) -> Vec<i64> {
    s.project()
        .data()
        .scene()
        .timeline
        .main
        .items
        .iter()
        .map(|item| item.start.0)
        .collect()
}
fn overlay_starts(s: &Session) -> Vec<(u64, i64)> {
    s.project()
        .data()
        .scene()
        .timeline
        .overlays
        .iter()
        .flat_map(|track| &track.items)
        .map(|item| (item.id.0, item.start.0))
        .collect()
}
fn ids(items: &[u64]) -> Vec<ItemId> {
    items.iter().copied().map(ItemId).collect()
}
#[test]
fn main_magnet_closes_only_the_span_that_removals_vacate_in_one_undo_step() {
    let delete = |items: &[u64]| Command::Delete { items: ids(items) };
    let split_left = |items: &[u64]| Command::TimelineSplitLeft {
        items: ids(items),
        at: Tick(60),
    };
    let split_right = |items: &[u64]| Command::TimelineSplitRight {
        items: ids(items),
        at: Tick(60),
    };
    let both_overlays = vec![(10, 0), (11, 60)];
    // (name, magnet, command, expected main starts, expected overlay starts)
    let rows: Vec<(&str, bool, Command, Vec<i64>, Vec<(u64, i64)>)> = vec![
        (
            "delete middle, magnet on",
            true,
            delete(&[21]),
            vec![0, 40],
            both_overlays.clone(),
        ),
        (
            "delete middle, magnet off",
            false,
            delete(&[21]),
            vec![0, 80],
            both_overlays.clone(),
        ),
        (
            "split left mid-clip, magnet on",
            true,
            split_left(&[21]),
            vec![0, 40, 60],
            both_overlays.clone(),
        ),
        (
            "split left mid-clip, magnet off",
            false,
            split_left(&[21]),
            vec![0, 60, 80],
            both_overlays.clone(),
        ),
        (
            "split right mid-clip, magnet on",
            true,
            split_right(&[21]),
            vec![0, 40, 60],
            both_overlays.clone(),
        ),
        (
            "split right mid-clip, magnet off",
            false,
            split_right(&[21]),
            vec![0, 40, 80],
            both_overlays.clone(),
        ),
        (
            "split keeping both pieces, magnet on",
            true,
            Command::SplitAt {
                item: ItemId(21),
                at: Tick(60),
            },
            vec![0, 40, 60, 80],
            both_overlays.clone(),
        ),
        (
            "overlay-only delete, magnet on",
            true,
            delete(&[10]),
            vec![0, 40, 80],
            vec![(11, 60)],
        ),
        (
            "batch of two main deletes closes one span, magnet on",
            true,
            Command::Batch {
                commands: vec![delete(&[20]), delete(&[21])],
            },
            vec![0],
            both_overlays.clone(),
        ),
        (
            "main delete with a listed overlay partner, magnet on",
            true,
            delete(&[21, 10]),
            vec![0, 40],
            vec![(11, 20)],
        ),
        (
            "main delete with a listed overlay partner, magnet off",
            false,
            delete(&[21, 10]),
            vec![0, 80],
            vec![(11, 60)],
        ),
    ];
    for (name, magnet, command, main, overlays) in rows {
        let mut s = main_magnet_fixture(magnet);
        let before = s.project().clone();
        s.apply(command).unwrap();
        assert_eq!(main_starts(&s), main, "{name}: main track");
        assert_eq!(overlay_starts(&s), overlays, "{name}: overlay tracks");
        let after = s.project().clone();
        assert_eq!(s.history_len(), (1, 0), "{name}: one undo step");
        s.undo().unwrap();
        assert_eq!(s.project(), &before, "{name}: undo");
        s.redo().unwrap();
        assert_eq!(s.project(), &after, "{name}: redo");
    }
    // A refused removal publishes no gap-closing candidate.
    let mut s = main_magnet_fixture(true);
    let before = s.project().clone();
    assert!(s.apply(delete(&[21, 99])).is_err());
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), (0, 0));
}
