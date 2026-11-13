#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{content, get, new_item, project, range, session};
use engine::{Command, EditError, Session};
use project::{Color, Item, ItemId, Project, TrackId};
use proptest::prelude::*;
use serde_json::json;
use std::sync::Arc;
use time::Tick;

#[test]
fn batch_is_atomic_and_uses_one_step_with_eviction_and_branching() {
    let mut s = Session::new(project(), 2).unwrap();
    let before = s.project().clone();
    let h = s.history_len();
    assert!(
        s.apply(Command::Batch {
            commands: vec![
                Command::MoveItem {
                    item: ItemId(10),
                    track: TrackId(2),
                    start: Tick(0)
                },
                Command::Delete {
                    items: vec![ItemId(99)]
                },
            ]
        })
        .is_err()
    );
    assert!(Arc::ptr_eq(&before, s.project()));
    assert_eq!(s.history_len(), h);
    s.apply(Command::Batch {
        commands: vec![
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(0),
            },
            Command::MoveItem {
                item: ItemId(11),
                track: TrackId(2),
                start: Tick(60),
            },
        ],
    })
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));
    let batch = s.project().clone();
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&batch, s.project()));
    for n in [1, 2] {
        s.apply(Command::BookmarkAdd {
            start: Tick(n),
            duration: Tick(0),
            note: "".into(),
            color: Color([0; 4]),
        })
        .unwrap();
    }
    assert_eq!(s.history_len(), (2, 0));
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&batch, s.project()));
    assert!(s.undo().is_err());
    s.apply(Command::Delete {
        items: vec![ItemId(10)],
    })
    .unwrap();
    assert!(!s.can_redo());
    assert!(
        s.apply(Command::Batch {
            commands: vec![Command::Undo {}]
        })
        .is_err()
    );
}
#[test]
fn continuous_gestures_merge_and_ids_do_not_alias_after_undo() {
    let mut s = session();
    let before = s.project().clone();
    let drag = |s: &mut Session, starts: &[i64]| {
        s.begin_gesture().unwrap();
        for &start in starts {
            s.apply(Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(start),
            })
            .unwrap();
        }
        s.commit_gesture().unwrap();
    };
    drag(&mut s, &[100, 110, 120]);
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
    s.redo().unwrap();
    drag(&mut s, &[130]);
    assert_eq!(s.history_len(), (2, 0));
    let first = s
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(2) },
            item: new_item(500, 10),
        })
        .unwrap();
    s.undo().unwrap();
    let second = s
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(2) },
            item: new_item(500, 10),
        })
        .unwrap();
    assert_ne!(first.created, second.created);
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

#[test]
    fn random_edits_keep_invariants_undo_initial_and_redo_final(
        ops in prop::collection::vec((0u8..9,0u64..40,-100i64..400),0..150),
        rate in 0usize..time::FrameRate::COMMON.len(),
    ) {
        let mut data=project().into_data(); data.canvas.fps=time::FrameRate::COMMON[rate];
        let mut s=Session::new(Project::new(data).unwrap(),1000).unwrap();
        let initial=s.project().clone();
        for (op,id,tick) in ops {
            let cmd=match op {
                0=>Command::InsertItem {placement: engine::Placement::Track { track: TrackId(id%2 + 1) },item:new_item(tick,10)},
                1=>Command::MoveItem {item:ItemId(id),track:TrackId(id%2 + 1),start:Tick(tick)},
                2=>Command::SplitAt {item:ItemId(id),at:Tick(tick)},
                3=>Command::TrimStart {item:ItemId(id),at:Tick(tick)},
                4=>Command::TrimEnd {item:ItemId(id),at:Tick(tick)},
                5=>Command::RippleDelete {item:ItemId(id)},
                6=>Command::Delete {items:vec![ItemId(id)]},
                7=>Command::SetItemParam {item:ItemId(id),content:content()},
                _=>Command::Batch{commands:vec![Command::MoveItem{item:ItemId(id),track:TrackId(id%2 + 1),start:Tick(tick)},Command::Delete{items:vec![ItemId(id.wrapping_add(1))]}]},
            };
            let before=s.project().clone(); let history=s.history_len();
            if s.apply(cmd).is_err() { prop_assert!(Arc::ptr_eq(&before,s.project())); prop_assert_eq!(s.history_len(),history); }
            prop_assert!(Project::new(s.project().data().clone()).is_ok());
        }
        let final_state=s.project().clone(); let n=s.history_len().0;
        for _ in 0..n { s.undo().unwrap(); }
        prop_assert!(Arc::ptr_eq(&initial,s.project()));
        for _ in 0..n { s.redo().unwrap(); }
        prop_assert!(Arc::ptr_eq(&final_state,s.project()));
    }
}
#[test]
fn noops_and_refusals_inside_a_gesture_leave_one_step_and_preserve_redo() {
    let mut s = session();
    let drag = |start| Command::MoveItem {
        item: ItemId(10),
        track: TrackId(2),
        start: Tick(start),
    };
    s.begin_gesture().unwrap();
    s.apply(drag(0)).unwrap();
    assert!(!s.apply(Command::Delete { items: vec![] }).unwrap().changed);
    s.apply(drag(10)).unwrap();
    assert!(crate::wire::run(&mut s, "bad", json!({})).is_err());
    s.apply(drag(20)).unwrap();
    s.commit_gesture().unwrap();
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    let snapshot = s.project().clone();
    assert!(!s.apply(Command::Delete { items: vec![] }).unwrap().changed);
    assert!(s.can_redo());
    assert!(Arc::ptr_eq(&snapshot, s.project()));
    assert!(
        s.apply(Command::SplitAt {
            item: ItemId(10),
            at: Tick(i64::MIN)
        })
        .is_err()
    );
    assert!(s.can_redo());
    s.redo().unwrap();
    assert_eq!(get(&s, 10).start, Tick(20));
}
#[test]
#[ignore = "manual metadata-edit benchmark; no wall-clock assertion"]
fn measure_large_project_gesture() {
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.items = (0..10000)
        .map(|n| Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(n + 10),
            source_asset: None,
            source_range: range(0, 10),
            start: Tick(n as i64 * 20),
            content: content(),
        })
        .collect();
    let mut s = Session::new(Project::new(data).unwrap(), 4).unwrap();
    let initial = s.project().clone();
    // Each run measures a whole public gesture, including candidate copy and validation.
    for run in 0..3 {
        let start = std::time::Instant::now();
        s.begin_gesture().unwrap();
        for n in 0..100 {
            s.apply(Command::MoveItem {
                item: ItemId(10009),
                track: TrackId(1),
                start: Tick(199_980 + (n % 2)),
            })
            .unwrap();
        }
        s.commit_gesture().unwrap();
        let elapsed = start.elapsed();
        assert_eq!(s.history_len(), (1, 0));
        assert!(Project::new(s.project().data().clone()).is_ok());
        assert_eq!(get(&s, 10009).source_range, range(0, 10));
        assert_eq!(get(&s, 10009).start, Tick(199_981));
        s.undo().unwrap();
        assert!(Arc::ptr_eq(&initial, s.project()));
        println!(
            "run={run} edits=100 items=10000 elapsed_us={} mean_us={}",
            elapsed.as_micros(),
            elapsed.as_micros() / 100
        );
    }
}
#[test]
fn explicit_gesture_commits_once_and_cancel_restores_history_redo_and_events() {
    let mut s = Session::new(project(), 1).unwrap();
    s.apply(Command::MoveItem {
        item: ItemId(10),
        track: TrackId(2),
        start: Tick(0),
    })
    .unwrap();
    let committed = s.project().clone();
    s.apply(Command::BeginGesture {}).unwrap();
    assert!(s.gesture_active());
    assert!(!s.can_undo());
    assert!(matches!(
        s.begin_gesture(),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    assert!(matches!(
        s.undo(),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    for start in 1..10 {
        s.apply(Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(start),
        })
        .unwrap();
    }
    assert_eq!(s.history_len(), (1, 0));
    assert!(s.take_change().is_none());
    assert!(s.cancel_gesture().unwrap().changed);
    assert!(Arc::ptr_eq(&committed, s.project()));
    assert!(Arc::ptr_eq(&committed, &s.take_change().unwrap()));
    assert!(s.take_change().is_none());
    s.undo().unwrap();
    let redo_base = s.project().clone();
    s.begin_gesture().unwrap();
    s.apply(Command::Delete {
        items: vec![ItemId(11)],
    })
    .unwrap();
    s.apply(Command::CancelGesture {}).unwrap();
    assert!(Arc::ptr_eq(&redo_base, s.project()));
    assert!(s.can_redo());
    s.redo().unwrap();
    s.take_change();
    s.begin_gesture().unwrap();
    for start in 1..100 {
        s.apply(Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(start),
        })
        .unwrap();
    }
    assert_eq!(s.history_len(), (1, 0));
    assert!(s.take_change().is_none());
    let final_preview = s.project().clone();
    s.apply(Command::CommitGesture {}).unwrap();
    assert_eq!(s.history_len(), (1, 0));
    assert!(Arc::ptr_eq(&s.take_change().unwrap(), &final_preview));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&committed, s.project()));
    assert!(matches!(
        s.cancel_gesture(),
        Err(EditError::Session(engine::SessionError::NoGesture))
    ));
    assert!(matches!(
        s.commit_gesture(),
        Err(EditError::Session(engine::SessionError::NoGesture))
    ));
}
#[test]
fn failed_batch_at_every_position_preserves_redo_and_pending_event() {
    let pending = || {
        let mut s = Session::new(project(), 1).unwrap();
        s.apply(Command::Delete {
            items: vec![ItemId(11)],
        })
        .unwrap();
        s.undo().unwrap();
        s
    };
    let bookmark = || Command::BookmarkAdd {
        start: Tick(0),
        duration: Tick(0),
        note: "".into(),
        color: Color([0; 4]),
    };
    let mut reference = pending();
    reference.redo().unwrap();
    let expected_ids = reference.apply(bookmark()).unwrap().created;
    for (failure, message) in [
        (
            Command::Delete {
                items: vec![ItemId(99)],
            },
            None,
        ),
        (
            Command::Inspect {},
            Some("export commands require job submission"),
        ),
        (Command::Undo {}, Some("session control inside batch")),
        (
            Command::Snapshot {
                time: Tick(0),
                path: "unsubmitted-snapshot.png".into(),
            },
            Some("export commands require job submission"),
        ),
        (
            Command::Preview { frames: vec![0] },
            Some("export commands require job submission"),
        ),
        (
            Command::TimelineEdit {
                commands: vec![],
                ripple: true,
            },
            Some("nested ripple policy"),
        ),
    ] {
        for position in 0..5 {
            let mut s = pending();
            let before = s.project().clone();
            let history = s.history_len();
            let mut commands = vec![bookmark(); 4];
            commands.insert(position, failure.clone());
            let result = s.apply(Command::Batch { commands });
            if let Some(expected) = message {
                assert!(matches!(result, Err(EditError::Invalid(actual)) if actual == expected));
            } else {
                assert!(matches!(result, Err(EditError::NoItem(ItemId(99)))));
            }
            assert!(Arc::ptr_eq(&before, s.project()));
            assert_eq!(s.history_len(), history);
            assert!(Arc::ptr_eq(&before, &s.take_change().unwrap()));
            assert!(s.can_redo());
            s.redo().unwrap();
            assert_eq!(s.apply(bookmark()).unwrap().created, expected_ids);
            assert_eq!(s.project(), reference.project());
        }
    }
}
#[test]
fn grouped_failure_and_unwind_restore_every_staged_change_without_eviction() {
    for unwind in [false, true] {
        let mut s = Session::new(project(), 1).unwrap();
        s.apply(Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(0),
        })
        .unwrap();
        s.apply(Command::Delete {
            items: vec![ItemId(11)],
        })
        .unwrap();
        s.undo().unwrap();
        let before = s.project().clone();
        let history = s.history_len();
        let result: Result<(), engine::EditError> =
            s.transaction(|group| -> Result<(), EditError> {
                assert!(matches!(
                    group.transaction(|_| Ok::<(), EditError>(())),
                    Err(EditError::Session(engine::SessionError::TransactionActive))
                ));
                assert!(matches!(
                    group.begin_gesture(),
                    Err(EditError::Session(engine::SessionError::TransactionActive))
                ));
                assert!(matches!(
                    group.redo(),
                    Err(EditError::Session(engine::SessionError::TransactionActive))
                ));
                for _ in 0..10 {
                    group.apply(Command::BookmarkAdd {
                        start: Tick(0),
                        duration: Tick(0),
                        note: "".into(),
                        color: Color([0; 4]),
                    })?;
                    assert_eq!(group.history_len(), history);
                    assert!(group.take_change().is_none());
                }
                if unwind {
                    panic!("injected staged-group unwind");
                }
                Err(EditError::Invalid("injected group failure"))
            });
        if unwind {
            assert!(matches!(
                result,
                Err(EditError::Session(engine::SessionError::Panicked))
            ));
        } else {
            assert!(matches!(result, Err(EditError::Invalid(_))));
        }
        assert!(!s.transaction_active());
        assert!(Arc::ptr_eq(&before, s.project()));
        assert_eq!(s.history_len(), history);
        assert!(s.can_redo());
        assert!(Arc::ptr_eq(&before, &s.take_change().unwrap()));
        s.redo().unwrap();
    }
    let mut s = Session::new(project(), 1).unwrap();
    let before = s.project().clone();
    let history = s.history_len();
    let cause: Arc<dyn engine::Failure> =
        Arc::new(engine::control::Error::Ai(ai::Error::RateLimit {
            retry_after_seconds: Some(3),
        }));
    let expected = cause.clone();
    let result: engine::control::Result<()> = s.transaction(move |group| {
        group.apply(Command::RenameProject {
            name: "discarded".into(),
        })?;
        Err(engine::control::Error::Runtime(
            engine::RuntimeError::Failed(cause),
        ))
    });
    let engine::control::Error::Runtime(engine::RuntimeError::Failed(returned)) =
        result.unwrap_err()
    else {
        panic!("mixed callback cause changed")
    };
    assert!(Arc::ptr_eq(&returned, &expected));
    assert!(Arc::ptr_eq(s.project(), &before));
    assert_eq!(s.history_len(), history);
    assert!(!s.transaction_active());
    assert!(s.take_change().is_none());
}
#[test]
fn groups_evict_only_one_step_and_noop_gestures_preserve_redo() {
    let mut s = Session::new(project(), 1).unwrap();
    let initial = s.project().clone();
    s.apply(Command::Delete {
        items: vec![ItemId(11)],
    })
    .unwrap();
    let before = s.project().clone();
    s.take_change();
    s.transaction(|group| -> Result<(), EditError> {
        for start in 0..10 {
            group.apply(Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(start),
            })?;
            assert_eq!(group.history_len(), (1, 0));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));
    assert!(s.take_change().is_some());
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
    assert!(s.undo().is_err());
    s.take_change();
    s.begin_gesture().unwrap();
    assert!(!s.commit_gesture().unwrap().changed);
    assert!(s.take_change().is_none());
    assert!(s.can_redo());
    s.transaction(|_| Ok::<(), EditError>(())).unwrap();
    assert!(s.can_redo());
    assert!(s.take_change().is_none());
    assert!(!Arc::ptr_eq(&initial, s.project()));
}
