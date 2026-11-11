#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{get, project, session};
use engine::{Command, EditError, ItemParamPatch, Placement, Session, TrimEdge};
use project::{ItemId, ItemParams, Project, TrackId, TrackKind};
use proptest::prelude::*;
use serde_json::json;
use std::sync::Arc;
use time::Tick;

#[test]
fn viewport_transform_text_preview_is_atomic_reversible_and_persistent() {
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = TrackKind::Text;
    for item in &mut track.items {
        item.content = ItemParams::Text(project::TextParams::new("Keep"));
    }
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let initial = project::encode(s.project(), false).unwrap();
    let viewport = render::viewport::Viewport::new(
        render::Rect {
            x: 0.,
            y: 0.,
            width: 1920.,
            height: 1080.,
        },
        [1920, 1080],
        1.,
        [0., 0.],
    )
    .unwrap();
    let renderer = render::Renderer::new(
        s.project(),
        render::RenderSettings::from_project(s.project()),
    )
    .unwrap();
    let selected = renderer
        .hit_test(&viewport, [960., 913.], Tick::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(selected, ItemId(10));
    s.set_selection([selected].into()).unwrap();
    let preview = |x: f64, text: &str| Command::TimelinePreview {
        commands: vec![
            Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::PositionX(x),
            },
            Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::PositionY(-25.0),
            },
            Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::ScaleX(0.5),
            },
            Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::ScaleY(0.75),
            },
            Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::Rotate(35.0),
            },
            Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::TextContent(text.into()),
            },
        ],
    };
    for x in [10., 40., -20.] {
        s.apply(preview(x, "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t"))
            .unwrap();
    }
    assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    assert_eq!(s.history_len(), (0, 0));
    let candidate = s.preview_project().unwrap().clone();
    for invalid in [
        Command::TimelinePreview {
            commands: vec![Command::SetItemField {
                item: ItemId(999),
                patch: ItemParamPatch::PositionX(2.),
            }],
        },
        Command::TimelinePreview {
            commands: vec![Command::SetItemField {
                item: ItemId(10),
                patch: ItemParamPatch::ScaleX(0.),
            }],
        },
        Command::TimelinePreview {
            commands: vec![
                Command::SetItemField {
                    item: ItemId(10),
                    patch: ItemParamPatch::TextContent("tentative".into()),
                },
                Command::Undo {},
            ],
        },
    ] {
        assert!(s.apply(invalid).is_err());
        assert_eq!(s.preview_project(), Some(&candidate));
        assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    }
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    assert_eq!(s.selection(), &[selected].into());
    assert_eq!(s.history_len(), (0, 0));
    // Reversing every edit to its original value produces no history entry.
    s.apply(preview(20., "draft")).unwrap();
    s.apply(Command::TimelinePreview { commands: vec![] })
        .unwrap();
    assert!(!s.apply(Command::TimelineCommitPreview {}).unwrap().changed);
    assert_eq!(s.history_len(), (0, 0));
    s.apply(preview(-20., "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t"))
        .unwrap();
    assert!(s.apply(Command::TimelineCommitPreview {}).unwrap().changed);
    assert_eq!(s.history_len(), (1, 0));
    assert!(s.preview_project().is_none());
    let edited = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&edited).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
    let edited_item = get(&s, 10);
    for (field, expected) in project::TransformField::ALL
        .into_iter()
        .zip([-20., -25., 0.5, 0.75, 35.])
    {
        assert_eq!(
            field.value_at(&edited_item.animations, Tick::ZERO).unwrap(),
            expected
        );
    }
    assert!(matches!(&edited_item.content,ItemParams::Text(p) if p.content.as_str()=="Tiếng Việt"));
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    assert!(!s.can_undo());
    s.redo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), edited);
    let mut data = loaded.into_data();
    data.scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .locked = true;
    let mut locked = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let before = locked.project().clone();
    assert!(locked.apply(preview(25., "refused")).is_err());
    assert_eq!(locked.project(), &before);
    assert!(locked.preview_project().is_none());
    assert_eq!(locked.history_len(), (0, 0));
}
#[test]
fn timeline_preview_isolated_replacement_commit_discard_and_history() {
    let mut s = session();
    let bytes = project::encode(s.project(), false).unwrap();
    let preview = |delta| Command::TimelinePreview {
        commands: vec![Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(11)],
            delta: Tick(delta),
            placement: Some(Placement::Track { track: TrackId(2) }),
        }],
    };
    s.apply(preview(20)).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(s.history_len(), (0, 0));
    assert!(s.take_change().is_none());
    s.apply(preview(5)).unwrap();
    let candidate = s.preview_project().unwrap().clone();
    assert_eq!(
        candidate
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == ItemId(10))
            .unwrap()
            .start,
        Tick(5)
    );
    assert!(s.apply(preview(-1)).is_err());
    assert_eq!(s.preview_project(), Some(&candidate));
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(s.history_len(), (0, 0));
    assert!(s.preview_project().is_none());
    assert!(s.take_change().is_none());
    s.apply(preview(5)).unwrap();
    s.apply(Command::TimelineCommitPreview {}).unwrap();
    assert_eq!(s.history_len(), (1, 0));
    assert_eq!(s.project(), &candidate);
    let committed = project::encode(s.project(), false).unwrap();
    assert_eq!(project::decode(&committed).unwrap().project, **s.project());
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    s.redo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), committed);
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == ItemId(10)))
            .unwrap()
            .id,
        TrackId(2)
    );
}
#[test]
fn timeline_groups_refuse_hostile_inputs_atomically() {
    let mut s = session();
    let before = s.project().clone();
    for command in [
        Command::TimelineMoveItems {
            items: vec![],
            delta: Tick(1),
            placement: None,
        },
        Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(10)],
            delta: Tick(1),
            placement: None,
        },
        Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(999)],
            delta: Tick(1),
            placement: None,
        },
        Command::TimelineMoveItems {
            items: vec![ItemId(10)],
            delta: Tick(-1),
            placement: None,
        },
        Command::TimelineMoveItems {
            items: vec![ItemId(10)],
            delta: Tick(-1),
            placement: Some(Placement::Above { track: TrackId(0) }),
        },
        Command::TimelineMoveItems {
            items: vec![ItemId(10)],
            delta: Tick(30),
            placement: None,
        },
        Command::TimelineMoveItems {
            items: vec![ItemId(10)],
            delta: Tick(1),
            placement: Some(Placement::Track {
                track: TrackId(999),
            }),
        },
        Command::TimelineTrim {
            items: vec![],
            edge: TrimEdge::End,
            delta: Tick(1),
        },
        Command::TimelineTrim {
            items: vec![ItemId(10), ItemId(10)],
            edge: TrimEdge::End,
            delta: Tick(1),
        },
        Command::TimelineTrim {
            items: vec![ItemId(10), ItemId(999)],
            edge: TrimEdge::End,
            delta: Tick(-1),
        },
        Command::TimelineTrim {
            items: vec![ItemId(10), ItemId(11)],
            edge: TrimEdge::Start,
            delta: Tick(40),
        },
        Command::TimelineTrim {
            items: vec![ItemId(10), ItemId(11)],
            edge: TrimEdge::End,
            delta: Tick(21),
        },
        Command::TimelinePreview {
            commands: vec![Command::Undo {}],
        },
        Command::TimelinePreview {
            commands: vec![Command::TimelineCommitPreview {}],
        },
    ] {
        assert!(s.apply(command.clone()).is_err(), "{command:?}");
        assert!(Arc::ptr_eq(s.project(), &before));
        assert_eq!(s.history_len(), (0, 0));
        assert!(!s.gesture_active());
    }
    assert!(
        crate::wire::run(
            &mut s,
            "timeline.trim",
            json!({"items":[10],"edge":"invalid","delta":1})
        )
        .is_err()
    );
    assert!(s.apply(Command::TimelineCommitPreview {}).is_err());
    assert!(s.apply(Command::TimelineDiscardPreview {}).is_err());
    assert!(matches!(
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(10); 100_001],
            delta: Tick(0),
            placement: None
        }),
        Err(EditError::Control(engine::control::ControlError::Limit))
    ));
    assert!(
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(11)],
            delta: Tick(i64::MAX),
            placement: None
        })
        .is_err()
    );
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    assert!(
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(10)],
            delta: Tick(1),
            placement: None
        })
        .is_err()
    );
    assert!(
        s.apply(Command::TimelineTrim {
            items: vec![ItemId(10)],
            edge: TrimEdge::End,
            delta: Tick(-1)
        })
        .is_err()
    );
    assert_eq!(s.project(), &locked);
}
#[test]
fn timeline_preview_reversal_preserves_redo_and_retires_created_tracks() {
    let mut s = session();
    s.apply(Command::TimelineMoveItems {
        items: vec![ItemId(10), ItemId(11)],
        delta: Tick(2),
        placement: None,
    })
    .unwrap();
    s.undo().unwrap();
    let original = s.project().clone();
    let history = s.history_len();
    s.take_change();
    let preview = |delta| Command::TimelinePreview {
        commands: vec![Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(11)],
            delta: Tick(delta),
            placement: None,
        }],
    };
    s.apply(preview(4)).unwrap();
    assert!(matches!(
        s.saved(),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    assert!(matches!(
        s.undo(),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    assert!(matches!(
        s.begin_gesture(),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    assert!(s.set_selection([ItemId(10)].into()).is_err());
    s.apply(preview(0)).unwrap();
    assert!(!s.apply(Command::TimelineCommitPreview {}).unwrap().changed);
    assert_eq!(s.project(), &original);
    assert_eq!(s.history_len(), history);
    assert!(s.can_redo());
    assert!(s.take_change().is_none());
    let new_track = Command::TimelinePreview {
        commands: vec![Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(11)],
            delta: Tick(1),
            placement: Some(Placement::Above { track: TrackId(0) }),
        }],
    };
    let retired = s.apply(new_track.clone()).unwrap().created;
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    let fresh = s.apply(new_track).unwrap().created;
    assert_ne!(retired, fresh);
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    assert_eq!(s.project(), &original);
    assert_eq!(s.history_len(), history);
    // An empty pointer gesture is promoted to a replacement preview and discards cleanly.
    s.begin_gesture().unwrap();
    s.apply(preview(1)).unwrap();
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    assert_eq!(s.project(), &original);
    // A gesture that already staged an edit refuses: a replacement would drop it.
    s.begin_gesture().unwrap();
    s.apply(Command::TimelineMoveItems {
        items: vec![ItemId(10), ItemId(11)],
        delta: Tick(1),
        placement: None,
    })
    .unwrap();
    assert!(s.apply(preview(1)).is_err());
    s.cancel_gesture().unwrap();
    assert_eq!(s.project(), &original);
}
#[test]
fn timeline_preview_wire_commit_validates_candidate_and_retains_failed_preview() {
    for commit in ["timeline.commit_preview", "commit_gesture"] {
        let mut s = session();
        let before = s.project().clone();
        s.apply(Command::TimelinePreview {
            commands: vec![Command::TimelineMoveItems {
                items: vec![ItemId(10), ItemId(11)],
                delta: Tick(1i64 << 53),
                placement: None,
            }],
        })
        .unwrap();
        let candidate = s.preview_project().unwrap().clone();
        assert!(crate::wire::run(&mut s, commit, json!({})).is_err());
        assert_eq!(s.project(), &before);
        assert_eq!(s.preview_project(), Some(&candidate));
        assert_eq!(s.history_len(), (0, 0));
        s.apply(Command::TimelineDiscardPreview {}).unwrap();
        assert_eq!(s.project(), &before);
    }
}
#[test]
fn timeline_group_new_track_and_main_origin_keep_common_delta() {
    for placement in [
        Placement::Above { track: TrackId(0) },
        Placement::Below { track: TrackId(1) },
    ] {
        let mut s = session();
        let before = s.project().clone();
        let outcome = s
            .apply(Command::TimelineMoveItems {
                items: vec![ItemId(10), ItemId(11)],
                delta: Tick(5),
                placement: Some(placement),
            })
            .unwrap();
        assert_eq!(outcome.created.len(), 1);
        assert_eq!(get(&s, 11).start.0 - get(&s, 10).start.0, 60);
        let candidate = s.project().clone();
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
        s.redo().unwrap();
        assert_eq!(s.project(), &candidate);
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(11)],
            delta: Tick(50),
            placement: Some(Placement::Track { track: TrackId(0) }),
        })
        .unwrap();
        assert_eq!(get(&s, 10).start, Tick(0));
        assert_eq!(get(&s, 11).start, Tick(60));
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

#[test]
    fn timeline_group_offsets_survive_moves_and_both_edge_resizes(
        delta in 0i64..1000, resize in 0i64..20, start_edge in any::<bool>()
    ) {
        let mut s = session();
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(10), ItemId(11)],
            delta: Tick(delta),
            placement: None,
        }).unwrap();
        prop_assert_eq!(get(&s,11).start.0-get(&s,10).start.0,60);
        s.apply(Command::TimelineTrim {
            items: vec![ItemId(10), ItemId(11)],
            edge: if start_edge { TrimEdge::Start } else { TrimEdge::End },
            delta: Tick(if start_edge { resize } else { -resize }),
        }).unwrap();
        prop_assert_eq!(get(&s,11).start.0-get(&s,10).start.0,60);
        prop_assert_eq!(get(&s,10).duration().unwrap(), Tick(40-resize));
        prop_assert_eq!(get(&s,11).duration().unwrap(), Tick(40-resize));
        prop_assert!(get(&s,10).end().unwrap() <= get(&s,11).start);
        prop_assert!(Project::new(s.project().data().clone()).is_ok());
    }
}
#[test]
fn rollback_must_invalidate_target_captured_in_discarded_preview() {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
        10,
    )
    .unwrap();
    let mut target = None;
    let result: engine::control::Result<()> = session.transaction(|s| {
        s.apply(engine::Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "".into(),
            kind: project::TrackKind::Video,
        })?;
        target = Some(s.job_target());
        Err(engine::control::Error::Runtime(
            engine::RuntimeError::Cancelled,
        ))
    });
    assert!(result.is_err());
    assert!(
        session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .is_empty()
    );
    assert!(
        matches!(
            session.apply_job_result(
                &target.unwrap(),
                engine::Command::AddTrack {
                    placement: engine::TrackPlacement::Auto,
                    name: "".into(),
                    kind: project::TrackKind::Video
                }
            ),
            Err(EditError::Session(engine::SessionError::StaleResult))
        ),
        "discarded preview target was accepted against rolled-back content"
    );
}
