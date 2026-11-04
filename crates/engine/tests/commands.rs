#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::{Command, CreatedId, EditError, Move, NewItem, Session, registry};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use proptest::prelude::*;
use serde_json::json;
use std::sync::Arc;
use time::{SourceRange, Tick};

fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(Tick(a), Tick(b)).unwrap()
}
fn content() -> ItemParams {
    ItemParams::Overlay(OverlayParams {
        color: Color([1, 2, 3, 255]),
    })
}
fn new_item(start: i64, duration: i64) -> NewItem {
    NewItem {
        animations: Default::default(),
        source_asset: None,
        source_range: range(100, 100 + duration),
        start: Tick(start),
        content: content(),
    }
}
fn project() -> Project {
    let mut data = Project::empty(ProjectId(0), "Tiếng Việt".into())
        .unwrap()
        .into_data();
    for id in [1, 0] {
        data.timeline.insert_track(Track {
            id: TrackId(id + 1),
            name: "track".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: Vec::new(),
        });
    }
    data.timeline.overlays.first_mut().unwrap().items = vec![
        Item {
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(10),
            source_asset: None,
            source_range: range(100, 140),
            start: Tick(0),
            content: content(),
        },
        Item {
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(11),
            source_asset: None,
            source_range: range(200, 240),
            start: Tick(60),
            content: content(),
        },
    ];
    Project::new(data).unwrap()
}
fn session() -> Session {
    Session::new(project(), 1000).unwrap()
}
fn get(s: &Session, id: u64) -> &Item {
    s.project()
        .data()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(id))
        .unwrap()
}
#[test]
fn all_commands_share_registry_and_strict_json_dispatch() {
    let mut s = Session::new(Project::empty(ProjectId(0), "".into()).unwrap(), 10).unwrap();
    let r = s
        .execute("track.add", json!({"name":"Tiếng", "kind":"video"}))
        .unwrap();
    assert_eq!(r.created, vec![CreatedId::Track(TrackId(1))]);
    assert_eq!(
        s.project()
            .data()
            .timeline
            .overlays
            .first()
            .unwrap()
            .name
            .as_str(),
        "Tiếng"
    );
    let r = s
        .execute("insert_item", json!({"track":1,"item":new_item(0,10)}))
        .unwrap();
    assert_eq!(r.created, vec![CreatedId::Item(ItemId(2))]);
    s.execute("move_item", json!({"item":2,"track":1,"start":10}))
        .unwrap();
    s.execute("set_item_param", json!({"item":2,"content":content()}))
        .unwrap();
    s.execute(
        "add_bookmark",
        json!({"start":1,"duration":0,"note":"ghi chú","color":[0,0,0,255]}),
    )
    .unwrap();
    let asset = SourceAsset::try_from(AssetData {
        id: AssetId(4),
        path: std::env::temp_dir().join("missing media.mp4"),
        content_hash: ContentHash([0; 32]),
        probe: SourceProbe {
            format: "mp4".into(),
            duration: Some(Tick(1000)),
            streams: vec![],
        },
    })
    .unwrap();
    s.execute("add_asset", json!({"asset":asset})).unwrap();
    s.execute("delete", json!({"items":[2]})).unwrap();
    s.execute(
        "track.set_flags",
        json!({"track":1,"muted":true,"hidden":true,"locked":true}),
    )
    .unwrap();
    assert!(matches!(
        s.apply(Command::RemoveTrack { track: TrackId(1) }),
        Err(EditError::Locked(_))
    ));
    s.execute(
        "track.set_flags",
        json!({"track":1,"muted":false,"hidden":false,"locked":false}),
    )
    .unwrap();
    s.execute("track.remove", json!({"track":1})).unwrap();
    for spec in registry() {
        assert!(!spec.label.is_empty());
        assert_eq!(registry().iter().filter(|r| r.id == spec.id).count(), 1);
        assert_eq!(spec.params_schema["type"], "object");
    }
    let undo = registry().iter().find(|r| r.id == "undo").unwrap();
    assert!(undo.enabled(&s));
    s.execute("undo", json!({})).unwrap();
    assert!(
        registry()
            .iter()
            .find(|r| r.id == "redo")
            .unwrap()
            .enabled(&s)
    );
    s.execute("redo", json!({})).unwrap();
}
#[test]
fn split_trim_ripple_and_history_use_exact_source_and_output_coordinates() {
    let mut s = session();
    let original = s.project().clone();
    let out = s
        .apply(Command::SplitAt {
            item: ItemId(10),
            at: Tick(20),
        })
        .unwrap();
    let CreatedId::Item(right) = *out.created.first().unwrap() else {
        panic!("item ID");
    };
    assert_eq!(get(&s, 10).source_range, range(100, 120));
    assert_eq!(get(&s, right.0).source_range, range(120, 140));
    assert_eq!(get(&s, right.0).start, Tick(20));
    s.apply(Command::TrimStart {
        item: right,
        at: Tick(25),
    })
    .unwrap();
    assert_eq!(get(&s, right.0).source_range, range(125, 140));
    s.apply(Command::TrimEnd {
        item: right,
        at: Tick(35),
    })
    .unwrap();
    assert_eq!(get(&s, right.0).source_range, range(125, 135));
    s.apply(Command::RippleDelete { item: ItemId(10) }).unwrap();
    assert_eq!(get(&s, right.0).start, Tick(5));
    assert_eq!(get(&s, 11).start, Tick(40));
    let final_state = s.project().clone();
    for _ in 0..4 {
        s.undo().unwrap();
    }
    assert!(Arc::ptr_eq(s.project(), &original));
    for _ in 0..4 {
        s.redo().unwrap();
    }
    assert!(Arc::ptr_eq(s.project(), &final_state));
}
#[test]
fn simultaneous_moves_can_swap_but_never_leave_overlap_or_touch_locked_tracks() {
    let mut s = session();
    s.apply(Command::MoveItems {
        moves: vec![
            Move {
                item: ItemId(10),
                track: TrackId(1),
                start: Tick(60),
            },
            Move {
                item: ItemId(11),
                track: TrackId(1),
                start: Tick(0),
            },
        ],
    })
    .unwrap();
    assert_eq!(get(&s, 10).start, Tick(60));
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::MoveItem {
            item: ItemId(10),
            track: TrackId(1),
            start: Tick(20)
        }),
        Err(EditError::Overlap(TrackId(1)))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    s.apply(Command::SetTrackFlags {
        track: TrackId(2),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    let before = s.project().clone();
    for command in [
        Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(0),
        },
        Command::InsertItem {
            track: TrackId(2),
            item: new_item(0, 10),
        },
        Command::Delete {
            items: vec![ItemId(10), ItemId(99)],
        },
        Command::MoveItems {
            moves: vec![
                Move {
                    item: ItemId(10),
                    track: TrackId(1),
                    start: Tick(60),
                },
                Move {
                    item: ItemId(10),
                    track: TrackId(1),
                    start: Tick(60),
                },
            ],
        },
    ] {
        assert!(s.apply(command).is_err());
        assert!(Arc::ptr_eq(&before, s.project()));
    }
}
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
        s.apply(Command::AddBookmark {
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
    for start in [100, 110, 120] {
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(start),
            },
            Some("drag-1"),
        )
        .unwrap();
    }
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
    s.redo().unwrap();
    s.end_merge();
    s.apply_merged(
        Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(130),
        },
        Some("drag-1"),
    )
    .unwrap();
    assert_eq!(s.history_len(), (2, 0));
    let first = s
        .apply(Command::InsertItem {
            track: TrackId(2),
            item: new_item(500, 10),
        })
        .unwrap();
    s.undo().unwrap();
    let second = s
        .apply(Command::InsertItem {
            track: TrackId(2),
            item: new_item(500, 10),
        })
        .unwrap();
    assert_ne!(first.created, second.created);
}
#[test]
fn hostile_parameters_and_limits_leave_project_and_history_intact() {
    let mut s = session();
    let before = s.project().clone();
    for (id, params) in [
        ("missing", json!({})),
        ("track.remove", json!({"track":0})),
        ("delete", json!({"items":[10,10]})),
        ("move_item", json!({"item":10,"track":1,"start":i64::MAX})),
        ("move_item", json!({"item":10,"track":1,"start":-1})),
        (
            "move_item",
            json!({"item":10,"track":1,"start":0,"typo":true}),
        ),
        ("trim_start", json!({"item":10,"at":i64::MIN})),
        ("trim_end", json!({"item":10,"at":0})),
        ("split_at", json!({"item":10,"at":40})),
        ("undo", json!({"unexpected":true})),
        ("insert_item", json!({"track":1,"item":null})),
    ] {
        assert!(s.execute(id, params).is_err(), "{id}");
        assert!(Arc::ptr_eq(&before, s.project()));
        assert_eq!(s.history_len(), (0, 0));
    }
    let mut nested = json!({});
    for _ in 0..40 {
        nested = json!([nested]);
    }
    assert!(matches!(s.execute("batch", nested), Err(EditError::Limit)));
    let command = Command::Batch {
        commands: vec![Command::Delete { items: vec![] }; 1025],
    };
    assert!(matches!(s.apply(command), Err(EditError::Limit)));
    assert!(Session::new(project(), 0).is_err());
    assert!(Session::new(project(), 1001).is_err());
    let mut data = project().into_data();
    data.timeline.overlays.first_mut().unwrap().id = TrackId(u64::MAX);
    let mut s = Session::new(Project::new(data).unwrap(), 1).unwrap();
    assert!(matches!(
        s.apply(Command::AddTrack {
            name: "".into(),
            kind: TrackKind::Video
        }),
        Err(EditError::IdExhausted)
    ));
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
                0=>Command::InsertItem {track:TrackId(id%2 + 1),item:new_item(tick,10)},
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
    #[test]
    fn arbitrary_hostile_json_never_changes_state_on_error(id in ".{0,20}", text in ".{0,100}", value in any::<i64>()) {
        let mut s=session(); let before=s.project().clone();
        let params=json!({"item":value,"start":text,"track":value});
        prop_assert!(s.execute(&id,params).is_err());
        prop_assert!(Arc::ptr_eq(&before,s.project()));
    }
}

#[test]
fn media_handles_and_typed_parameter_validation_are_atomic() {
    use media::{AudioInfo, ColorInfo, SourceStream, StreamKind, TimeBase, VideoInfo};
    use project::{AudioParams, VideoParams};
    let mut data = project().into_data();
    data.timeline.overlays.clear();
    for (id, kind) in [(0, TrackKind::Video), (1, TrackKind::Audio)] {
        data.timeline.insert_track(Track {
            id: TrackId(id + 1),
            name: "".into(),
            kind,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![],
        });
    }
    let stream = |index, kind, video, audio| SourceStream {
        index,
        kind,
        codec: "fixture".into(),
        time_base: TimeBase::new(1, 1000).unwrap(),
        duration: Some(Tick(1000)),
        start: Some(Tick(0)),
        frame_rate: None,
        frame_count: None,
        variable_frame_rate: None,
        video,
        audio,
    };
    data.assets.push(
        SourceAsset::try_from(AssetData {
            id: AssetId(1),
            path: std::env::temp_dir().join("Tiếng Việt missing media.mp4"),
            content_hash: ContentHash([0; 32]),
            probe: SourceProbe {
                format: "mp4".into(),
                duration: Some(Tick(1000)),
                streams: vec![
                    stream(
                        0,
                        StreamKind::Video,
                        Some(VideoInfo {
                            width: 16,
                            height: 16,
                            color: ColorInfo::default(),
                            bit_depth: Some(8),
                        }),
                        None,
                    ),
                    stream(
                        1,
                        StreamKind::Audio,
                        None,
                        Some(AudioInfo {
                            sample_rate: 48000,
                            channels: 2,
                            initial_padding: 0,
                            trailing_padding: 0,
                        }),
                    ),
                ],
            },
        })
        .unwrap(),
    );
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let out = s
        .apply(Command::InsertItem {
            track: TrackId(1),
            item: NewItem {
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: range(200, 300),
                start: Tick(100),
                content: ItemParams::Video(VideoParams {
                    source_audio_enabled: true,
                }),
            },
        })
        .unwrap();
    let CreatedId::Item(id) = *out.created.first().unwrap() else {
        panic!("item ID")
    };
    s.apply(Command::TrimStart {
        item: id,
        at: Tick(0),
    })
    .unwrap();
    assert_eq!(get(&s, id.0).source_range, range(100, 300));
    s.apply(Command::TrimEnd {
        item: id,
        at: Tick(900),
    })
    .unwrap();
    assert_eq!(get(&s, id.0).source_range, range(100, 1000));
    let snapshot = s.project().clone();
    let history = s.history_len();
    for command in [
        Command::TrimEnd {
            item: id,
            at: Tick(901),
        },
        Command::MoveItem {
            item: id,
            track: TrackId(2),
            start: Tick(0),
        },
        Command::SetItemParam {
            item: id,
            content: ItemParams::Audio(AudioParams {
                gain_db: 0.0,
                muted: false,
            }),
        },
        Command::InsertItem {
            track: TrackId(2),
            item: NewItem {
                animations: Default::default(),
                source_asset: Some(AssetId(99)),
                source_range: range(0, 10),
                start: Tick(0),
                content: ItemParams::Audio(AudioParams {
                    gain_db: 0.0,
                    muted: false,
                }),
            },
        },
        Command::InsertItem {
            track: TrackId(2),
            item: NewItem {
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: range(0, 10),
                start: Tick(0),
                content: ItemParams::Audio(AudioParams {
                    gain_db: f32::NAN,
                    muted: false,
                }),
            },
        },
    ] {
        assert!(s.apply(command).is_err());
        assert!(Arc::ptr_eq(&snapshot, s.project()));
        assert_eq!(s.history_len(), history);
    }
}

#[test]
fn caption_splits_and_trims_reuse_mapping_and_preserve_source_text_metadata() {
    let mut s = Session::new(caption_project(), 10).unwrap();
    let ItemParams::Caption(params) = &get(&s, 1).content else {
        panic!("caption")
    };
    let cue = params.cues.cues[0].clone();
    let original = s.project().clone();
    let outcome = s
        .apply(Command::SplitAt {
            item: ItemId(1),
            at: Tick(1050),
        })
        .unwrap();
    let CreatedId::Item(right) = *outcome.created.first().unwrap() else {
        panic!("item ID")
    };
    for (id, expected) in [(ItemId(1), range(110, 150)), (right, range(150, 190))] {
        let ItemParams::Caption(params) = &get(&s, id.0).content else {
            panic!("caption")
        };
        let clipped = params.cues.cues.first().unwrap();
        assert_eq!(clipped.range, expected);
        assert_eq!(clipped.id, cue.id);
        assert_eq!(clipped.texts, cue.texts);
        assert_eq!(clipped.source_lang, cue.source_lang);
    }
    s.apply(Command::TrimEnd {
        item: ItemId(1),
        at: Tick(1020),
    })
    .unwrap();
    let ItemParams::Caption(params) = &get(&s, 1).content else {
        panic!("caption")
    };
    assert_eq!(params.cues.cues.first().unwrap().range, range(110, 120));
    assert_eq!(params.cues.cues.first().unwrap().id, cue.id);
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&original, s.project()));
}

#[test]
fn noops_preserve_redo_and_refusals_preserve_merge_state() {
    let mut s = session();
    let drag = |start| Command::MoveItem {
        item: ItemId(10),
        track: TrackId(2),
        start: Tick(start),
    };
    s.apply_merged(drag(0), Some("gesture")).unwrap();
    assert!(!s.apply(Command::Delete { items: vec![] }).unwrap().changed);
    s.apply_merged(drag(10), Some("gesture")).unwrap();
    assert_eq!(s.history_len(), (2, 0));
    assert!(s.execute("bad", json!({})).is_err());
    s.apply_merged(drag(20), Some("gesture")).unwrap();
    assert_eq!(s.history_len(), (2, 0));
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
    let track = data.timeline.overlays.first_mut().unwrap();
    track.items = (0..10000)
        .map(|n| Item {
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
        for n in 0..100 {
            s.apply_merged(
                Command::MoveItem {
                    item: ItemId(10009),
                    track: TrackId(1),
                    start: Tick(199_980 + (n % 2)),
                },
                Some("benchmark"),
            )
            .unwrap();
        }
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
    s.execute("begin_gesture", json!({})).unwrap();
    assert!(s.gesture_active());
    assert!(!s.can_undo());
    assert!(matches!(
        s.begin_gesture(),
        Err(EditError::TransactionActive)
    ));
    assert!(matches!(s.undo(), Err(EditError::TransactionActive)));
    for start in 1..10 {
        s.execute("move_item", json!({"item":10,"track":2,"start":start}))
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
    s.execute("cancel_gesture", json!({})).unwrap();
    assert!(Arc::ptr_eq(&redo_base, s.project()));
    assert!(s.can_redo());
    s.redo().unwrap();
    s.take_change();
    s.begin_gesture().unwrap();
    for start in 1..100 {
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(start),
            },
            Some("same"),
        )
        .unwrap();
    }
    assert_eq!(s.history_len(), (1, 0));
    assert!(s.take_change().is_none());
    let final_preview = s.project().clone();
    s.execute("commit_gesture", json!({})).unwrap();
    assert_eq!(s.history_len(), (1, 0));
    assert!(Arc::ptr_eq(&s.take_change().unwrap(), &final_preview));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&committed, s.project()));
    assert!(matches!(s.cancel_gesture(), Err(EditError::NoGesture)));
    assert!(matches!(s.commit_gesture(), Err(EditError::NoGesture)));
}

#[test]
fn failed_batch_at_every_position_preserves_redo_and_pending_event() {
    for position in 0..5 {
        let mut s = Session::new(project(), 1).unwrap();
        s.apply(Command::Delete {
            items: vec![ItemId(11)],
        })
        .unwrap();
        s.undo().unwrap();
        let before = s.project().clone();
        let history = s.history_len();
        let mut commands = vec![
            Command::AddBookmark {
                start: Tick(0),
                duration: Tick(0),
                note: "".into(),
                color: Color([0; 4])
            };
            4
        ];
        commands.insert(
            position,
            Command::Delete {
                items: vec![ItemId(99)],
            },
        );
        assert!(s.apply(Command::Batch { commands }).is_err());
        assert!(Arc::ptr_eq(&before, s.project()));
        assert_eq!(s.history_len(), history);
        assert!(Arc::ptr_eq(&before, &s.take_change().unwrap()));
        assert!(s.can_redo());
        s.redo().unwrap();
    }
}

#[test]
fn grouped_failure_and_unwind_restore_every_staged_change_without_eviction() {
    for unwind in [false, true] {
        let mut s = Session::new(project(), 1).unwrap();
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(0),
            },
            Some("prior"),
        )
        .unwrap();
        s.apply(Command::Delete {
            items: vec![ItemId(11)],
        })
        .unwrap();
        s.undo().unwrap();
        let before = s.project().clone();
        let history = s.history_len();
        let result: engine::Result<()> = s.transaction(|group| {
            assert!(matches!(
                group.transaction(|_| Ok(())),
                Err(EditError::TransactionActive)
            ));
            assert!(matches!(
                group.begin_gesture(),
                Err(EditError::TransactionActive)
            ));
            assert!(matches!(group.redo(), Err(EditError::TransactionActive)));
            for _ in 0..10 {
                group.apply(Command::AddBookmark {
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
            assert!(matches!(result, Err(EditError::Panicked)));
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
    s.transaction(|group| {
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
    s.transaction(|_| Ok(())).unwrap();
    assert!(s.can_redo());
    assert!(s.take_change().is_none());
    assert!(!Arc::ptr_eq(&initial, s.project()));
}

#[test]
fn group_rollback_restores_merge_key_and_explicit_boundaries_break_it() {
    for unwind in [false, true] {
        let mut s = session();
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(0),
            },
            Some("drag"),
        )
        .unwrap();
        let result: engine::Result<()> = s.transaction(|group| {
            group.apply(Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(5),
            })?;
            if unwind {
                panic!("injected group unwind");
            }
            Err(EditError::Invalid("refused"))
        });
        assert!(result.is_err());
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(10),
            },
            Some("drag"),
        )
        .unwrap();
        assert_eq!(s.history_len(), (1, 0));
        // App save/selection transitions call this same engine-owned boundary.
        s.end_merge();
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(20),
            },
            Some("drag"),
        )
        .unwrap();
        assert_eq!(s.history_len(), (2, 0));
        s.undo().unwrap();
        s.redo().unwrap();
        s.apply_merged(
            Command::MoveItem {
                item: ItemId(10),
                track: TrackId(2),
                start: Tick(30),
            },
            Some("drag"),
        )
        .unwrap();
        assert_eq!(s.history_len(), (3, 0));
    }
}

fn caption_project() -> Project {
    use captions::{Cue, CueText, CueTrack, LanguageTag};
    use project::CaptionParams;
    use std::collections::BTreeMap;
    let mut data = project().into_data();
    data.timeline.overlays.clear();
    data.assets.push(
        SourceAsset::try_from(AssetData {
            id: AssetId(1),
            path: std::env::temp_dir().join("caption source"),
            content_hash: ContentHash([0; 32]),
            probe: SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: vec![],
            },
        })
        .unwrap(),
    );
    let lang = LanguageTag::parse("vi").unwrap();
    let cue = Cue {
        id: "source/0".into(),
        range: range(110, 190),
        source_lang: lang.clone(),
        texts: BTreeMap::from([(
            lang,
            CueText {
                text: "Tiếng Việt".into(),
                edited: true,
                translation_ref: Some("ref".into()),
            },
        )]),
    };
    data.timeline.insert_track(Track {
        id: TrackId(1),
        name: "".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(1),
            source_asset: Some(AssetId(1)),
            source_range: range(100, 200),
            start: Tick(1000),
            content: ItemParams::Caption(CaptionParams {
                cues: CueTrack {
                    cues: vec![cue.clone()],
                },
            }),
        }],
    });
    Project::new(data).unwrap()
}

fn caption_trim(start: bool, at: i64) -> Command {
    if start {
        Command::TrimStart {
            item: ItemId(1),
            at: Tick(at),
        }
    } else {
        Command::TrimEnd {
            item: ItemId(1),
            at: Tick(at),
        }
    }
}

#[test]
fn caption_drag_inward_and_back_preserves_original_content_and_redo() {
    for start in [false, true] {
        // Exercise partial clipping and a preview that removes the cue entirely.
        for inward in if start { [1050, 1095] } else { [1050, 1005] } {
            let mut s = Session::new(caption_project(), 10).unwrap();
            let original = s.project().clone();
            s.apply(Command::AddBookmark {
                start: Tick(0),
                duration: Tick(0),
                note: "redo".into(),
                color: Color([0, 0, 0, 255]),
            })
            .unwrap();
            let redo = s.project().clone();
            s.undo().unwrap();
            s.take_change();
            s.begin_gesture().unwrap();
            s.apply(caption_trim(start, inward)).unwrap();
            s.apply(caption_trim(start, if start { 1000 } else { 1100 }))
                .unwrap();
            assert_eq!(s.project().data(), original.data());
            assert!(!s.commit_gesture().unwrap().changed);
            assert_eq!(s.history_len(), (0, 1));
            assert!(s.take_change().is_none());
            s.redo().unwrap();
            assert_eq!(s.project().data(), redo.data());
        }
    }
}

#[test]
fn caption_drag_commit_cancel_undo_redo_and_group_paths() {
    for start in [false, true] {
        for mode in ["gesture", "group", "batch", "merged"] {
            let mut s = Session::new(caption_project(), 10).unwrap();
            let original = s.project().clone();
            let inward = if start { 1095 } else { 1005 };
            let outward = if start { 1020 } else { 1080 };
            let mut direct = Session::new(caption_project(), 10).unwrap();
            direct.apply(caption_trim(start, outward)).unwrap();
            let drag = |s: &mut Session| -> engine::Result<()> {
                for at in [inward, outward] {
                    if mode == "merged" {
                        s.apply_merged(caption_trim(start, at), Some("trim"))?;
                    } else {
                        s.apply(caption_trim(start, at))?;
                    }
                }
                Ok(())
            };
            match mode {
                "gesture" => {
                    s.begin_gesture().unwrap();
                    drag(&mut s).unwrap();
                    assert!(s.take_change().is_none());
                    s.cancel_gesture().unwrap();
                    assert_eq!(s.project().data(), original.data());
                    assert_eq!(s.history_len(), (0, 0));
                    assert!(s.take_change().is_none());
                    s.begin_gesture().unwrap();
                    drag(&mut s).unwrap();
                    assert!(s.commit_gesture().unwrap().changed);
                }
                "group" => s.transaction(drag).unwrap(),
                "batch" => {
                    s.apply(Command::Batch {
                        commands: vec![caption_trim(start, inward), caption_trim(start, outward)],
                    })
                    .unwrap();
                }
                _ => drag(&mut s).unwrap(),
            }
            assert_eq!(s.project().data(), direct.project().data());
            assert_eq!(s.history_len(), (1, 0));
            assert!(s.take_change().is_some());
            s.undo().unwrap();
            assert_eq!(s.project().data(), original.data());
            s.redo().unwrap();
            assert_eq!(s.project().data(), direct.project().data());
            // A boundary discards transient source retention: later independent
            // edits cannot resurrect content discarded by the committed trim.
            s.end_merge();
            s.apply(caption_trim(start, if start { 1000 } else { 1100 }))
                .unwrap();
            let ItemParams::Caption(params) = &get(&s, 1).content else {
                panic!("caption")
            };
            let ItemParams::Caption(expected) = &get(&direct, 1).content else {
                panic!("caption")
            };
            assert_eq!(params, expected);
        }
    }
}

#[test]
fn explicit_caption_edits_replace_retained_gesture_content_and_refusals_roll_back() {
    for group in [false, true] {
        let mut s = Session::new(caption_project(), 10).unwrap();
        let run = |s: &mut Session| -> engine::Result<()> {
            s.apply(caption_trim(false, 1050))?;
            let mut content = get(s, 1).content.clone();
            let ItemParams::Caption(params) = &mut content else {
                panic!("caption")
            };
            let lang = captions::LanguageTag::parse("en").unwrap();
            params.cues.cues[0].texts.insert(
                lang,
                captions::CueText {
                    text: "Edited translation".into(),
                    edited: true,
                    translation_ref: Some("new".into()),
                },
            );
            // The replacement must not survive a refused batch.
            assert!(
                s.apply(Command::Batch {
                    commands: vec![
                        Command::SetItemParam {
                            item: ItemId(1),
                            content: content.clone()
                        },
                        Command::Delete {
                            items: vec![ItemId(999)]
                        },
                    ]
                })
                .is_err()
            );
            s.apply(caption_trim(false, 1100))?;
            assert_eq!(s.project().data(), caption_project().data());
            s.apply(caption_trim(false, 1050))?;
            s.apply(Command::SetItemParam {
                item: ItemId(1),
                content: content.clone(),
            })?;
            s.apply(caption_trim(false, 1100))?;
            assert_eq!(get(s, 1).content, content);
            Ok(())
        };
        if group {
            s.transaction(run).unwrap();
        } else {
            s.begin_gesture().unwrap();
            run(&mut s).unwrap();
            s.commit_gesture().unwrap();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn caption_gesture_previews_depend_only_on_final_edge(
        start in any::<bool>(),
        edges in prop::collection::vec(1001i64..1100, 1..40),
    ) {
        let mut s = Session::new(caption_project(), 10).unwrap();
        s.begin_gesture().unwrap();
        for at in &edges {
            s.apply(caption_trim(start, *at)).unwrap();
            let mut direct = Session::new(caption_project(), 10).unwrap();
            direct.apply(caption_trim(start, *at)).unwrap();
            prop_assert_eq!(s.project().data(), direct.project().data());
        }
        let committed = s.project().clone();
        s.commit_gesture().unwrap();
        s.undo().unwrap();
        let original = caption_project();
        prop_assert_eq!(s.project().data(), original.data());
        s.redo().unwrap();
        prop_assert_eq!(s.project().data(), committed.data());
    }
}

#[test]
fn merged_caption_roundtrip_and_equal_content_replacement() {
    let mut s = Session::new(caption_project(), 10).unwrap();
    let original = s.project().clone();
    for at in [1005, 1100] {
        s.apply_merged(caption_trim(false, at), Some("trim"))
            .unwrap();
    }
    assert_eq!(s.project().data(), original.data());
    s.end_merge();
    s.begin_gesture().unwrap();
    s.apply(caption_trim(false, 1050)).unwrap();
    let replacement = get(&s, 1).content.clone();
    assert!(
        !s.apply(Command::SetItemParam {
            item: ItemId(1),
            content: replacement.clone(),
        })
        .unwrap()
        .changed
    );
    s.apply(caption_trim(false, 1100)).unwrap();
    assert_eq!(get(&s, 1).content, replacement);
    s.commit_gesture().unwrap();
}

#[test]
fn track_output_controls_enforce_kind_capabilities() {
    let mut s = Session::new(Project::empty(ProjectId(1), "Tracks".into()).unwrap(), 100).unwrap();
    for (kind, mute_allowed, hide_allowed) in [
        (TrackKind::Video, true, true),
        (TrackKind::Audio, true, false),
        (TrackKind::Text, false, true),
        (TrackKind::Sticker, false, true),
        (TrackKind::Effect, false, true),
        (TrackKind::Filter, false, true),
        (TrackKind::Adjustment, false, true),
    ] {
        let result = s
            .apply(Command::AddTrack {
                name: "Track".into(),
                kind,
            })
            .unwrap();
        let track = result
            .created
            .iter()
            .find_map(|id| match id {
                CreatedId::Track(track) => Some(*track),
                _ => None,
            })
            .unwrap();
        for (command, field, allowed) in [
            ("track.set_muted", "muted", mute_allowed),
            ("track.set_hidden", "hidden", hide_allowed),
        ] {
            let before = s.project().clone();
            let result = s.execute(command, json!({"track":track,(field):true}));
            if allowed {
                result.unwrap();
                let value = s
                    .project()
                    .data()
                    .timeline
                    .iter()
                    .find(|value| value.id == track)
                    .unwrap();
                assert!(match field {
                    "muted" => value.muted,
                    _ => value.hidden,
                });
            } else {
                assert!(result.is_err());
                assert!(Arc::ptr_eq(&before, s.project()));
            }
        }
    }
}
