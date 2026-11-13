#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{content, get, new_item, project, range, session};
use engine::{Command, EditError, Session};
use project::{Color, ItemId, Project, TrackId};
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
        .map(|n| {
            testkit::item::item(ItemId(n + 10), content())
                .source(range(0, 10))
                .start(Tick(n as i64 * 20))
                .build()
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

fn bookmark() -> Command {
    Command::BookmarkAdd {
        start: Tick(1),
        duration: Tick(0),
        note: "".into(),
        color: Color([0; 4]),
    }
}

#[test]
fn a_fork_applies_back_as_one_undo_step_and_only_to_the_revision_it_was_taken_at() {
    use crate::support::control::{Allow, caller, runner};
    let mut r = runner();
    let mut live = Session::new(project(), 4).unwrap();
    let pristine = live.project().clone();
    let mut fork = live.fork().unwrap();
    for command in [
        Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(200),
        },
        bookmark(),
        Command::Delete {
            items: vec![ItemId(11)],
        },
    ] {
        fork.control_command(command, caller(&Allow), &mut r)
            .unwrap();
    }
    // A refused command, and the session controls a fork never runs, leave no log entry.
    for refused in [
        Command::Delete {
            items: vec![ItemId(99)],
        },
        Command::Undo {},
        Command::TimelineSetRipple { enabled: true },
    ] {
        assert!(
            fork.control_command(refused, caller(&Allow), &mut r)
                .is_err()
        );
    }
    assert_eq!(fork.commands().count(), 3);
    assert!(Arc::ptr_eq(&pristine, live.project()));
    assert_eq!(live.history_len(), (0, 0));
    assert!(live.take_change().is_none());

    let summary = fork.summary();
    assert_eq!(summary.moved, vec![ItemId(10)]);
    assert_eq!(summary.removed, vec![ItemId(11)]);
    assert!(summary.added.is_empty() && summary.trimmed.is_empty() && summary.retimed.is_empty());
    assert!(summary.params_changed.is_empty() && summary.captions_changed.is_empty());
    assert_eq!(
        (summary.duration_before, summary.duration_after),
        (Tick(100), Tick(240))
    );

    let forked = fork.session().project().clone();
    let outcome = live.apply_fork(fork).unwrap();
    assert!(outcome.changed);
    assert_eq!(outcome.created.len(), 1);
    assert_eq!(live.project(), &forked);
    assert_eq!(live.history_len(), (1, 0));
    assert!(live.take_change().is_some());
    assert!(live.take_change().is_none());
    live.undo().unwrap();
    assert!(Arc::ptr_eq(&pristine, live.project()));
}

#[test]
fn a_fork_cannot_outlive_its_session_identity_in_the_revision_and_never_forks_in_a_group() {
    use crate::support::control::{Allow, caller, runner};
    fn forked(live: &Session, r: &mut engine::jobs::JobRunner) -> engine::Fork {
        let mut fork = live.fork().unwrap();
        fork.control_command(bookmark(), caller(&Allow), r).unwrap();
        fork
    }
    let mut r = runner();
    let mut live = Session::new(project(), 4).unwrap();
    // Rows: a live edit since the fork, and a successor session of the same project, which
    // is what reopening a deleted and restored project creates.
    let moved_on = forked(&live, &mut r);
    live.apply(bookmark()).unwrap();
    let successor = forked(&live, &mut r);
    let mut reopened = Session::new(live.project().as_ref().clone(), 4).unwrap();
    for (target, fork) in [(&mut live, moved_on), (&mut reopened, successor)] {
        let project = target.project().clone();
        let (revision, history) = (target.revision(), target.history_len());
        assert!(matches!(
            target.apply_fork(fork),
            Err(EditError::Control(
                engine::control::ControlError::StaleRevision
            ))
        ));
        assert!(Arc::ptr_eq(&project, target.project()));
        assert_eq!(
            (target.revision(), target.history_len()),
            (revision, history)
        );
    }

    // An empty log is a no-op: no undo entry, no change event, no new revision.
    live.take_change();
    let (revision, history) = (live.revision(), live.history_len());
    let empty = live.fork().unwrap();
    let outcome = live.apply_fork(empty).unwrap();
    assert!(!outcome.changed);
    assert_eq!(live.revision(), revision);
    assert_eq!(live.history_len(), history);
    assert!(live.take_change().is_none());

    // No fork while a gesture or a group holds an uncommitted project.
    live.begin_gesture().unwrap();
    assert!(live.fork().is_err());
    live.cancel_gesture().unwrap();
    assert!(
        live.transaction(|s| Ok::<_, EditError>(s.fork().is_err()))
            .unwrap()
    );
}

#[test]
fn a_failing_fork_transaction_drops_the_children_it_logged_and_a_good_one_applies_whole() {
    let mut live = Session::new(project(), 4).unwrap();
    let mut fork = live.fork().unwrap();
    let failed = fork.transaction(|s| {
        s.apply(bookmark())?;
        s.apply(Command::Delete {
            items: vec![ItemId(99)],
        })?;
        Ok::<_, EditError>(())
    });
    assert!(failed.is_err());
    assert!(fork.is_empty());
    assert_eq!(fork.commands().count(), 0);
    assert_eq!(fork.session().project(), fork.base_project());

    fork.transaction(|s| {
        s.apply(bookmark())?;
        s.apply(Command::BookmarkAdd {
            start: Tick(2),
            duration: Tick(0),
            note: "".into(),
            color: Color([0; 4]),
        })?;
        Ok::<_, EditError>(())
    })
    .unwrap();
    assert_eq!(fork.commands().count(), 2);
    let forked = fork.session().project().clone();
    live.apply_fork(fork).unwrap();
    assert_eq!(live.project(), &forked);
    assert_eq!(live.history_len(), (1, 0));
}

/// A change-summary row: its name, the commands, and the expected ids per category.
type SummaryRow<'a> = (&'a str, Vec<Command>, [Vec<u64>; 6]);

#[test]
fn change_summary_rows_fill_one_category_each_and_an_item_can_be_in_two_lists() {
    use crate::support::control::seed;
    let half = Tick(time::TICKS_PER_SECOND / 2);
    let tenth = Tick(time::TICKS_PER_SECOND / 10);
    let trim = |item: u64| Command::TrimEnd {
        item: ItemId(item),
        at: half,
    };
    // Added, moved, trimmed, retimed, parameters, captions.
    let rows: Vec<SummaryRow<'_>> = vec![
        (
            "trim",
            vec![trim(1)],
            [vec![], vec![], vec![1], vec![], vec![], vec![]],
        ),
        (
            "retime",
            vec![Command::SetSpeed {
                items: vec![ItemId(5)],
                speed: time::Speed::from_f64(2.0).unwrap(),
                maintain_pitch: true,
            }],
            [vec![], vec![], vec![], vec![5], vec![], vec![]],
        ),
        (
            "parameters",
            vec![Command::SetItemField {
                item: ItemId(5),
                patch: engine::ItemParamPatch::Opacity(0.5),
            }],
            [vec![], vec![], vec![], vec![], vec![5], vec![]],
        ),
        (
            "captions and trim",
            vec![trim(4)],
            [vec![], vec![], vec![4], vec![], vec![], vec![4]],
        ),
        (
            "moved, trimmed and captions",
            vec![
                trim(4),
                Command::MoveItem {
                    item: ItemId(4),
                    track: TrackId(4),
                    start: tenth,
                },
            ],
            [vec![], vec![4], vec![4], vec![], vec![], vec![4]],
        ),
    ];
    let ids = |list: &[ItemId]| list.iter().map(|id| id.0).collect::<Vec<_>>();
    for (name, commands, [added, moved, trimmed, retimed, params, captions]) in rows {
        let mut s = seed();
        let before = s.project().clone();
        s.apply(Command::Batch { commands }).unwrap();
        let got = engine::ChangeSummary::between(&before, s.project());
        assert_eq!(ids(&got.added), added, "{name}");
        assert_eq!(ids(&got.moved), moved, "{name}");
        assert_eq!(ids(&got.trimmed), trimmed, "{name}");
        assert_eq!(ids(&got.retimed), retimed, "{name}");
        assert_eq!(ids(&got.params_changed), params, "{name}");
        assert_eq!(ids(&got.captions_changed), captions, "{name}");
        assert!(got.removed.is_empty(), "{name}");
        assert!(!got.is_empty(), "{name}");
    }
    let mut s = seed();
    let before = s.project().clone();
    let created = s
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(3) },
            item: new_item(0, 10),
        })
        .unwrap()
        .placements
        .first()
        .unwrap()
        .item;
    let got = engine::ChangeSummary::between(&before, s.project());
    assert_eq!(got.added, vec![created]);
    assert!(got.moved.is_empty() && got.trimmed.is_empty() && got.params_changed.is_empty());
    assert!(engine::ChangeSummary::between(&before, &before).is_empty());
}
