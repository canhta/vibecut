#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::{Command, CreatedId, EditError, Move, NewItem, Session, registry};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use proptest::prelude::*;
use serde_json::{Value, json};
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
        retime: Default::default(),
        content: content(),
    }
}
fn project() -> Project {
    let mut data = Project::empty(ProjectId(0), "Tiếng Việt".into())
        .unwrap()
        .into_data();
    for id in [1, 0] {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(id + 1),
            name: "track".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: Vec::new(),
        });
    }
    data.scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items = vec![
        Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(10),
            source_asset: None,
            source_range: range(100, 140),
            start: Tick(0),
            content: content(),
        },
        Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
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
    let preview = |x: f64, text: &str| {
        json!({"commands":[
            {"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"transform.positionX","value":x}}},
            {"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"transform.positionY","value":-25.0}}},
            {"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"transform.scaleX","value":0.5}}},
            {"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"transform.scaleY","value":0.75}}},
            {"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"transform.rotate","value":35.0}}},
            {"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"text_content","value":text}}}
        ]})
    };
    for x in [10., 40., -20.] {
        s.execute(
            "timeline.preview",
            preview(x, "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t"),
        )
        .unwrap();
    }
    assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    assert_eq!(s.history_len(), (0, 0));
    let candidate = s.preview_project().unwrap().clone();
    for invalid in [
        json!({"commands":[{"command":"timeline.set_item_param","params":{"item":999,"patch":{"field":"transform.positionX","value":2.}}}]}),
        json!({"commands":[{"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"transform.scaleX","value":0.}}}]}),
        json!({"commands":[{"command":"timeline.set_item_param","params":{"item":10,"patch":{"field":"text_content","value":"tentative"}}},{"command":"undo","params":{}}]}),
    ] {
        assert!(s.execute("timeline.preview", invalid).is_err());
        assert_eq!(s.preview_project(), Some(&candidate));
        assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    }
    s.execute("timeline.discard_preview", json!({})).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), initial);
    assert_eq!(s.selection(), &[selected].into());
    assert_eq!(s.history_len(), (0, 0));
    // Reversing every edit to its original value produces no history entry.
    s.execute("timeline.preview", preview(20., "draft"))
        .unwrap();
    s.execute("timeline.preview", json!({"commands":[]}))
        .unwrap();
    assert!(
        !s.execute("timeline.commit_preview", json!({}))
            .unwrap()
            .changed
    );
    assert_eq!(s.history_len(), (0, 0));
    s.execute(
        "timeline.preview",
        preview(-20., "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t"),
    )
    .unwrap();
    assert!(
        s.execute("timeline.commit_preview", json!({}))
            .unwrap()
            .changed
    );
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
    assert!(
        locked
            .execute("timeline.preview", preview(25., "refused"))
            .is_err()
    );
    assert_eq!(locked.project(), &before);
    assert!(locked.preview_project().is_none());
    assert_eq!(locked.history_len(), (0, 0));
}

#[test]
fn timeline_preview_isolated_replacement_commit_discard_and_history() {
    let mut s = session();
    let bytes = project::encode(s.project(), false).unwrap();
    let preview = |delta| {
        json!({"commands":[{"command":"timeline.move_items",
        "params":{"items":[10,11],"delta":delta,"placement":{"mode":"track","track":2}}}]})
    };
    s.execute("timeline.preview", preview(20)).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(s.history_len(), (0, 0));
    assert!(s.take_change().is_none());
    s.execute("timeline.preview", preview(5)).unwrap();
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
    assert!(s.execute("timeline.preview", preview(-1)).is_err());
    assert_eq!(s.preview_project(), Some(&candidate));
    s.execute("timeline.discard_preview", json!({})).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(s.history_len(), (0, 0));
    assert!(s.preview_project().is_none());
    assert!(s.take_change().is_none());
    s.execute("timeline.preview", preview(5)).unwrap();
    s.execute("timeline.commit_preview", json!({})).unwrap();
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
    for (command, params) in [
        ("timeline.move_items", json!({"items":[],"delta":1})),
        ("timeline.move_items", json!({"items":[10,10],"delta":1})),
        ("timeline.move_items", json!({"items":[10,999],"delta":1})),
        ("timeline.move_items", json!({"items":[10],"delta":-1})),
        (
            "timeline.move_items",
            json!({"items":[10],"delta":-1,"placement":{"mode":"above","track":0}}),
        ),
        ("timeline.move_items", json!({"items":[10],"delta":30})),
        (
            "timeline.move_items",
            json!({"items":[10],"delta":1,"placement":{"mode":"track","track":999}}),
        ),
        ("timeline.trim", json!({"items":[],"edge":"end","delta":1})),
        (
            "timeline.trim",
            json!({"items":[10,10],"edge":"end","delta":1}),
        ),
        (
            "timeline.trim",
            json!({"items":[10,999],"edge":"end","delta":-1}),
        ),
        (
            "timeline.trim",
            json!({"items":[10,11],"edge":"start","delta":40}),
        ),
        (
            "timeline.trim",
            json!({"items":[10,11],"edge":"end","delta":21}),
        ),
        (
            "timeline.trim",
            json!({"items":[10],"edge":"invalid","delta":1}),
        ),
        (
            "timeline.preview",
            json!({"commands":[{"command":"undo","params":{}}]}),
        ),
        (
            "timeline.preview",
            json!({"commands":[{"command":"timeline.commit_preview","params":{}}]}),
        ),
    ] {
        assert!(
            s.execute(command, params.clone()).is_err(),
            "{command}: {params}"
        );
        assert!(Arc::ptr_eq(s.project(), &before));
        assert_eq!(s.history_len(), (0, 0));
        assert!(!s.gesture_active());
    }
    assert!(s.execute("timeline.commit_preview", json!({})).is_err());
    assert!(s.execute("timeline.discard_preview", json!({})).is_err());
    assert!(matches!(
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(10); 100_001],
            delta: Tick(0),
            placement: None
        }),
        Err(EditError::Limit)
    ));
    assert!(
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(11)],
            delta: Tick(i64::MAX),
            placement: None
        })
        .is_err()
    );
    s.execute("track.set_locked", json!({"track":1,"locked":true}))
        .unwrap();
    let locked = s.project().clone();
    assert!(
        s.execute("timeline.move_items", json!({"items":[10],"delta":1}))
            .is_err()
    );
    assert!(
        s.execute(
            "timeline.trim",
            json!({"items":[10],"edge":"end","delta":-1})
        )
        .is_err()
    );
    assert_eq!(s.project(), &locked);
}

#[test]
fn timeline_preview_reversal_preserves_redo_and_retires_created_tracks() {
    let mut s = session();
    s.execute("timeline.move_items", json!({"items":[10,11],"delta":2}))
        .unwrap();
    s.undo().unwrap();
    let original = s.project().clone();
    let history = s.history_len();
    s.take_change();
    let preview = |delta| {
        json!({"commands":[{"command":"timeline.move_items",
        "params":{"items":[10,11],"delta":delta}}]})
    };
    s.execute("timeline.preview", preview(4)).unwrap();
    assert!(matches!(s.saved(), Err(EditError::TransactionActive)));
    assert!(matches!(s.undo(), Err(EditError::TransactionActive)));
    assert!(matches!(
        s.begin_gesture(),
        Err(EditError::TransactionActive)
    ));
    assert!(s.set_selection([ItemId(10)].into()).is_err());
    s.execute("timeline.preview", preview(0)).unwrap();
    assert!(
        !s.execute("timeline.commit_preview", json!({}))
            .unwrap()
            .changed
    );
    assert_eq!(s.project(), &original);
    assert_eq!(s.history_len(), history);
    assert!(s.can_redo());
    assert!(s.take_change().is_none());
    let new_track = json!({"commands":[{"command":"timeline.move_items","params":{
        "items":[10,11],"delta":1,"placement":{"mode":"above","track":0}}}]});
    let retired = s
        .execute("timeline.preview", new_track.clone())
        .unwrap()
        .created;
    s.execute("timeline.discard_preview", json!({})).unwrap();
    let fresh = s.execute("timeline.preview", new_track).unwrap().created;
    assert_ne!(retired, fresh);
    s.execute("timeline.discard_preview", json!({})).unwrap();
    assert_eq!(s.project(), &original);
    assert_eq!(s.history_len(), history);
    // An empty pointer gesture is promoted to a replacement preview and discards cleanly.
    s.begin_gesture().unwrap();
    s.execute("timeline.preview", preview(1)).unwrap();
    s.execute("timeline.discard_preview", json!({})).unwrap();
    assert_eq!(s.project(), &original);
    // A gesture that already staged an edit refuses: a replacement would drop it.
    s.begin_gesture().unwrap();
    s.execute("timeline.move_items", json!({"items":[10,11],"delta":1}))
        .unwrap();
    assert!(s.execute("timeline.preview", preview(1)).is_err());
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
        assert!(s.execute(commit, json!({})).is_err());
        assert_eq!(s.project(), &before);
        assert_eq!(s.preview_project(), Some(&candidate));
        assert_eq!(s.history_len(), (0, 0));
        s.execute("timeline.discard_preview", json!({})).unwrap();
        assert_eq!(s.project(), &before);
    }
}

#[test]
fn timeline_group_new_track_and_main_origin_keep_common_delta() {
    for placement in [
        json!({"mode":"above","track":0}),
        json!({"mode":"below","track":1}),
    ] {
        let mut s = session();
        let before = s.project().clone();
        let outcome = s
            .execute(
                "timeline.move_items",
                json!({"items":[10,11],"delta":5,"placement":placement}),
            )
            .unwrap();
        assert_eq!(outcome.created.len(), 1);
        assert_eq!(get(&s, 11).start.0 - get(&s, 10).start.0, 60);
        let candidate = s.project().clone();
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
        s.redo().unwrap();
        assert_eq!(s.project(), &candidate);
        s.execute(
            "timeline.move_items",
            json!({"items":[10,11],"delta":50,
        "placement":{"mode":"track","track":0}}),
        )
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
        s.execute("timeline.move_items", json!({"items":[10,11],"delta":delta})).unwrap();
        prop_assert_eq!(get(&s,11).start.0-get(&s,10).start.0,60);
        s.execute("timeline.trim", json!({"items":[10,11],
            "edge":if start_edge {"start"} else {"end"},"delta":if start_edge {resize} else {-resize}})).unwrap();
        prop_assert_eq!(get(&s,11).start.0-get(&s,10).start.0,60);
        prop_assert_eq!(get(&s,10).duration().unwrap(), Tick(40-resize));
        prop_assert_eq!(get(&s,11).duration().unwrap(), Tick(40-resize));
        prop_assert!(get(&s,10).end().unwrap() <= get(&s,11).start);
        prop_assert!(Project::new(s.project().data().clone()).is_ok());
    }
}

#[test]
fn transform_command_bounds_atomicity_undo_and_persistence() {
    let mut session = session();
    for field in project::TransformField::ALL {
        let before = session.project().clone();
        let (min, max) = field.bounds();
        for invalid in [min - 0.01, max + 0.01] {
            assert!(
                session
                    .execute(
                        "timeline.set_item_param",
                        json!({"item":10,"patch":{"field":field.path(),"value":invalid}})
                    )
                    .is_err()
            );
            assert_eq!(session.project(), &before);
        }
        session
            .execute(
                "timeline.set_item_param",
                json!({"item":10,"patch":{"field":field.path(),"value":max}}),
            )
            .unwrap();
        assert_eq!(
            field
                .value_at(&get(&session, 10).animations, Tick::ZERO)
                .unwrap(),
            max
        );
        let saved = project::encode(session.project(), false).unwrap();
        assert_eq!(
            project::decode(&saved).unwrap().project,
            *session.project().as_ref()
        );
        session.undo().unwrap();
        assert_eq!(session.project(), &before);
        session.redo().unwrap();
        assert_eq!(
            field
                .value_at(&get(&session, 10).animations, Tick::ZERO)
                .unwrap(),
            max
        );
    }
    session
        .execute("project.set_background", json!({"color":[12,34,56,78]}))
        .unwrap();
    assert_eq!(
        session.project().data().canvas.background,
        Color([12, 34, 56, 78])
    );
}
#[test]
fn crop_patch_is_validated_undoable_persisted_and_survives_trim_and_retime() {
    use engine::ItemParamPatch;
    let mut data = project().into_data();
    let asset = testkit::source::asset(7, std::env::temp_dir().join("crop-controls.mp4"))
        .format("mp4")
        .duration(Tick(1000))
        .content_hash(ContentHash::from_bytes(&[7]))
        .stream(
            testkit::source::video()
                .duration(Tick(1000))
                .start(Tick::ZERO)
                .build(),
        )
        .build();
    data.assets.push(asset);
    data.scene_mut().timeline.main.items = vec![Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(20),
        source_asset: Some(AssetId(7)),
        source_range: range(0, 500),
        start: Tick::ZERO,
        content: ItemParams::Video(project::VideoParams {
            crop: None,
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
        }),
    }];
    let mut s = Session::new(Project::new(data).unwrap(), 100).unwrap();
    let initial = s.project().clone();
    let crop = |s: &Session| match &get(s, 20).content {
        ItemParams::Video(p) => p.crop,
        _ => panic!("video fixture"),
    };
    assert_eq!(crop(&s), None);
    let centred = project::Crop {
        x: 0.25,
        y: 0.25,
        width: 0.5,
        height: 0.5,
    };
    s.execute(
        "timeline.set_item_param",
        json!({"item":20,"patch":{"field":"crop","value":{"x":0.25,"y":0.25,"width":0.5,"height":0.5}}}),
    )
    .unwrap();
    assert_eq!(crop(&s), Some(centred));
    assert_eq!(s.history_len(), (1, 0));
    let edited = s.project().clone();
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &initial);
    s.redo().unwrap();
    assert_eq!(s.project(), &edited);
    for invalid in [
        project::Crop {
            x: 0.25,
            y: 0.25,
            width: 0.0,
            height: 0.5,
        },
        project::Crop {
            x: 0.25,
            y: 0.25,
            width: -0.1,
            height: 0.5,
        },
        project::Crop {
            x: 0.75,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        },
        project::Crop {
            x: 0.25,
            y: 0.75,
            width: 0.5,
            height: 0.5,
        },
        project::Crop {
            x: -0.01,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        },
        project::Crop {
            x: 0.0,
            y: 0.0,
            width: f64::NAN,
            height: 0.5,
        },
        project::Crop {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: f64::INFINITY,
        },
    ] {
        let before = s.project().clone();
        assert!(
            s.apply(Command::SetItemField {
                item: ItemId(20),
                patch: ItemParamPatch::Crop(Some(invalid)),
            })
            .is_err(),
            "{invalid:?}"
        );
        assert_eq!(s.project(), &before);
    }
    s.execute("trim_start", json!({"item":20,"at":50})).unwrap();
    assert_eq!(crop(&s), Some(centred));
    s.execute("trim_end", json!({"item":20,"at":400})).unwrap();
    assert_eq!(crop(&s), Some(centred));
    s.execute(
        "timeline.set_speed",
        json!({"items":[20],"speed":2.0,"maintain_pitch":true}),
    )
    .unwrap();
    assert_eq!(crop(&s), Some(centred));
    s.execute(
        "timeline.set_item_param",
        json!({"item":20,"patch":{"field":"crop","value":null}}),
    )
    .unwrap();
    assert_eq!(crop(&s), None);
    let before = s.project().clone();
    assert!(
        s.apply(Command::SetItemField {
            item: ItemId(10),
            patch: ItemParamPatch::Crop(Some(centred)),
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
}
fn get(s: &Session, id: u64) -> &Item {
    get_project(s.project(), id)
}
fn get_project(project: &Project, id: u64) -> &Item {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(id))
        .unwrap()
}
fn get_id(s: &Session, id: ItemId) -> &Item {
    get(s, id.0)
}
fn get_project_id(project: &Project, id: ItemId) -> &Item {
    get_project(project, id.0)
}
#[test]
fn all_commands_share_registry_and_strict_json_dispatch() {
    let mut s = Session::new(Project::empty(ProjectId(0), "".into()).unwrap(), 10).unwrap();
    let r = s
        .execute(
            "timeline.add_track",
            json!({"name":"Tiếng", "kind":"video"}),
        )
        .unwrap();
    assert_eq!(r.created, vec![CreatedId::Track(TrackId(1))]);
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .overlays
            .first()
            .unwrap()
            .name
            .as_str(),
        "Tiếng"
    );
    let r = s
        .execute(
            "timeline.insert",
            json!({"placement":{"mode":"track","track":1},"item":new_item(0,10)}),
        )
        .unwrap();
    assert_eq!(r.created, vec![CreatedId::Item(ItemId(2))]);
    s.execute("move_item", json!({"item":2,"track":1,"start":10}))
        .unwrap();
    s.execute("set_item_param", json!({"item":2,"content":content()}))
        .unwrap();
    s.execute(
        "bookmark.add",
        json!({"start":1,"duration":0,"note":"ghi chú","color":[0,0,0,255]}),
    )
    .unwrap();
    let asset = SourceAsset::try_from(AssetData {
        ephemeral: false,
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
    s.execute("import.add_asset", json!({"asset":asset}))
        .unwrap();
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
fn split_variants_refuse_boundaries_and_remove_the_requested_side() {
    let (mut s, _video, _audio, clip) = audible_session();
    let start = Tick(0);
    let end = Tick(100);
    let before = s.project().clone();
    for at in [start, end, Tick(101)] {
        for command in [
            Command::TimelineSplitLeft {
                items: vec![clip],
                at,
            },
            Command::TimelineSplitRight {
                items: vec![clip],
                at,
            },
        ] {
            let result = s.apply(command);
            let Err(error) = result else {
                panic!("at {at:?} unexpectedly succeeded");
            };
            assert!(
                matches!(
                    error,
                    EditError::Invalid(_) | EditError::Project(project::Error::Invalid(_))
                ),
                "at {at:?}: {error:?}"
            );
            assert!(Arc::ptr_eq(&before, s.project()));
        }
    }
    let out = s
        .apply(Command::TimelineSplitLeft {
            items: vec![clip],
            at: Tick(40),
        })
        .unwrap();
    let CreatedId::Item(kept) = out.created.first().copied().unwrap() else {
        panic!("right piece");
    };
    assert_eq!(get(&s, kept.0).start, Tick(40));
    assert_eq!(get(&s, kept.0).source_range, range(40, 100));
    assert!(
        !s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .any(|track| track.items.iter().any(|item| item.id == clip))
    );
    s.undo().unwrap();
    assert_eq!(get(&s, clip.0).source_range, range(0, 100));
    s.redo().unwrap();
    assert_eq!(get(&s, kept.0).source_range, range(40, 100));

    let (mut s, _, _, clip) = audible_session();
    let out = s
        .apply(Command::TimelineSplitRight {
            items: vec![clip],
            at: Tick(40),
        })
        .unwrap();
    assert!(out.created.is_empty());
    assert_eq!(get(&s, clip.0).source_range, range(0, 40));
    assert_eq!(get(&s, clip.0).start, Tick::ZERO);
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .count(),
        2,
        "the right piece is removed and the audio item stays"
    );
    let after = s.project().clone();
    s.undo().unwrap();
    assert_eq!(get(&s, clip.0).source_range, range(0, 100));
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&after, s.project()));
}

#[test]
fn duplicate_keeps_start_and_places_named_tracks_at_highest_insert() {
    let (mut s, video, _audio, clip) = audible_session();
    let audio_clip = s
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .id;
    assert_eq!(video, s.project().data().scene().timeline.overlays[0].id);
    let original_start = get_id(&s, clip).start;
    let out = s
        .apply(Command::DuplicateItems {
            items: vec![clip, audio_clip],
        })
        .unwrap();
    let created: Vec<ItemId> = out
        .created
        .iter()
        .filter_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .collect();
    assert_eq!(created.len(), 2);
    let timeline = &s.project().data().scene().timeline;
    assert_eq!(timeline.overlays[0].name.as_str(), "Video (copy)");
    assert_eq!(timeline.overlays[0].kind, TrackKind::Video);
    assert_eq!(timeline.audio[0].name.as_str(), "Audio (copy)");
    assert_eq!(timeline.audio[0].kind, TrackKind::Audio);
    for copy in &created {
        assert_eq!(get(&s, copy.0).start, original_start);
        assert_eq!(get(&s, copy.0).hidden, get_id(&s, clip).hidden);
    }
    let after = s.project().clone();
    s.undo().unwrap();
    assert_eq!(s.project().data().scene().timeline.overlays.len(), 1);
    assert_eq!(s.project().data().scene().timeline.audio.len(), 1);
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&after, s.project()));
    assert!(matches!(
        s.apply(Command::DuplicateItems {
            items: vec![ItemId(999)],
        }),
        Err(EditError::NoItem(ItemId(999)))
    ));
}

#[test]
fn hide_and_mute_refuse_incompatible_kinds_and_survive_persistence() {
    let (mut s, video, _, clip) = audible_session();
    let audio_clip = s
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .id;
    s.apply(Command::SetItemsHidden {
        items: vec![clip],
        hidden: true,
    })
    .unwrap();
    assert!(get_id(&s, clip).hidden);
    s.apply(Command::SetItemsMuted {
        items: vec![clip, audio_clip],
        muted: true,
    })
    .unwrap();
    for id in [clip, audio_clip] {
        assert!(match &get_id(&s, id).content {
            ItemParams::Video(params) => params.muted,
            ItemParams::Audio(params) => params.muted,
            _ => false,
        });
    }
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::SetItemsHidden {
            items: vec![audio_clip],
            hidden: true,
        }),
        Err(EditError::Invalid("audio item has no video"))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(!get_id(&s, clip).hidden);
    s.redo().unwrap();
    s.redo().unwrap();
    assert!(get_id(&s, clip).hidden);
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(loaded.project.data(), s.project().data());
    assert!(get_project_id(&loaded.project, clip).hidden);
    assert_eq!(video, s.project().data().scene().timeline.overlays[0].id);
}

#[test]
fn extract_and_recover_source_audio_reuse_the_source_range() {
    let (mut s, _, _, clip) = audible_session();
    let source = get_id(&s, clip).source_range;
    let out = s
        .apply(Command::ExtractAudio { items: vec![clip] })
        .unwrap();
    let CreatedId::Item(audio) = out
        .created
        .iter()
        .find(|id| matches!(id, CreatedId::Item(_)))
        .copied()
        .unwrap()
    else {
        panic!("audio item");
    };
    assert!(!matches!(
        &get_id(&s, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    assert_eq!(get(&s, audio.0).source_range, source);
    assert_eq!(get(&s, audio.0).start, get_id(&s, clip).start);
    assert!(matches!(get(&s, audio.0).content, ItemParams::Audio(_)));
    assert!(s.project().data().scene().timeline.audio.len() >= 2);
    let extracted = s.project().clone();
    assert!(matches!(
        s.apply(Command::ExtractAudio { items: vec![clip] }),
        Err(EditError::Invalid("source audio is extracted"))
    ));
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert!(matches!(
        &get_project_id(&loaded.project, clip).content,
        ItemParams::Video(params) if !params.source_audio_enabled
    ));
    assert!(matches!(
        get_project_id(&loaded.project, audio).content,
        ItemParams::Audio(_)
    ));
    assert_eq!(get_project_id(&loaded.project, audio).source_range, source);
    s.undo().unwrap();
    assert!(matches!(
        &get_id(&s, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&extracted, s.project()));
    s.apply(Command::RecoverAudio { items: vec![clip] })
        .unwrap();
    assert!(matches!(
        &get_id(&s, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    assert!(matches!(
        s.apply(Command::RecoverAudio { items: vec![clip] }),
        Err(EditError::Invalid("source audio is enabled"))
    ));
    assert!(matches!(
        s.apply(Command::RecoverAudio { items: vec![audio] }),
        Err(EditError::Invalid("item is not video"))
    ));
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    let recovered = &loaded.project;
    assert!(matches!(
        &get_project_id(recovered, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    assert!(matches!(
        get_project(recovered, audio.0).content,
        ItemParams::Audio(_)
    ));
    assert_eq!(loaded.project.data(), s.project().data());
}

/// One unlocked video overlay with audible source audio and one audio clip.
fn audible_session() -> (Session, TrackId, TrackId, ItemId) {
    let mut s = Session::new(Project::empty(ProjectId(7), "Actions".into()).unwrap(), 50).unwrap();
    let asset = testkit::source::asset(1, std::env::temp_dir().join("vibecut-actions.mp4"))
        .format("mp4")
        .duration(Tick(100))
        .content_hash(ContentHash::from_bytes(b"actions"))
        .stream(
            testkit::source::video()
                .time_base(media::TimeBase::new(1, 30000).unwrap())
                .duration(Tick(100))
                .start(Tick::ZERO)
                .size(64, 64)
                .build(),
        )
        .stream(
            testkit::source::audio()
                .index(1)
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(Tick(100))
                .start(Tick::ZERO)
                .build(),
        )
        .build();
    s.apply(Command::AddAsset { asset }).unwrap();
    let mut add = |name: &str, kind: TrackKind| {
        s.apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: name.into(),
            kind,
        })
        .unwrap()
        .created
        .first()
        .and_then(|id| match id {
            CreatedId::Track(id) => Some(*id),
            _ => None,
        })
        .unwrap()
    };
    let video = add("Video", TrackKind::Video);
    let audio = add("Audio", TrackKind::Audio);
    let insert = |s: &mut Session, track, start: i64, content: ItemParams| {
        s.apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: range(start, start + 100),
                start: Tick(start),
                retime: Default::default(),
                content,
            },
        })
        .unwrap()
        .created
        .first()
        .and_then(|id| match id {
            CreatedId::Item(id) => Some(*id),
            _ => None,
        })
        .unwrap()
    };
    let clip = insert(
        &mut s,
        video,
        0,
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    );
    insert(
        &mut s,
        audio,
        0,
        ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
        }),
    );
    (s, video, audio, clip)
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
            placement: engine::Placement::Track { track: TrackId(2) },
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
        (
            "timeline.insert",
            json!({"placement":{"mode":"track","track":1},"item":null}),
        ),
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
    data.scene_mut().timeline.overlays.first_mut().unwrap().id = TrackId(u64::MAX);
    let mut s = Session::new(Project::new(data).unwrap(), 1).unwrap();
    assert!(matches!(
        s.apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
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
    use media::{AudioInfo, ColorInfo, StreamKind, TimeBase, VideoInfo};
    use project::{AudioParams, VideoParams};
    let mut data = project().into_data();
    data.scene_mut().timeline.overlays.clear();
    for (id, kind) in [(0, TrackKind::Video), (1, TrackKind::Audio)] {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(id + 1),
            name: "".into(),
            kind,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![],
        });
    }
    let stream =
        |index: usize, kind: StreamKind, video: Option<VideoInfo>, audio: Option<AudioInfo>| {
            let mut builder = testkit::source::stream(kind)
                .index(index)
                .codec("fixture")
                .time_base(TimeBase::new(1, 1000).unwrap())
                .duration(Tick(1000))
                .start(Tick(0));
            if let Some(video) = video {
                builder = builder.video_info(video);
            }
            if let Some(audio) = audio {
                builder = builder.audio_info(audio);
            }
            builder.build()
        };
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("Tiếng Việt missing media.mp4"))
            .format("mp4")
            .duration(Tick(1000))
            .stream(stream(
                0,
                StreamKind::Video,
                Some(VideoInfo {
                    width: 16,
                    height: 16,
                    color: ColorInfo::default(),
                    bit_depth: Some(8),
                }),
                None,
            ))
            .stream(stream(
                1,
                StreamKind::Audio,
                None,
                Some(AudioInfo {
                    sample_rate: 48000,
                    channels: 2,
                    initial_padding: 0,
                    trailing_padding: 0,
                }),
            ))
            .build(),
    );
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let out = s
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(1) },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: range(200, 300),
                start: Tick(100),
                content: ItemParams::Video(VideoParams {
                    fades: Default::default(),
                    source_audio_enabled: true,
                    volume: 0.0,
                    muted: false,
                    crop: None,
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
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        },
        Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(2) },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(99)),
                source_range: range(0, 10),
                start: Tick(0),
                content: ItemParams::Audio(AudioParams {
                    fades: Default::default(),
                    volume: 0.0,
                    muted: false,
                    source: Default::default(),
                }),
            },
        },
        Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(2) },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: range(0, 10),
                start: Tick(0),
                content: ItemParams::Audio(AudioParams {
                    fades: Default::default(),
                    volume: f32::NAN,
                    muted: false,
                    source: Default::default(),
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
            Command::BookmarkAdd {
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
    data.scene_mut().timeline.overlays.clear();
    data.assets.push(
        SourceAsset::try_from(AssetData {
            ephemeral: false,
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
        word_ranges: Vec::new(),
        style: None,
    };
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(1),
            source_asset: Some(AssetId(1)),
            source_range: range(100, 200),
            start: Tick(1000),
            content: ItemParams::Caption(CaptionParams {
                cues: CueTrack {
                    styles: Vec::new(),
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
            s.apply(Command::BookmarkAdd {
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
                placement: engine::TrackPlacement::Auto,
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
                    .scene()
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

#[test]
#[allow(clippy::panic, reason = "validated fixture assertions")]
fn shared_fields_are_atomic_and_history_restores_transient_selection() {
    use engine::ItemParamPatch;
    use std::collections::BTreeSet;
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = TrackKind::Text;
    for (item, size) in track.items.iter_mut().zip([20.0, 30.0]) {
        item.content = ItemParams::Text(project::TextParams {
            content: format!("item {}", item.id.0).into(),
            font_size: size,
            color: Color([10, 20, 30, 255]),
            ..Default::default()
        });
    }
    let mut s = Session::new(Project::new(data).unwrap(), 2).unwrap();
    let before = s.project().clone();
    let bytes = serde_json::to_vec(s.project().data()).unwrap();
    let both: BTreeSet<_> = [ItemId(10), ItemId(11)].into_iter().collect();
    s.set_selection(both.clone()).unwrap();
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), bytes);
    assert_eq!(s.history_len(), (0, 0));
    assert!(
        s.set_selection([ItemId(999)].into_iter().collect())
            .is_err()
    );
    assert_eq!(s.selection(), &both);
    s.execute(
        "timeline.set_item_param_many",
        json!({"items":[10,11],"patch":{"field":"text_size","value":72}}),
    )
    .unwrap();
    let after = s.project().clone();
    assert_eq!(s.history_len(), (1, 0));
    for (id, size) in [(10, 20.0), (11, 30.0)] {
        let ItemParams::Text(p) = &get(&s, id).content else {
            panic!("text fixture")
        };
        assert_eq!(p.font_size, 72.0);
        assert_eq!(p.content.as_str(), format!("item {id}"));
        assert_eq!(p.color, Color([10, 20, 30, 255]));
        let ItemParams::Text(p) = &before
            .data()
            .scene()
            .timeline
            .overlays
            .first()
            .unwrap()
            .items
            .iter()
            .find(|i| i.id == ItemId(id))
            .unwrap()
            .content
        else {
            panic!("text fixture")
        };
        assert_eq!(p.font_size, size);
    }
    s.set_selection(BTreeSet::new()).unwrap();
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    assert_eq!(s.selection(), &both);
    s.redo().unwrap();
    assert_eq!(s.project(), &after);
    assert_eq!(s.selection(), &both);
    for (items, patch) in [
        (vec![ItemId(10), ItemId(10)], ItemParamPatch::TextSize(10.0)),
        (
            vec![ItemId(10), ItemId(999)],
            ItemParamPatch::TextSize(10.0),
        ),
        (vec![], ItemParamPatch::TextSize(10.0)),
        (vec![ItemId(10)], ItemParamPatch::TextSize(f32::NAN)),
        (vec![ItemId(10)], ItemParamPatch::AudioGain(25.0)),
        (vec![ItemId(10)], ItemParamPatch::Opacity(f64::NAN)),
        (vec![ItemId(10)], ItemParamPatch::Opacity(f64::INFINITY)),
        (vec![ItemId(10)], ItemParamPatch::Opacity(f64::NEG_INFINITY)),
    ] {
        assert!(s.apply(Command::SetItemParamMany { items, patch }).is_err());
        assert_eq!(s.project(), &after);
        assert_eq!(s.history_len(), (1, 0));
    }
    // Incompatible kinds are intentionally skipped, including locked incompatible tracks.
    assert!(
        !s.apply(Command::SetItemParamMany {
            items: vec![ItemId(10), ItemId(11)],
            patch: ItemParamPatch::AudioGain(-6.0)
        })
        .unwrap()
        .changed
    );
    s.set_selection(both.clone()).unwrap();
    s.apply(Command::SetItemParam {
        item: ItemId(10),
        content: ItemParams::Text(project::TextParams {
            content: "independent".into(),
            font_size: 18.0,
            color: Color([1, 2, 3, 255]),
            ..Default::default()
        }),
    })
    .unwrap();
    let single = s.project().clone();
    s.set_selection(BTreeSet::new()).unwrap();
    s.undo().unwrap();
    assert_eq!(s.selection(), &both);
    s.redo().unwrap();
    assert_eq!(s.project(), &single);
    s.set_selection(both.clone()).unwrap();
    s.begin_gesture().unwrap();
    assert!(matches!(
        s.set_selection(BTreeSet::new()),
        Err(EditError::TransactionActive)
    ));
    s.apply(Command::SetItemParamMany {
        items: vec![ItemId(10)],
        patch: ItemParamPatch::TextSize(44.0),
    })
    .unwrap();
    s.cancel_gesture().unwrap();
    assert_eq!(s.project(), &single);
    assert_eq!(s.selection(), &both);
    s.apply(Command::Delete {
        items: vec![ItemId(10)],
    })
    .unwrap();
    assert_eq!(s.selection(), &[ItemId(11)].into_iter().collect());
    assert_eq!(s.history_len(), (2, 0));
    s.undo().unwrap();
    assert_eq!(s.selection(), &both);
    s.undo().unwrap();
    assert!(!s.can_undo());
}

#[test]
#[allow(clippy::panic, reason = "validated fixture assertions")]
fn text_style_patches_update_one_field_and_validate() {
    use engine::ItemParamPatch;
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = TrackKind::Text;
    for item in &mut track.items {
        item.content = ItemParams::Text(project::TextParams {
            content: "Keep".into(),
            font_size: 24.0,
            color: Color([1, 2, 3, 255]),
            ..Default::default()
        });
    }
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let item = ItemId(10);
    for patch in [
        ItemParamPatch::TextContent("Xin chào".into()),
        ItemParamPatch::TextFont("Noto Serif".into()),
        ItemParamPatch::TextWeight(700),
        ItemParamPatch::TextItalic(true),
        ItemParamPatch::TextLetterSpacing(-2.5),
        ItemParamPatch::TextLineHeight(60.0),
        ItemParamPatch::TextBackground(Some(project::TextBackground {
            color: Color([9, 8, 7, 128]),
            padding_x: 6.0,
            padding_y: 5.0,
            offset_x: -2.0,
            offset_y: 3.0,
            radius: 3.0,
        })),
        ItemParamPatch::TextBackgroundColor(Color([7, 6, 5, 128])),
        ItemParamPatch::TextBackgroundPaddingX(14.0),
        ItemParamPatch::TextBackgroundPaddingY(9.0),
        ItemParamPatch::TextBackgroundOffsetX(-30.5),
        ItemParamPatch::TextBackgroundOffsetY(12.0),
        ItemParamPatch::TextBackgroundCornerRadius(20.0),
        ItemParamPatch::TextAlign(project::TextAlign::Right),
        ItemParamPatch::TextDecoration(project::TextDecoration::LineThrough),
        ItemParamPatch::TextPositionX(25.0),
        ItemParamPatch::TextPositionY(30.0),
    ] {
        let before = s.project().clone();
        assert!(
            s.apply(Command::SetItemParamMany {
                items: vec![item],
                patch: patch.clone()
            })
            .unwrap()
            .changed
        );
        // Every key is one undo step and redo restores it exactly.
        let edited = s.project().clone();
        s.undo().unwrap();
        assert_eq!(s.project(), &before, "{patch:?} undo");
        s.redo().unwrap();
        assert_eq!(s.project(), &edited, "{patch:?} redo");
    }
    let ItemParams::Text(p) = &get(&s, item.0).content else {
        panic!("text fixture")
    };
    assert_eq!(p.content.as_str(), "Xin chào");
    assert_eq!(p.font.as_str(), "Noto Serif");
    assert_eq!(
        (p.weight, p.italic, p.letter_spacing, p.line_height),
        (700, true, -2.5, 60.0)
    );
    assert_eq!(p.font_size, 24.0);
    assert_eq!(p.color, Color([1, 2, 3, 255]));
    assert_eq!(p.background.as_ref().unwrap().color, Color([7, 6, 5, 128]));
    let background = p.background.as_ref().unwrap();
    assert_eq!(
        (
            background.padding_x,
            background.padding_y,
            background.radius
        ),
        (14.0, 9.0, 20.0)
    );
    assert_eq!((background.offset_x, background.offset_y), (-30.5, 12.0));
    assert_eq!(
        (p.align, p.decoration),
        (
            project::TextAlign::Right,
            project::TextDecoration::LineThrough
        )
    );
    // The patched style serialises with the stable field names the file format and CLI use.
    let json = serde_json::to_value(&get(&s, item.0).content).unwrap();
    assert_eq!(json["params"]["align"], "right");
    assert_eq!(json["params"]["decoration"], "line_through");
    assert_eq!(json["params"]["background"]["offset_x"], -30.5);
    assert_eq!(p.position, project::TextPosition { x: 25.0, y: 30.0 });

    let after = s.project().clone();
    for patch in [
        ItemParamPatch::TextFont("".into()),
        ItemParamPatch::TextWeight(0),
        ItemParamPatch::TextWeight(1000),
        ItemParamPatch::TextLetterSpacing(f32::NAN),
        ItemParamPatch::TextLetterSpacing(project::MAX_LETTER_SPACING + 1.0),
        ItemParamPatch::TextLineHeight(-1.0),
        ItemParamPatch::TextLineHeight(f32::INFINITY),
        ItemParamPatch::TextPositionX(101.0),
        ItemParamPatch::TextPositionY(f32::NAN),
        ItemParamPatch::TextBackgroundPaddingX(-1.0),
        ItemParamPatch::TextBackgroundPaddingY(129.0),
        ItemParamPatch::TextBackgroundCornerRadius(f32::NAN),
        ItemParamPatch::TextBackgroundOffsetX(project::TextBackground::MAX_OFFSET + 1.0),
        ItemParamPatch::TextBackgroundOffsetY(f32::INFINITY),
        ItemParamPatch::TextBackground(Some(project::TextBackground {
            color: Color([0; 4]),
            padding_x: 129.0,
            padding_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            radius: 0.0,
        })),
    ] {
        assert!(
            s.apply(Command::SetItemParamMany {
                items: vec![item],
                patch
            })
            .is_err()
        );
        assert_eq!(s.project(), &after);
    }
}

#[test]
fn text_patch_expansion_is_bounded_and_atomic() {
    for selections in [vec![4], vec![5], vec![3, 3]] {
        let mut data = project().into_data();
        let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
        track.kind = TrackKind::Text;
        let template = track.items.first().unwrap().clone();
        track.items = (0..6)
            .map(|index| Item {
                hidden: false,
                id: ItemId(100 + index),
                start: Tick(index as i64 * 1000),
                content: ItemParams::Text(project::TextParams::new("Keep")),
                ..template.clone()
            })
            .collect();
        let mut offset = 0;
        let mut commands: Vec<_> = selections
            .iter()
            .map(|count| {
                let items = track
                    .items
                    .iter()
                    .skip(offset)
                    .take(*count)
                    .map(|item| item.id)
                    .collect();
                offset += count;
                Command::SetItemParamMany {
                    items,
                    patch: engine::ItemParamPatch::TextContent("x".repeat(1 << 20).into()),
                }
            })
            .collect();
        let command = if commands.len() == 1 {
            commands.pop().unwrap()
        } else {
            Command::Batch {
                commands: vec![Command::Batch { commands }],
            }
        };
        let mut session = Session::new(Project::new(data).unwrap(), 20).unwrap();
        let before = session.project().clone();
        let result = session.apply(command);
        if selections == [4] {
            assert!(result.unwrap().changed);
            assert_eq!(session.history_len(), (1, 0));
            session.undo().unwrap();
        } else {
            assert!(matches!(result, Err(EditError::Limit)));
            assert_eq!(session.history_len(), (0, 0));
        }
        assert_eq!(session.project(), &before);
    }
}

#[test]
fn shared_field_refuses_locked_compatible_target_without_partial_edit() {
    use engine::ItemParamPatch;
    let mut data = project().into_data();
    let source = data.scene_mut().timeline.overlays.first_mut().unwrap();
    source.kind = TrackKind::Text;
    for item in &mut source.items {
        item.content = ItemParams::Text(project::TextParams {
            content: "Keep".into(),
            font_size: 24.0,
            color: Color([1, 2, 3, 255]),
            ..Default::default()
        });
    }
    let second = source.items.pop().unwrap();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(99),
        name: "Locked".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: true,
        items: vec![second],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 2).unwrap();
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::SetItemParamMany {
            items: vec![ItemId(10), ItemId(11)],
            patch: ItemParamPatch::TextSize(80.0)
        }),
        Err(EditError::Locked(TrackId(99)))
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), (0, 0));
    assert!(
        !s.apply(Command::SetItemParamMany {
            items: vec![ItemId(10), ItemId(11)],
            patch: ItemParamPatch::AudioGain(-6.0)
        })
        .unwrap()
        .changed
    );
}

#[test]
#[allow(clippy::indexing_slicing, clippy::panic)]
fn placement_is_atomic_and_respects_groups_locks_and_main_magnet() {
    use engine::{Placement, TrackPlacement};
    let mut s = Session::new(
        Project::empty(ProjectId(9), "Placement".into()).unwrap(),
        100,
    )
    .unwrap();
    for (at, duration) in [(50, 10), (100, 20), (5, 4)] {
        let outcome = s
            .execute(
                "timeline.insert",
                json!({"placement":{"mode":"auto"},"item":new_item(at,duration)}),
            )
            .unwrap();
        assert_eq!(outcome.placements.len(), 1);
        assert_eq!(outcome.placements[0].track, TrackId(0));
    }
    assert!(s.project().data().scene().timeline.overlays.is_empty());
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .iter()
            .map(|i| i.start.0)
            .collect::<Vec<_>>(),
        vec![0, 4, 14]
    );
    let magnetic = s.project().clone();
    s.apply(Command::SetTrackMagnet { enabled: false }).unwrap();
    // Occupied main range falls back to a secondary track; next free range reuses it.
    let a = s
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: new_item(5, 10),
        })
        .unwrap();
    let overlay = a.placements[0].track;
    assert_ne!(overlay, TrackId(0));
    let b = s
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: new_item(15, 10),
        })
        .unwrap();
    assert_eq!(b.placements[0].track, overlay);
    let before = s.project().clone();
    let history = s.history_len();
    for placement in [
        Placement::Below { track: TrackId(0) },
        Placement::Track { track: overlay },
        Placement::Above {
            track: TrackId(999),
        },
    ] {
        assert!(
            s.apply(Command::InsertItem {
                placement,
                item: new_item(5, 10)
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
    assert!(
        s.apply(Command::InsertItem {
            placement: Placement::Above { track: overlay },
            item: new_item(i64::MAX, 10)
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    let result = s
        .apply(Command::InsertItem {
            placement: Placement::Above { track: overlay },
            item: new_item(5, 10),
        })
        .unwrap();
    assert_eq!(result.created.len(), 2);
    let layout = s.project().clone();
    assert_eq!(
        layout.data().scene().timeline.overlays[0].id,
        result.placements[0].track
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    assert_eq!(s.project(), &layout);
    assert_eq!(
        project::decode(&project::encode(&layout, false).unwrap())
            .unwrap()
            .project,
        *layout
    );
    // Explicit secondary creation supports both sides and refuses crossing groups.
    let lower = s
        .apply(Command::AddTrack {
            placement: TrackPlacement::Below { track: overlay },
            name: "Lower".into(),
            kind: TrackKind::Text,
        })
        .unwrap();
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .overlays
            .last()
            .unwrap()
            .id,
        match lower.created[0] {
            CreatedId::Track(id) => id,
            _ => panic!("track"),
        }
    );
    s.apply(Command::SetTrackFlags {
        track: TrackId(0),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    let result = s
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: new_item(100, 10),
        })
        .unwrap();
    assert_ne!(result.placements[0].track, TrackId(0));
    assert!(
        s.apply(Command::InsertItem {
            placement: Placement::Track { track: TrackId(0) },
            item: new_item(100, 10)
        })
        .is_err()
    );
    let ids: Vec<_> = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .map(|t| t.id)
        .collect();
    for track in ids {
        s.apply(Command::SetTrackFlags {
            track,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    }
    assert!(
        registry()
            .iter()
            .find(|spec| spec.id == "timeline.insert")
            .unwrap()
            .enabled(&s)
    );
    let tracks = s.project().data().scene().timeline.track_count();
    s.apply(Command::InsertItem {
        placement: Placement::Auto,
        item: new_item(100, 10),
    })
    .unwrap();
    assert_eq!(
        s.project().data().scene().timeline.track_count(),
        tracks + 1
    );
    // Every placement entry point clamps empty main; occupied main keeps normal timing.
    for magnet in [false, true] {
        for (occupied, command, starts) in [
            (
                false,
                Command::InsertItem {
                    placement: Placement::Track { track: TrackId(0) },
                    item: new_item(100, 10),
                },
                vec![0],
            ),
            (
                false,
                Command::MoveItem {
                    item: ItemId(10),
                    track: TrackId(0),
                    start: Tick(100),
                },
                vec![0],
            ),
            (
                false,
                Command::MoveItems {
                    moves: vec![
                        Move {
                            item: ItemId(10),
                            track: TrackId(0),
                            start: Tick(100),
                        },
                        Move {
                            item: ItemId(11),
                            track: TrackId(0),
                            start: Tick(160),
                        },
                    ],
                },
                vec![0, 60],
            ),
            (
                true,
                Command::InsertItem {
                    placement: Placement::Track { track: TrackId(0) },
                    item: new_item(100, 10),
                },
                vec![0, 100],
            ),
            (
                true,
                Command::MoveItem {
                    item: ItemId(10),
                    track: TrackId(0),
                    start: Tick(100),
                },
                vec![0, 100],
            ),
            (
                true,
                Command::MoveItems {
                    moves: vec![
                        Move {
                            item: ItemId(10),
                            track: TrackId(0),
                            start: Tick(100),
                        },
                        Move {
                            item: ItemId(11),
                            track: TrackId(0),
                            start: Tick(160),
                        },
                    ],
                },
                vec![0, 100, 160],
            ),
        ] {
            let mut placed = session();
            placed
                .apply(Command::SetTrackMagnet { enabled: magnet })
                .unwrap();
            if occupied {
                placed
                    .apply(Command::InsertItem {
                        placement: Placement::Track { track: TrackId(0) },
                        item: new_item(0, 10),
                    })
                    .unwrap();
            }
            let before = placed.project().clone();
            let history = placed.history_len();
            placed.apply(command).unwrap();
            let after = placed.project().clone();
            assert_eq!(
                after
                    .data()
                    .scene()
                    .timeline
                    .main
                    .items
                    .iter()
                    .map(|item| item.start.0)
                    .collect::<Vec<_>>(),
                starts
            );
            assert_eq!(placed.history_len().0, history.0 + 1);
            placed.undo().unwrap();
            assert_eq!(placed.project(), &before);
            placed.redo().unwrap();
            assert_eq!(placed.project(), &after);
        }
    }
    assert!(magnetic.data().scene().timeline.magnet);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, rng_seed: proptest::test_runner::RngSeed::Fixed(14002021), ..ProptestConfig::default() })]
    #[test]
    fn random_placements_preserve_layout_and_history(operations in prop::collection::vec((0u8..5,0i64..300,1i64..30,any::<bool>()),1..60)) {
        use engine::Placement;
        let mut s = Session::new(Project::empty(ProjectId(9), "Placement".into()).unwrap(), 1000).unwrap();
        let mut snapshots = vec![s.project().clone()];
        for (mode, at, duration, text) in operations {
            if s.apply(Command::SetTrackMagnet { enabled: mode % 2 == 0 }).unwrap().changed { snapshots.push(s.project().clone()); }
            let placement = match mode {
                0 => Placement::Above {track: TrackId(0)},
                1 => Placement::Below {track: TrackId(0)},
                2 => Placement::Track {track: TrackId(0)},
                _ => Placement::Auto,
            };
            let mut item = new_item(at,duration);
            if text { item.content = ItemParams::Text(project::TextParams {content:"Tiếng Việt".into(),font_size:24.0,color:Color([255;4]),..Default::default()}); }
            let before = s.project().clone();
            match s.apply(Command::InsertItem {placement,item}) {
                Ok(outcome) => {
                    prop_assert!(outcome.changed);
                    prop_assert_eq!(outcome.placements.len(),1);
                    snapshots.push(s.project().clone());
                }
                Err(_) => prop_assert_eq!(s.project(), &before),
            }
            let timeline = &s.project().data().scene().timeline;
            for track in timeline.iter() {
                let mut end=Tick::ZERO;
                for item in &track.items {
                    prop_assert!(item.start>=end);
                    prop_assert_eq!(item.content.kind(),track.kind);
                    end=item.end().unwrap();
                }
            }
            if let Some(first)=timeline.main.items.first(){ prop_assert_eq!(first.start,Tick::ZERO); }
        }
        let final_layout = s.project().clone();
        for before in snapshots.iter().rev().skip(1) { s.undo().unwrap(); prop_assert_eq!(s.project(),before); }
        for after in snapshots.iter().skip(1) { s.redo().unwrap(); prop_assert_eq!(s.project(),after); }
        prop_assert_eq!(s.project(),&final_layout);
        prop_assert_eq!(&project::decode(&project::encode(&final_layout, false).unwrap()).unwrap().project,final_layout.as_ref());
    }

}

#[test]
fn imported_asset_is_one_undo_step_and_restores_the_exact_record() {
    let mut s = Session::new(Project::empty(ProjectId(9), "Assets".into()).unwrap(), 8).unwrap();
    let asset = SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(3),
        path: std::env::temp_dir().join("import record.mp4"),
        content_hash: ContentHash([3; 32]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(Tick(2_000)),
            streams: vec![],
        },
    })
    .unwrap();
    s.apply(Command::AddAsset {
        asset: asset.clone(),
    })
    .unwrap();
    assert_eq!(s.project().data().assets, vec![asset.clone()]);
    s.execute("undo", json!({})).unwrap();
    assert!(s.project().data().assets.is_empty());
    s.execute("redo", json!({})).unwrap();
    assert_eq!(s.project().data().assets, vec![asset]);
}

fn policy_asset(id: u64, format: &str, rate: Option<time::FrameRate>) -> SourceAsset {
    policy_asset_sized(id, format, rate, (640, 360))
}
fn policy_asset_sized(
    id: u64,
    format: &str,
    rate: Option<time::FrameRate>,
    (width, height): (u32, u32),
) -> SourceAsset {
    let stream = testkit::source::video()
        .codec("synthetic")
        .duration(Tick(1000))
        .size(width, height);
    let stream = match rate {
        Some(rate) => stream.frame_rate(rate),
        None => stream.no_frame_rate(),
    };
    testkit::source::asset(id, std::env::temp_dir().join("policy source"))
        .ephemeral(true)
        .format(format)
        .duration(Tick(1000))
        .stream(stream.build())
        .build()
}
fn policy_insert(id: u64) -> Command {
    Command::InsertItem {
        placement: Default::default(),
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: Some(AssetId(id)),
            source_range: range(0, 1000),
            start: Tick::ZERO,
            content: ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
                crop: None,
            }),
        },
    }
}
#[test]
fn import_policy_ratchets_exact_rates_and_refuses_invalid_metadata_atomically() {
    use time::FrameRate as F;
    for (initial, incoming, format, expected) in [
        (F::FPS_30, Some(F::FPS_60), "mp4", F::FPS_60),
        (F::FPS_60, Some(F::FPS_30), "mp4", F::FPS_60),
        (F::FPS_29_97, Some(F::FPS_30), "mp4", F::FPS_30),
        (F::FPS_30, Some(F::FPS_29_97), "mp4", F::FPS_30),
        (F::FPS_23_976, Some(F::FPS_29_97), "mp4", F::FPS_29_97),
        (F::FPS_30, Some(F::FPS_60), "png_pipe", F::FPS_30),
        (F::FPS_30, Some(F::FPS_60), "gif", F::FPS_30),
        (F::FPS_30, Some(F::FPS_60), "apng", F::FPS_30),
        (F::FPS_30, None, "mp4", F::FPS_30),
    ] {
        let mut data = Project::empty(ProjectId(1), "policy".into())
            .unwrap()
            .into_data();
        data.canvas.fps = initial;
        let before = Project::new(data).unwrap();
        let mut s = Session::new(before.clone(), 10).unwrap();
        s.execute(
            "import.add_asset",
            json!({"asset":policy_asset(1,format,incoming)}),
        )
        .unwrap();
        assert_eq!(s.project().data().canvas.fps, expected);
        s.execute("undo", json!({})).unwrap();
        assert_eq!(s.project().as_ref(), &before);
        s.execute("redo", json!({})).unwrap();
        assert_eq!(s.project().data().canvas.fps, expected);
        let frozen = s.project().clone();
        for rate in [
            json!({"num":0,"den":1}),
            json!({"num":30,"den":0}),
            json!({"num":-30,"den":1}),
            json!({"num":29.97,"den":1}),
        ] {
            let mut asset = serde_json::to_value(policy_asset(2, "mp4", None)).unwrap();
            asset["probe"]["streams"][0]["frame_rate"] = rate;
            assert!(
                s.execute("import.add_asset", json!({"asset":asset}))
                    .is_err()
            );
            assert_eq!(s.project(), &frozen);
        }
        // A valid probe rational without exact canvas ticks is refused, never rounded.
        assert!(
            s.apply(Command::AddAsset {
                asset: policy_asset(2, "mp4", Some(F::new(1001, 1).unwrap()))
            })
            .is_err()
        );
        assert_eq!(s.project(), &frozen);
    }
    let mut audio = AssetData::from(policy_asset(1, "wav", Some(F::FPS_120)));
    audio.probe.streams.first_mut().unwrap().kind = media::StreamKind::Audio;
    audio.probe.streams.first_mut().unwrap().video = None;
    audio.probe.streams.first_mut().unwrap().audio = Some(media::AudioInfo {
        sample_rate: 48000,
        channels: 2,
        initial_padding: 0,
        trailing_padding: 0,
    });
    let mut s = Session::new(Project::empty(ProjectId(1), "".into()).unwrap(), 10).unwrap();
    s.apply(Command::AddAsset {
        asset: SourceAsset::try_from(audio).unwrap(),
    })
    .unwrap();
    assert_eq!(s.project().data().canvas.fps, F::FPS_30);
}
#[test]
fn first_media_canvas_is_atomic_persisted_and_batch_ordered() {
    use time::FrameRate as F;
    // Odd captured media sizes resolve the Original preset to the nearest even
    // size (ties up), never zero, in one undo step.
    for ((width, height), even) in [
        ((641, 361), (642, 362)),
        ((1, 1), (2, 2)),
        ((8191, 8191), (8192, 8192)),
    ] {
        let mut data = Project::empty(ProjectId(1), "odd".into())
            .unwrap()
            .into_data();
        data.assets.push(policy_asset_sized(
            1,
            "mp4",
            Some(F::FPS_30),
            (width, height),
        ));
        let before = Project::new(data).unwrap();
        let mut s = Session::new(before.clone(), 10).unwrap();
        s.apply(policy_insert(1)).unwrap();
        let size = project::CanvasSize {
            width: even.0,
            height: even.1,
        };
        let data = s.project().data();
        assert_eq!((data.canvas.width, data.canvas.height), even);
        assert_eq!(data.original_canvas_size, Some(size));
        assert_eq!(data.canvas.preset, Some(project::AspectPreset::Original));
        s.execute("undo", json!({})).unwrap();
        assert_eq!(s.project().as_ref(), &before);
    }
    for format in ["mp4", "png_pipe"] {
        let mut data = Project::empty(ProjectId(1), "policy".into())
            .unwrap()
            .into_data();
        data.assets.push(policy_asset(1, format, Some(F::FPS_60)));
        let before = Project::new(data).unwrap();
        let mut s = Session::new(before.clone(), 10).unwrap();
        s.apply(policy_insert(1)).unwrap();
        let after = s.project().clone();
        assert_eq!(
            (after.data().canvas.width, after.data().canvas.height),
            (640, 360)
        );
        assert_eq!(
            after.data().original_canvas_size,
            Some(project::CanvasSize {
                width: 640,
                height: 360
            })
        );
        assert_eq!(
            after.data().canvas.preset,
            Some(project::AspectPreset::Original)
        );
        assert_eq!(
            after.data().canvas.fps,
            if format == "mp4" {
                F::FPS_60
            } else {
                F::FPS_30
            }
        );
        let loaded = project::decode(&project::encode(&after, false).unwrap()).unwrap();
        assert_eq!(&loaded.project, after.as_ref());
        assert!(loaded.project.data().assets.first().unwrap().ephemeral());
        s.execute("undo", json!({})).unwrap();
        assert_eq!(s.project().as_ref(), &before);
        s.execute("redo", json!({})).unwrap();
        assert_eq!(s.project(), &after);
    }
    let mut data = Project::empty(ProjectId(1), "policy".into())
        .unwrap()
        .into_data();
    data.assets.push(policy_asset(1, "mp4", Some(F::FPS_60)));
    let mut second = AssetData::from(policy_asset(2, "mp4", Some(F::FPS_120)));
    second
        .probe
        .streams
        .first_mut()
        .unwrap()
        .video
        .as_mut()
        .unwrap()
        .width = 1280;
    data.assets.push(SourceAsset::try_from(second).unwrap());
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = s.project().as_ref().clone();
    // A later failure must roll back first-insert canvas/FPS as well as items.
    assert!(
        s.apply(Command::Batch {
            commands: vec![policy_insert(2), policy_insert(999)]
        })
        .is_err()
    );
    assert_eq!(s.project().as_ref(), &before);
    s.execute("begin_gesture", json!({})).unwrap();
    s.apply(policy_insert(2)).unwrap();
    s.execute("cancel_gesture", json!({})).unwrap();
    assert_eq!(s.project().as_ref(), &before);
    s.apply(Command::Batch {
        commands: vec![policy_insert(2), policy_insert(1)],
    })
    .unwrap();
    assert_eq!(s.project().data().canvas.width, 1280);
    assert_eq!(s.project().data().canvas.fps, F::FPS_120);
    let ids = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter().map(|item| item.id))
        .collect();
    s.apply(Command::Delete { items: ids }).unwrap();
    s.apply(policy_insert(1)).unwrap();
    assert_eq!(s.project().data().canvas.width, 1280);
    assert_eq!(
        s.project()
            .data()
            .original_canvas_size
            .as_ref()
            .unwrap()
            .width,
        1280
    );
    s.execute("undo", json!({})).unwrap();
    s.execute("undo", json!({})).unwrap();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.project().as_ref(), &before);
    // Any existing timeline item prevents canvas initialization.
    s.apply(Command::InsertItem {
        placement: Default::default(),
        item: new_item(0, 10),
    })
    .unwrap();
    s.apply(policy_insert(1)).unwrap();
    assert_eq!(s.project().data().canvas.width, 1920);
    assert_eq!(s.project().data().original_canvas_size, None);
}

#[test]
fn canvas_settings_commit_one_step_and_keep_exact_rationals() {
    let original = project::CanvasSize {
        width: 1080,
        height: 1920,
    };
    let mut data = Project::empty(ProjectId(1), "Canvas".into())
        .unwrap()
        .into_data();
    data.original_canvas_size = Some(original);
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let initial = s.project().clone();
    let mut steps = 0;
    let mut commit = |s: &mut Session, command: &str, params: serde_json::Value| {
        s.execute(command, params).unwrap();
        steps += 1;
        assert_eq!(s.history_len().0, steps);
    };

    commit(
        &mut s,
        "project.set_canvas",
        json!({"width":1080,"height":1920,"preset":"9:16"}),
    );
    assert_eq!(
        (
            s.project().data().canvas.width,
            s.project().data().canvas.height
        ),
        (1080, 1920)
    );
    assert_eq!(
        s.project().data().canvas.preset,
        Some(project::AspectPreset::Portrait)
    );

    commit(
        &mut s,
        "project.set_canvas",
        json!({"width":1000,"height":700}),
    );
    assert_eq!(s.project().data().canvas.preset, None);
    assert_eq!(
        s.project().data().canvas.last_custom_size,
        project::CanvasSize {
            width: 1000,
            height: 700
        }
    );

    commit(
        &mut s,
        "project.set_canvas",
        json!({"width":1081,"height":721}),
    );
    assert_eq!(
        (
            s.project().data().canvas.width,
            s.project().data().canvas.height
        ),
        (1082, 722)
    );
    assert_eq!(
        s.project().data().canvas.last_custom_size,
        project::CanvasSize {
            width: 1082,
            height: 722
        }
    );
    commit(&mut s, "project.set_fps", json!({"num":48,"den":2}));
    assert_eq!(s.project().data().canvas.fps, time::FrameRate::FPS_24);
    commit(&mut s, "project.set_fps", json!({"num":24000,"den":1001}));
    assert_eq!(s.project().data().canvas.fps, time::FrameRate::FPS_23_976);
    assert_eq!(s.project().data().canvas.fps.label(), "23.976");
    commit(&mut s, "project.set_fps", json!({"num":30000,"den":1001}));
    assert_eq!(s.project().data().canvas.fps, time::FrameRate::FPS_29_97);
    assert_eq!(s.project().data().canvas.fps.label(), "29.97");

    commit(
        &mut s,
        "project.set_background",
        json!({"color":[12,34,56,78]}),
    );
    assert_eq!(
        s.project().data().canvas.background,
        Color([12, 34, 56, 78])
    );

    commit(
        &mut s,
        "project.set_canvas",
        json!({"width":1080,"height":1920,"preset":"original"}),
    );
    assert_eq!(
        s.project().data().canvas.preset,
        Some(project::AspectPreset::Original)
    );
    assert_eq!(
        (
            s.project().data().canvas.width,
            s.project().data().canvas.height
        ),
        (original.width, original.height)
    );

    let saved = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&saved).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
    for _ in 0..steps {
        s.undo().unwrap();
    }
    assert_eq!(s.project(), &initial);
}
#[test]
fn canvas_custom_sizes_commit_the_nearest_even_size() {
    // (requested, committed): odd axes take the nearest even value (ties up),
    // one never becomes zero, and the largest odd square stays inside the bound.
    for (requested, committed) in [
        ((1081, 721), (1082, 722)),
        ((1, 1), (2, 2)),
        ((1081, 1), (1082, 2)),
        ((8191, 8191), (8192, 8192)),
    ] {
        let mut s =
            Session::new(Project::empty(ProjectId(1), "Canvas".into()).unwrap(), 10).unwrap();
        let initial = s.project().clone();
        s.execute(
            "project.set_canvas",
            json!({"width":requested.0,"height":requested.1}),
        )
        .unwrap();
        assert_eq!(
            (
                s.project().data().canvas.width,
                s.project().data().canvas.height
            ),
            committed,
            "{requested:?}"
        );
        assert_eq!(s.project().data().canvas.preset, None);
        assert_eq!(
            s.project().data().canvas.last_custom_size,
            project::CanvasSize {
                width: committed.0,
                height: committed.1
            },
            "{requested:?}"
        );
        assert_eq!(s.history_len().0, 1, "{requested:?}");
        s.undo().unwrap();
        assert_eq!(s.project(), &initial, "{requested:?}");
    }
}
#[test]
fn canvas_settings_reject_invalid_sizes_rates_and_preset_mismatches() {
    let mut data = Project::empty(ProjectId(1), "Canvas".into())
        .unwrap()
        .into_data();
    data.original_canvas_size = Some(project::CanvasSize {
        width: 1920,
        height: 1080,
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = s.project().clone();
    for params in [
        json!({"width":0,"height":1080}),
        json!({"width":1920,"height":0}),
        json!({"width":-1,"height":1080}),
        json!({"width":67_108_865,"height":1}),
        json!({"width":67_108_863,"height":1}),
        json!({"width":8193,"height":8193}),
        json!({"width":u32::MAX,"height":1}),
        json!({"width":1,"height":u32::MAX}),
        json!({"width":u32::MAX,"height":u32::MAX}),
        json!({"width":1080,"height":1920,"preset":"16:9"}),
        json!({"width":1080,"height":1920,"preset":"original"}),
    ] {
        assert!(s.execute("project.set_canvas", params).is_err());
        assert_eq!(s.project(), &before);
    }
    for params in [
        json!({"num":0,"den":1}),
        json!({"num":30,"den":0}),
        json!({"num":-30,"den":1}),
        json!({"num":17,"den":1}),
    ] {
        assert!(s.execute("project.set_fps", params).is_err());
        assert_eq!(s.project(), &before);
    }
}
#[test]
fn track_rename_reorder_lock_and_add_persist_and_undo_exactly() {
    let mut s = Session::new(Project::empty(ProjectId(31), "Tracks".into()).unwrap(), 20).unwrap();
    let CreatedId::Track(a) = *s
        .apply(Command::AddTrack {
            name: "First".into(),
            kind: TrackKind::Video,
            placement: Default::default(),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("track id")
    };
    let outcome = s
        .execute(
            "timeline.add_track",
            json!({"name":"Second","kind":"video"}),
        )
        .unwrap();
    let CreatedId::Track(b) = outcome.created[0] else {
        panic!("track id")
    };
    assert_eq!(s.project().data().scene().timeline.overlays.len(), 2);
    assert_eq!(s.project().data().scene().timeline.overlays[0].id, b);

    let original_main = s.project().data().scene().timeline.main.clone();
    s.execute(
        "track.rename",
        json!({"track":original_main.id,"name":"Renamed main"}),
    )
    .unwrap();
    s.execute("track.rename", json!({"track":a,"name":"Rãnh đã đổi"}))
        .unwrap();
    s.execute("track.reorder", json!({"track":b,"index":1}))
        .unwrap();
    s.execute("track.set_locked", json!({"track":a,"locked":true}))
        .unwrap();
    let changed = s.project().clone();
    let overlays = &changed.data().scene().timeline.overlays;
    assert_eq!(overlays[0].id, a);
    assert_eq!(overlays[1].id, b);
    assert_eq!(overlays[0].name.as_str(), "Rãnh đã đổi");
    assert!(overlays[0].locked);
    assert!(!overlays[1].locked);

    let bytes = project::encode(&changed, false).unwrap();
    let loaded = project::decode(&bytes).unwrap().project;
    assert_eq!(&loaded, changed.as_ref());
    assert_eq!(
        loaded.data().scene().timeline.main.name.as_str(),
        "Renamed main"
    );

    for _ in 0..4 {
        s.undo().unwrap();
    }
    assert_eq!(s.project().data().scene().timeline.main, original_main);
    let original = s.project().data().scene().timeline.overlays.clone();
    assert_eq!(original[0].id, b);
    assert_eq!(original[0].name.as_str(), "Second");
    assert_eq!(original[1].name.as_str(), "First");
    assert!(original.iter().all(|track| !track.locked));
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(s.project().data().scene().timeline.overlays.is_empty());
    for _ in 0..6 {
        s.redo().unwrap();
    }
    assert_eq!(s.project(), &changed);
}

#[test]
fn locked_tracks_refuse_structural_item_edits_but_allow_metadata_and_unlock() {
    let mut s = Session::new(Project::empty(ProjectId(32), "Lock".into()).unwrap(), 20).unwrap();
    let outcome = s
        .execute(
            "timeline.add_track",
            json!({"name":"Locked","kind":"video"}),
        )
        .unwrap();
    let CreatedId::Track(track) = outcome.created[0] else {
        panic!("track id")
    };
    let item = new_item(0, 20);
    let outcome = s
        .execute(
            "timeline.insert",
            json!({"placement":{"mode":"track","track":track},"item":item}),
        )
        .unwrap();
    let CreatedId::Item(one) = outcome.created[0] else {
        panic!("item id")
    };
    let outcome = s
        .execute(
            "timeline.insert",
            json!({"placement":{"mode":"track","track":0},"item":new_item(0,20)}),
        )
        .unwrap();
    let CreatedId::Item(other) = outcome.created[0] else {
        panic!("item id")
    };
    s.execute("track.set_locked", json!({"track":track,"locked":true}))
        .unwrap();
    let before = s.project().clone();

    let edits = [
        (
            "timeline.insert",
            json!({"placement":{"mode":"track","track":track},"item":new_item(30,10)}),
        ),
        ("move_item", json!({"item":one,"track":track,"start":5})),
        ("move_item", json!({"item":one,"track":0,"start":30})),
        ("move_item", json!({"item":other,"track":track,"start":30})),
        (
            "move_items",
            json!({"moves":[{"item":one,"track":track,"start":5}]}),
        ),
        ("split_at", json!({"item":one,"at":5})),
        ("trim_start", json!({"item":one,"at":2})),
        ("trim_end", json!({"item":one,"at":10})),
        ("delete", json!({"items":[one]})),
        ("ripple_delete", json!({"item":one})),
        ("track.remove", json!({"track":track})),
        (
            "batch",
            json!({"commands":[{"command":"track.rename","params":{"track":0,"name":"Must roll back"}},{"command":"delete","params":{"items":[one]}}]}),
        ),
        (
            "set_item_param",
            json!({"item":one,"content":{"kind":"overlay","params":{"color":[1,2,3,255]}}}),
        ),
    ];
    for (id, params) in edits {
        assert!(
            matches!(
                s.execute(id, params.clone()),
                Err(EditError::Locked(found)) if found == track
            ),
            "{id} did not refuse a locked track: {:?}",
            s.execute(id, params)
        );
        assert_eq!(s.project(), &before, "{id} changed a locked project");
    }

    // Selection, mute and the name remain editable while items are protected.
    s.set_selection([one].into_iter().collect()).unwrap();
    assert_eq!(s.selection().len(), 1);
    s.execute("track.set_muted", json!({"track":track,"muted":true}))
        .unwrap();
    s.execute("track.rename", json!({"track":track,"name":"Still locked"}))
        .unwrap();
    assert!(s.project().data().scene().timeline.overlays[0].locked);
    s.execute("track.set_locked", json!({"track":track,"locked":false}))
        .unwrap();
    s.execute("move_item", json!({"item":one,"track":track,"start":5}))
        .unwrap();
}

#[test]
fn bookmark_move_is_one_undo_step_and_refuses_missing_or_negative_targets() {
    let mut s = Session::new(Project::empty(ProjectId(35), "Marks".into()).unwrap(), 20).unwrap();
    let CreatedId::Bookmark(bookmark) = *s
        .apply(Command::BookmarkAdd {
            start: Tick(10),
            duration: Tick(5),
            note: "Beat".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    let before = s.project().clone();
    s.execute("bookmark.move", json!({"bookmark":bookmark,"start":40}))
        .unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].start, Tick(40));
    assert_eq!(s.history_len().0, 2);
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].start, Tick(40));
    assert!(matches!(
        s.execute("bookmark.move", json!({"bookmark":bookmark,"start":-1})),
        Err(EditError::Invalid("bookmark start"))
    ));
    assert!(matches!(
        s.execute("bookmark.move", json!({"bookmark":999,"start":0})),
        Err(EditError::Invalid("missing bookmark"))
    ));
    assert_eq!(s.project().data().scene().bookmarks[0].start, Tick(40));
}

#[test]
fn bookmark_toggle_snaps_adds_defaults_removes_and_undoes_every_step() {
    let mut data = project().into_data();
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let frame = s.project().data().canvas.fps.tick_of(1).unwrap();

    // One off-frame tick snaps to the nearest frame and adds a default bookmark.
    let at = Tick(frame.0 * 2 + 1);
    let outcome = s.execute("bookmark.toggle", json!({"at":at})).unwrap();
    assert!(outcome.changed);
    let CreatedId::Bookmark(bookmark) = *outcome.created.first().unwrap() else {
        panic!("bookmark id")
    };
    let added = s.project().data().scene().bookmarks[0].clone();
    assert_eq!(added.id, bookmark);
    assert_eq!(added.start, Tick(frame.0 * 2));
    assert_eq!(added.duration, Tick::ZERO);
    assert_eq!(added.note.as_str(), "");
    assert_eq!(added.color, Color([0, 157, 255, 255]));
    assert_eq!(s.history_len(), (1, 0));

    // The same frame toggles it off; undo and redo restore each side exactly.
    let outcome = s.execute("bookmark.toggle", json!({"at":at})).unwrap();
    assert!(outcome.changed);
    assert!(outcome.created.is_empty());
    assert!(s.project().data().scene().bookmarks.is_empty());
    assert_eq!(s.history_len(), (2, 0));
    s.undo().unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0], added);
    s.redo().unwrap();
    assert!(s.project().data().scene().bookmarks.is_empty());

    // Off-frame inputs snap to the nearest frame; an exact tie snaps later.
    for (index, (input, expected)) in [
        (frame.0 * 3 + 1, frame.0 * 3),
        (frame.0 * 4 + frame.0 / 2 + 1, frame.0 * 5),
        (frame.0 * 6 - 1, frame.0 * 6),
        (frame.0 * 7 + frame.0 / 2, frame.0 * 8),
    ]
    .into_iter()
    .enumerate()
    {
        let outcome = s.execute("bookmark.toggle", json!({"at":input})).unwrap();
        let CreatedId::Bookmark(id) = *outcome.created.first().unwrap() else {
            panic!("bookmark id")
        };
        let bookmark = s
            .project()
            .data()
            .scene()
            .bookmarks
            .iter()
            .find(|bookmark| bookmark.id == id)
            .unwrap();
        assert_eq!(bookmark.start, Tick(expected), "row {index}");
        assert_eq!(s.project().data().scene().bookmarks.len(), index + 1);
    }
}

#[test]
fn bookmark_update_and_remove_are_one_undo_step_with_persistence() {
    let mut data = project().into_data();
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let CreatedId::Bookmark(bookmark) = *s
        .apply(Command::BookmarkAdd {
            start: Tick::ZERO,
            duration: Tick::ZERO,
            note: "Beat".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    let mut previous = s.project().data().scene().bookmarks[0].clone();

    // Every field updates alone, and every update is one undo step.
    for (index, (params, note, color, duration)) in [
        (
            json!({"bookmark":bookmark,"note":"Intro"}),
            "Intro",
            Color([1, 2, 3, 255]),
            Tick::ZERO,
        ),
        (
            json!({"bookmark":bookmark,"color":[9,8,7,255]}),
            "Intro",
            Color([9, 8, 7, 255]),
            Tick::ZERO,
        ),
        (
            json!({"bookmark":bookmark,"duration":500}),
            "Intro",
            Color([9, 8, 7, 255]),
            Tick(100),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        s.execute("bookmark.update", params).unwrap();
        let current = s.project().data().scene().bookmarks[0].clone();
        assert_eq!(current.note.as_str(), note, "row {index}");
        assert_eq!(current.color, color, "row {index}");
        assert_eq!(current.duration, duration, "row {index}");
        assert_eq!(s.history_len(), (index + 2, 0), "row {index}");
        s.undo().unwrap();
        assert_eq!(s.project().data().scene().bookmarks[0], previous);
        s.redo().unwrap();
        assert_eq!(s.project().data().scene().bookmarks[0], current);
        previous = current;
    }
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        project::decode(&saved)
            .unwrap()
            .project
            .data()
            .scene()
            .bookmarks,
        s.project().data().scene().bookmarks
    );

    // Removal is one undo step and restores the exact bookmark.
    let before_remove = s.project().clone();
    let history = s.history_len();
    s.execute("bookmark.remove", json!({"bookmark":bookmark}))
        .unwrap();
    assert!(s.project().data().scene().bookmarks.is_empty());
    assert_eq!(s.history_len(), (history.0 + 1, 0));
    s.undo().unwrap();
    assert_eq!(s.project(), &before_remove);
    s.redo().unwrap();
    assert!(s.project().data().scene().bookmarks.is_empty());

    // A later bookmark clamps to the remaining scene content.
    let CreatedId::Bookmark(tail) = *s
        .apply(Command::BookmarkAdd {
            start: Tick(60),
            duration: Tick::ZERO,
            note: "Tail".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    s.execute("bookmark.update", json!({"bookmark":tail,"duration":500}))
        .unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].duration, Tick(40));
}

#[test]
fn bookmark_commands_refuse_hostile_inputs_without_history() {
    let mut data = project().into_data();
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let CreatedId::Bookmark(bookmark) = *s
        .apply(Command::BookmarkAdd {
            start: Tick::ZERO,
            duration: Tick::ZERO,
            note: "Beat".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    let before = s.project().clone();
    let history = s.history_len();
    for (index, (id, params)) in [
        ("bookmark.toggle", json!({"at":-1})),
        ("bookmark.update", json!({"bookmark":999,"note":"x"})),
        (
            "bookmark.update",
            json!({"bookmark":bookmark,"duration":-1}),
        ),
        ("bookmark.update", json!({"bookmark":bookmark})),
        (
            "bookmark.update",
            json!({"bookmark":bookmark,"note":"Beat"}),
        ),
        (
            "bookmark.update",
            json!({"bookmark":bookmark,"color":[256,0,0,255]}),
        ),
        (
            "bookmark.update",
            json!({"bookmark":bookmark,"color":[-1,0,0,255]}),
        ),
        (
            "bookmark.update",
            json!({"bookmark":bookmark,"color":[0,0,0,1.5]}),
        ),
        ("bookmark.remove", json!({"bookmark":999})),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            s.execute(id, params.clone()).is_err(),
            "row {index}: {id} accepted {params}"
        );
        assert_eq!(
            s.project(),
            &before,
            "row {index}: {id} changed the project"
        );
        assert_eq!(s.history_len(), history, "row {index}: {id} added history");
    }

    // The project owner's 1 MiB text limit remains the only note bound.
    let oversized = "n".repeat((1 << 20) + 1);
    assert!(matches!(
        s.execute(
            "bookmark.update",
            json!({"bookmark":bookmark,"note":oversized})
        ),
        Err(EditError::Project(project::Error::TooLarge("text")))
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);

    // A duration beyond the scene end clamps once, then the same request is a no-op.
    s.execute(
        "bookmark.update",
        json!({"bookmark":bookmark,"duration":500}),
    )
    .unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].duration, Tick(100));
    let changed = s.project().clone();
    let changed_history = s.history_len();
    assert!(matches!(
        s.execute(
            "bookmark.update",
            json!({"bookmark":bookmark,"duration":500})
        ),
        Err(EditError::Invalid("bookmark update"))
    ));
    assert_eq!(s.project(), &changed);
    assert_eq!(s.history_len(), changed_history);

    // A locked track cannot hide a scene-owned bookmark: scenes have no lock state.
    let frame = s.project().data().canvas.fps.tick_of(1).unwrap();
    s.execute("track.set_locked", json!({"track":0,"locked":true}))
        .unwrap();
    s.execute("bookmark.toggle", json!({"at":frame.0})).unwrap();
    assert_eq!(s.project().data().scene().bookmarks.len(), 2);
}

#[test]
fn audio_reorder_is_screen_order_and_main_cannot_move() {
    let mut s = Session::new(Project::empty(ProjectId(33), "Audio".into()).unwrap(), 20).unwrap();
    let CreatedId::Track(a) = *s
        .apply(Command::AddTrack {
            name: "A".into(),
            kind: TrackKind::Audio,
            placement: Default::default(),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("track id")
    };
    let CreatedId::Track(b) = *s
        .apply(Command::AddTrack {
            name: "B".into(),
            kind: TrackKind::Audio,
            placement: Default::default(),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("track id")
    };
    assert_eq!(s.project().data().scene().timeline.audio[0].id, b);
    s.execute("track.reorder", json!({"track":a,"index":0}))
        .unwrap();
    assert_eq!(s.project().data().scene().timeline.audio[0].id, a);
    assert_eq!(s.project().data().scene().timeline.audio[1].id, b);
    let main = s.project().data().scene().timeline.main.id;
    assert!(matches!(
        s.execute("track.reorder", json!({"track":main,"index":0})),
        Err(EditError::Invalid("main track cannot be moved"))
    ));
    assert!(matches!(
        s.execute("track.reorder", json!({"track":a,"index":5})),
        Err(EditError::Invalid("track index"))
    ));
    assert!(
        !Project::empty(ProjectId(34), "Fresh".into())
            .unwrap()
            .data()
            .scene()
            .timeline
            .main
            .locked
    );
}

#[test]
fn audio_controls_are_atomic_undoable_persisted_and_strict() {
    let mut data = project().into_data();
    // Reuse the existing synthetic media fixture without filesystem IO.
    let source = testkit::source::asset(7, std::env::temp_dir().join("audio-controls.wav"))
        .format("wav")
        .duration(Tick(1000))
        .content_hash(ContentHash::from_bytes(&[7]))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(Tick(1000))
                .build(),
        )
        .build();
    data.assets.push(source);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(8),
        name: "".into(),
        kind: TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            id: ItemId(20),
            source_asset: Some(AssetId(7)),
            source_range: range(0, 1000),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            retime: Default::default(),
            content: ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    let mut session = Session::new(Project::new(data).unwrap(), 100).unwrap();
    for (command, args) in [
        (
            "timeline.set_item_param",
            json!({"item":20,"patch":{"field":"volume","value":-6.01}}),
        ),
        (
            "timeline.set_item_param",
            json!({"item":20,"patch":{"field":"muted","value":true}}),
        ),
        ("audio.set_master", json!({"volume":-1.25,"muted":true})),
        (
            "audio.set_loudness",
            json!({"scope":"clip","target_lufs":-16.0,"true_peak_db":-2.0}),
        ),
        (
            "audio.set_loudness",
            json!({"scope":"master","target_lufs":-14.0,"true_peak_db":-1.0}),
        ),
        (
            "audio.apply_loudness",
            json!({"gains":{"key":vec![7; 32],"clip":[[20,3.5]],"master_db":-1.0}}),
        ),
        ("audio.clear_loudness", json!({"scope":"clip"})),
    ] {
        let before = session.project().clone();
        session.execute(command, args).unwrap();
        let after = session.project().clone();
        assert_ne!(before, after);
        session.undo().unwrap();
        assert_eq!(&before, session.project());
        session.redo().unwrap();
        assert_eq!(&after, session.project());
        let encoded = project::encode(session.project(), false).unwrap();
        assert_eq!(&project::decode(&encoded).unwrap().project, after.as_ref());
    }
    assert_eq!(session.project().data().settings.master_volume, -1.25);
    assert!(session.project().data().settings.master_muted);
    let loudness = &session.project().data().settings.loudness;
    assert!(loudness.clip.is_none() && loudness.master.is_some() && loudness.measured.is_none());
    // Invalid targets and gains refuse atomically.
    for (command, args) in [
        (
            "audio.set_loudness",
            json!({"scope":"master","target_lufs":-4.0,"true_peak_db":-1.0}),
        ),
        (
            "audio.set_loudness",
            json!({"scope":"master","target_lufs":-51.0,"true_peak_db":-1.0}),
        ),
        (
            "audio.set_loudness",
            json!({"scope":"master","target_lufs":-14.0,"true_peak_db":0.5}),
        ),
        (
            "audio.set_loudness",
            json!({"scope":"clip","target_lufs":-14.0,"true_peak_db":-21.0}),
        ),
        (
            "audio.set_loudness",
            json!({"scope":"clip","target_lufs":-14.001,"true_peak_db":-1.0}),
        ),
        (
            "audio.apply_loudness",
            json!({"gains":{"key":vec![0; 32],"clip":[],"master_db":61.0}}),
        ),
        (
            "audio.apply_loudness",
            json!({"gains":{"key":vec![0; 32],"clip":[[2,0.0],[1,0.0]],"master_db":0.0}}),
        ),
    ] {
        let before = session.project().clone();
        assert!(session.execute(command, args).is_err());
        assert_eq!(session.project(), &before);
    }
    for value in [-60.01, 20.01, 0.001] {
        let before = session.project().clone();
        assert!(
            session
                .execute("audio.set_master", json!({"volume":value,"muted":false}))
                .is_err()
        );
        assert!(
            session
                .execute(
                    "timeline.set_item_param",
                    json!({"item":20,"patch":{"field":"volume","value":value}})
                )
                .is_err()
        );
        assert_eq!(session.project(), &before);
    }
    for value in [-60.0, 20.0] {
        session
            .execute("audio.set_master", json!({"volume":value,"muted":false}))
            .unwrap();
    }
    assert!(
        session
            .execute(
                "timeline.set_item_param",
                json!({"item":20,"patch":{"field":"audio_gain","value":0.0}})
            )
            .is_err()
    );
    let mut encoded: serde_json::Value =
        serde_json::from_slice(&project::encode(session.project(), false).unwrap()).unwrap();
    encoded["schema_version"] = json!(10);
    assert!(matches!(
        project::decode(&serde_json::to_vec(&encoded).unwrap()),
        Err(project::Error::TooOld { .. })
    ));
}

fn ass_style() -> captions::CueStyle {
    captions::CueStyle {
        font_family: "Arial".into(),
        font_size_ratio: 0.05,
        primary_color: [255, 255, 255, 255],
        outline_color: [0, 0, 0, 255],
        background_color: [0, 0, 0, 0],
        border_style: 1,
        alignment: captions::CueAlignment {
            numpad: 2,
            horizontal: captions::HorizontalAlignment::Center,
            vertical: captions::VerticalAlignment::Bottom,
        },
        margin_left_ratio: 0.0,
        margin_right_ratio: 0.0,
        margin_vertical_ratio: 0.0,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    }
}

fn ass_asset() -> SourceAsset {
    SourceAsset::try_from(AssetData {
        ephemeral: true,
        id: AssetId(30),
        path: std::env::temp_dir().join("captions.ass"),
        content_hash: ContentHash::from_bytes(b"ass"),
        probe: SourceProbe {
            format: "ass".into(),
            duration: Some(Tick(254_016_000_000)),
            streams: Vec::new(),
        },
    })
    .unwrap()
}

fn ass_import() -> Command {
    Command::ImportCaptions {
        asset: ass_asset(),
        language: captions::LanguageTag::parse("en").unwrap(),
        start: Tick(100),
        track: None,
        name: Some("Imported captions".into()),
        cues: vec![
            captions::AssCue {
                range: range(0, 254_016_000_000),
                text: "Hello".into(),
                style: ass_style(),
            },
            captions::AssCue {
                range: range(508_032_000_000, 762_048_000_000),
                text: "World".into(),
                style: ass_style(),
            },
        ],
    }
}

#[test]
fn captions_import_is_one_undo_step_and_persists_style() {
    let mut s = session();
    let before = s.history_len();
    let outcome = s.apply(ass_import()).unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (before.0 + 1, 0));
    assert_eq!(outcome.placements.len(), 1);
    assert!(
        outcome
            .created
            .iter()
            .any(|id| matches!(id, CreatedId::Track(_)))
    );
    assert!(
        outcome
            .created
            .iter()
            .any(|id| matches!(id, CreatedId::Item(_)))
    );

    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap();
    let ItemParams::Caption(params) = &item.content else {
        panic!("expected caption item")
    };
    assert_eq!(params.cues.cues.len(), 2);
    assert_eq!(
        params.cues.cues[0]
            .texts
            .values()
            .next()
            .unwrap()
            .text
            .as_str(),
        "Hello"
    );
    assert_eq!(params.cues.styles.len(), 1);
    assert_eq!(params.cues.cues[0].style, Some(captions::CueStyleId(0)));
    assert_eq!(
        params
            .cues
            .style_of(&params.cues.cues[0])
            .unwrap()
            .font_family
            .as_str(),
        "Arial"
    );
    assert_eq!(item.start, Tick(100));
    assert_eq!(item.source_range, range(0, 762_048_000_000));

    let encoded = project::encode(s.project().as_ref(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap();
    assert_eq!(loaded.project, **s.project());
    let ItemParams::Caption(reloaded) = &loaded
        .project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap()
        .content
    else {
        panic!("expected caption item")
    };
    assert_eq!(reloaded.cues.cues[0].style, Some(captions::CueStyleId(0)));
    assert_eq!(
        reloaded.cues.style_of(&reloaded.cues.cues[0]).unwrap(),
        &ass_style()
    );

    s.apply(Command::Undo {}).unwrap();
    assert!(
        !s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .any(|track| track.kind == TrackKind::Text)
    );
    assert!(
        !s.project()
            .data()
            .assets
            .iter()
            .any(|asset| asset.id() == AssetId(30))
    );

    s.apply(Command::Redo {}).unwrap();
    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap();
    let ItemParams::Caption(params) = &item.content else {
        panic!("expected caption item")
    };
    assert_eq!(params.cues.styles, vec![ass_style()]);
    assert_eq!(params.cues.cues[0].style, Some(captions::CueStyleId(0)));
}

#[test]
fn captions_import_command_admits_json_and_rejects_bad_cues_atomically() {
    let mut s = session();
    let command = ass_import();
    let Command::ImportCaptions {
        asset,
        language,
        start,
        track,
        name,
        cues,
    } = command
    else {
        panic!("expected import command")
    };
    let params = json!({
        "asset": serde_json::to_value(&asset).unwrap(),
        "language": language.as_str(),
        "start": start.0,
        "track": track,
        "name": name,
        "cues": serde_json::to_value(&cues).unwrap(),
    });
    let before = s.project().clone();
    let outcome = s.execute("captions.import", params.clone()).unwrap();
    assert!(outcome.changed);
    assert_eq!(s.project().data().scene().timeline.track_count(), 4);

    let mut bad = params;
    bad["cues"][0]["range"]["end"] = json!(0);
    let after = s.project().clone();
    assert!(s.execute("captions.import", bad).is_err());
    assert_eq!(s.project(), &after);
    assert_ne!(after, before);

    // Hostile placement, count and style bursts are refused whole, never a
    // panic or an unsaveable project.
    let make = |count: usize, start: Tick| {
        let Command::ImportCaptions {
            asset,
            language,
            track,
            name,
            ..
        } = ass_import()
        else {
            panic!("expected import command")
        };
        let cues = (0..count)
            .map(|index| captions::AssCue {
                range: range(index as i64 * 1_000_000, index as i64 * 1_000_000 + 500_000),
                text: format!("line {index}").into(),
                style: ass_style(),
            })
            .collect();
        Command::ImportCaptions {
            asset,
            language,
            start,
            track,
            name,
            cues,
        }
    };
    let make_unique_styles = |count: usize| {
        let Command::ImportCaptions {
            asset,
            language,
            track,
            name,
            ..
        } = ass_import()
        else {
            panic!("expected import command")
        };
        let cues = (0..count)
            .map(|index| captions::AssCue {
                range: range(index as i64 * 1_000_000, index as i64 * 1_000_000 + 500_000),
                text: format!("line {index}").into(),
                style: captions::CueStyle {
                    font_size_ratio: 0.001 + index as f32 * 0.0001,
                    ..ass_style()
                },
            })
            .collect();
        Command::ImportCaptions {
            asset,
            language,
            start: Tick(0),
            track,
            name,
            cues,
        }
    };
    for (count, start) in [(1, Tick(i64::MAX - 10)), (1, Tick(-1))] {
        let mut fresh = session();
        let untouched = fresh.project().clone();
        assert!(
            fresh.apply(make(count, start)).is_err(),
            "{count} cues at {start:?}"
        );
        assert_eq!(fresh.project(), &untouched);
    }
    let mut fresh = session();
    fresh.apply(make(20_000, Tick(0))).unwrap();
    let encoded = project::encode(fresh.project().as_ref(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap();
    let ItemParams::Caption(params) = &loaded
        .project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap()
        .content
    else {
        panic!("expected caption item")
    };
    assert_eq!(params.cues.cues.len(), 20_000);
    assert_eq!(params.cues.styles.len(), 1);
    assert!(
        params
            .cues
            .cues
            .iter()
            .all(|cue| cue.style == Some(captions::CueStyleId(0)))
    );

    let mut fresh = session();
    let untouched = fresh.project().clone();
    assert!(matches!(
        fresh.apply(make_unique_styles(captions::MAX_CUE_STYLES + 1)),
        Err(engine::EditError::Limit)
    ));
    assert_eq!(fresh.project(), &untouched);
    assert!(matches!(
        session().apply(make(captions::ass::MAX_ASS_CUES + 1, Tick(0))),
        Err(engine::EditError::Limit)
    ));
}

#[test]
fn rollback_must_invalidate_target_captured_in_discarded_preview() {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
        10,
    )
    .unwrap();
    let mut target = None;
    let result: Result<(), EditError> = session.transaction(|s| {
        s.apply(engine::Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "".into(),
            kind: project::TrackKind::Video,
        })?;
        target = Some(s.job_target());
        Err(EditError::Cancelled)
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
            Err(EditError::StaleResult)
        ),
        "discarded preview target was accepted against rolled-back content"
    );
}

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
        s.apply_with_ripple(command, enabled).unwrap();
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
    s.execute("timeline.set_ripple", json!({"enabled":true}))
        .unwrap();
    let preview = |at| Command::TimelinePreview {
        commands: vec![Command::TrimEnd {
            item: ItemId(10),
            at: Tick(at),
        }],
    };
    // The desktop starts a gesture before its first replacement preview.
    s.begin_gesture().unwrap();
    s.apply_with_ripple(preview(40), true).unwrap();
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
        s.execute("timeline.set_ripple", json!({"enabled":false})),
        Err(EditError::TransactionActive)
    ));
    s.apply_with_ripple(preview(60), true).unwrap();
    assert_eq!(s.preview_project().unwrap().as_ref(), s.project().as_ref());
    s.apply_with_ripple(preview(40), true).unwrap();
    assert!(s.apply_with_ripple(preview(-1), true).is_err());
    s.execute("timeline.discard_preview", json!({})).unwrap();
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
        s.apply_with_ripple(preview(60), true),
        Err(EditError::TransactionActive)
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
    assert!(
        s.execute("timeline.set_ripple", json!({"enabled":"yes"}))
            .is_err()
    );
    assert!(
        s.execute(
            "batch",
            json!({"commands":[{"command":"timeline.set_ripple","params":{"enabled":false}}]})
        )
        .is_err()
    );
}

#[test]
fn ripple_batch_subtracts_occupied_spans_and_never_closes_an_old_gap_twice() {
    let (mut s, track, _) = ripple_fixture(&[10, 10, 10], 20);
    s.execute("timeline.set_ripple", json!({"enabled":true}))
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
    s.apply_with_ripple(
        Command::MoveItem {
            item: ItemId(10),
            track,
            start: Tick(0),
        },
        true,
    )
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
    s.apply_with_ripple(
        Command::MoveItems {
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
        },
        true,
    )
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
    for (id, params, expected) in [
        ("delete", json!({"items":[10]}), 20),
        ("ripple_delete", json!({"item":10}), 20),
        ("trim_end", json!({"item":10,"at":40}), 40),
        ("trim_start", json!({"item":10,"at":40}), 60),
        (
            "timeline.trim",
            json!({"items":[10],"edge":"end","delta":-20}),
            40,
        ),
        ("move_item", json!({"item":10,"track":2,"start":10}), 50),
        (
            "move_items",
            json!({"moves":[{"item":10,"track":2,"start":10}]}),
            50,
        ),
        ("timeline.move_items", json!({"items":[10],"delta":-10}), 50),
    ] {
        let (mut s, track, _) = ripple_fixture(&[40, 40], 20);
        let mut params = params;
        if let Some(value) = params.get_mut("track") {
            *value = json!(track);
        }
        if let Some(value) = params.pointer_mut("/moves/0/track") {
            *value = json!(track);
        }
        let before = project::encode(s.project(), false).unwrap();
        s.execute_with_ripple(id, params, true).unwrap();
        assert_eq!(
            ripple_layout(&s)
                .iter()
                .find(|row| row.1 == ItemId(11))
                .unwrap()
                .2,
            Tick(expected),
            "{id}"
        );
        s.apply(Command::Undo {}).unwrap();
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
        assert!(
            s.execute_with_ripple(
                "move_item",
                json!({"item":10,"track":track,"start":i64::MAX}),
                true
            )
            .is_err()
        );
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
    }
    let (mut s, _, _) = ripple_fixture(&[], 0);
    assert!(
        !s.execute_with_ripple("delete", json!({"items":[]}), true)
            .unwrap()
            .changed
    );
    let (mut s, _, _) = ripple_fixture(&[1], 0);
    s.execute_with_ripple("delete", json!({"items":[10]}), true)
        .unwrap();
    assert!(!ripple_layout(&s).iter().any(|row| row.1 == ItemId(10)));
}

#[test]
fn ripple_wire_policy_is_atomic_explicit_and_cannot_nest() {
    for (default, explicit) in [(false, true), (true, false)] {
        let (mut s, _, _) = ripple_fixture(&[40, 40], 20);
        s.execute("timeline.set_ripple", json!({"enabled":default}))
            .unwrap();
        s.execute(
            "timeline.edit",
            json!({"commands":[{"command":"delete","params":{"items":[10]}}],"ripple":explicit}),
        )
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
            assert!(s.execute("timeline.edit", params).is_err());
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
            Err(EditError::Limit)
        ));
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
        for command in ["batch", "timeline.edit"] {
            let mut params = json!({"commands":[{"command":"delete","params":{"items":[10]}},
                {"command":"timeline.edit","params":{"commands":[],"ripple":true}}]});
            if command == "timeline.edit" {
                params["ripple"] = json!(true);
            }
            assert!(s.execute(command, params).is_err());
            assert_eq!(project::encode(s.project(), false).unwrap(), before);
            assert_eq!(s.history_len(), history);
        }
    }
}

// Pause removal: reviewed output-time spans cut every track and close their tails.

fn pause_audio_asset() -> SourceAsset {
    let duration = Tick(12 * time::TICKS_PER_SECOND);
    testkit::source::asset(1, std::env::temp_dir().join("pause-audio.wav"))
        .format("wav")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(b"pause audio"))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(duration)
                .build(),
        )
        .build()
}

fn pause_video_asset() -> SourceAsset {
    let duration = Tick(12 * time::TICKS_PER_SECOND);
    testkit::source::asset(2, std::env::temp_dir().join("pause-video.mp4"))
        .format("mp4")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(b"pause video"))
        .stream(
            testkit::source::video()
                .time_base(media::TimeBase::new(1, 30000).unwrap())
                .duration(duration)
                .size(64, 36)
                .build(),
        )
        .build()
}

fn five_clip_pairs() -> Session {
    let edge = |seconds: i64| Tick(seconds * time::TICKS_PER_SECOND);
    let edges = [0, 2, 4, 6, 8, 12];
    let mut data = project().into_data();
    data.assets = vec![pause_audio_asset(), pause_video_asset()];
    data.scene_mut().timeline.overlays.clear();
    data.scene_mut().timeline.audio.clear();
    data.scene_mut().timeline.main.items = edges
        .windows(2)
        .enumerate()
        .map(|(index, pair)| {
            let start = pair.first().copied().unwrap_or_default();
            let end = pair.last().copied().unwrap_or_default();
            Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: ItemId(index as u64 + 1),
                source_asset: Some(AssetId(2)),
                source_range: range(edge(start).0, edge(end).0),
                start: edge(start),
                content: ItemParams::Video(project::VideoParams {
                    fades: Default::default(),
                    source_audio_enabled: false,
                    volume: 0.0,
                    muted: false,
                    crop: None,
                }),
            }
        })
        .collect();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(5),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: edges
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                let start = pair.first().copied().unwrap_or_default();
                let end = pair.last().copied().unwrap_or_default();
                Item {
                    hidden: false,
                    blend_mode: Default::default(),
                    retime: Default::default(),
                    masks: Vec::new(),
                    effects: Vec::new(),
                    animations: Default::default(),
                    id: ItemId(index as u64 + 11),
                    source_asset: Some(AssetId(1)),
                    source_range: range(edge(start).0, edge(end).0),
                    start: edge(start),
                    content: ItemParams::Audio(project::AudioParams {
                        fades: Default::default(),
                        volume: 0.0,
                        muted: false,
                        source: Default::default(),
                    }),
                }
            })
            .collect(),
    });
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}

#[test]
fn remove_pauses_cuts_reviewed_spans_as_one_undo_step() {
    let mut s = session();
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let outcome = s
        .execute(
            "timeline.remove_pauses",
            json!({"spans":[{"start":20,"end":60}],"crossfade_ms":0}),
        )
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0));
    assert_ne!(s.revision(), revision);
    let edited = project::encode(s.project(), false).unwrap();
    let track = &s.project().data().scene().timeline.overlays[0];
    let layout: Vec<(Tick, Tick)> = track
        .items
        .iter()
        .map(|item| (item.start, item.end().unwrap()))
        .collect();
    // The cut half-open span [20,60) leaves one left piece and shifts the follower.
    assert_eq!(layout, vec![(Tick(0), Tick(20)), (Tick(40), Tick(80))]);

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), edited);
    let loaded = project::decode(&edited).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());

    // A nested batch child still cuts through the ripple path.
    let mut batched = session();
    batched
        .execute(
            "batch",
            json!({"commands":[{"command":"timeline.remove_pauses",
                "params":{"spans":[{"start":20,"end":60}],"crossfade_ms":0}}]}),
        )
        .unwrap();
    assert_eq!(project::encode(batched.project(), false).unwrap(), edited);
}

#[test]
fn remove_pauses_refuses_invalid_reviewed_spans_atomically() {
    let mut s = session();
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let mut cases = vec![
        json!([]),
        json!([{"start":100,"end":60}]),
        json!([{"start":20,"end":60},{"start":10,"end":15}]),
        json!([{"start":10,"end":50},{"start":40,"end":60}]),
        json!([{"start":20,"end":20}]),
        json!([{"start":-1,"end":10}]),
        json!([{"start":0,"end":1000}]),
    ];
    let many: Vec<Value> = (0..=engine::control::MAX_PAUSE_SPANS)
        .map(|index| json!({"start":index as i64 * 2,"end":index as i64 * 2 + 1}))
        .collect();
    cases.push(Value::Array(many));
    for spans in cases {
        assert!(
            s.execute(
                "timeline.remove_pauses",
                json!({"spans":spans,"crossfade_ms":0})
            )
            .is_err(),
            "{spans:?}"
        );
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), (0, 0));
    }
    assert!(
        s.execute(
            "timeline.remove_pauses",
            json!({"spans":[{"start":0,"end":10}],"crossfade_ms":10001})
        )
        .is_err()
    );

    // A caption word forbids any span that crosses it.
    let mut data = project().into_data();
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("pause-words.mp4"))
            .format("mp4")
            .duration(Tick(1000))
            .content_hash(ContentHash::from_bytes(b"words"))
            .build(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(90),
            source_asset: Some(AssetId(7)),
            source_range: range(0, 1000),
            start: Tick::ZERO,
            content: ItemParams::Caption(project::CaptionParams {
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues: vec![captions::Cue {
                        id: "cue-1".into(),
                        range: range(0, 1000),
                        source_lang: captions::LanguageTag::parse("en").unwrap(),
                        texts: std::collections::BTreeMap::new(),
                        word_ranges: vec![range(10, 20)],
                        style: None,
                    }],
                },
            }),
        }],
    });
    let mut words = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = project::encode(words.project(), false).unwrap();
    assert!(
        words
            .execute(
                "timeline.remove_pauses",
                json!({"spans":[{"start":5,"end":15}],"crossfade_ms":0})
            )
            .is_err()
    );
    assert_eq!(project::encode(words.project(), false).unwrap(), before);
    // A span beside the word is allowed.
    assert!(
        words
            .execute(
                "timeline.remove_pauses",
                json!({"spans":[{"start":20,"end":30}],"crossfade_ms":0})
            )
            .is_ok()
    );

    // A locked track refuses the whole edit.
    let mut locked = session();
    let track = locked.project().data().scene().timeline.overlays[0].id;
    locked
        .apply(Command::SetTrackLocked {
            track,
            locked: true,
        })
        .unwrap();
    let before = project::encode(locked.project(), false).unwrap();
    assert!(matches!(
        locked.execute(
            "timeline.remove_pauses",
            json!({"spans":[{"start":20,"end":60}],"crossfade_ms":0})
        ),
        Err(EditError::Locked(_))
    ));
    assert_eq!(project::encode(locked.project(), false).unwrap(), before);

    // An empty timeline has no span to remove.
    let mut empty = Session::new(Project::empty(ProjectId(0), "".into()).unwrap(), 1).unwrap();
    assert!(
        empty
            .execute(
                "timeline.remove_pauses",
                json!({"spans":[{"start":0,"end":10}],"crossfade_ms":0})
            )
            .is_err()
    );
}

#[test]
fn remove_pauses_keeps_five_clip_audio_video_aligned() {
    let mut s = five_clip_pairs();
    let before = project::encode(s.project(), false).unwrap();
    let spans = |seconds: &[(i64, i64)]| -> Vec<Value> {
        seconds
            .iter()
            .map(|(start, end)| json!({"start":start * time::TICKS_PER_SECOND,"end":end * time::TICKS_PER_SECOND}))
            .collect()
    };
    s.execute(
        "timeline.remove_pauses",
        json!({"spans":spans(&[(2, 3), (4, 5), (8, 10)]),"crossfade_ms":20}),
    )
    .unwrap();
    let data = s.project().data().clone();
    let starts = |kind: TrackKind| -> Vec<Tick> {
        let mut starts: Vec<Tick> = data
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == kind)
            .flat_map(|track| track.items.iter().map(|item| item.start))
            .collect();
        starts.sort_unstable();
        starts
    };
    assert_eq!(starts(TrackKind::Video), starts(TrackKind::Audio));
    assert_eq!(starts(TrackKind::Video).len(), 5);
    let saved = 4 * time::TICKS_PER_SECOND;
    assert_eq!(
        data.scene()
            .timeline
            .main
            .items
            .last()
            .unwrap()
            .end()
            .unwrap(),
        Tick(12 * time::TICKS_PER_SECOND - saved)
    );
    // Three joins per media track each carry one equal-power fade-in and fade-out.
    let (fade_ins, fade_outs) = data
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == TrackKind::Audio)
        .flat_map(|track| track.items.iter())
        .fold((0, 0), |(ins, outs), item| match &item.content {
            ItemParams::Audio(params) => (
                ins + usize::from(params.fades.fade_in.window() > Tick::ZERO),
                outs + usize::from(params.fades.fade_out.window() > Tick::ZERO),
            ),
            _ => (ins, outs),
        });
    assert_eq!((fade_ins, fade_outs), (3, 3));
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}

// --- Transcript word cuts -------------------------------------------------

fn word_cue(id: &str, start: i64, end: i64, words: &[(i64, i64)], edited: bool) -> captions::Cue {
    let language = captions::LanguageTag::parse("vi").unwrap();
    let text = ["Một", "hai", "ba", "bốn"]
        .get(..words.len())
        .unwrap_or_default()
        .join(" ");
    captions::Cue {
        id: id.into(),
        range: range(start, end),
        source_lang: language.clone(),
        texts: std::collections::BTreeMap::from([(
            language,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        word_ranges: words.iter().map(|&(a, b)| range(a, b)).collect(),
        style: None,
    }
}

fn caption_words_item(id: u64, start: i64, source: (i64, i64), cues: Vec<captions::Cue>) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: Some(AssetId(7)),
        source_range: range(source.0, source.1),
        start: Tick(start),
        content: ItemParams::Caption(project::CaptionParams {
            cues: captions::CueTrack {
                styles: Vec::new(),
                cues,
            },
        }),
    }
}

fn clip_item(id: u64, start: i64, source: (i64, i64)) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: None,
        source_range: range(source.0, source.1),
        start: Tick(start),
        content: content(),
    }
}

fn word_asset(data: &mut project::ProjectData, duration: i64) {
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("transcript-words.mp4"))
            .format("mp4")
            .duration(Tick(duration))
            .content_hash(ContentHash::from_bytes(b"words"))
            .build(),
    );
}

/// Two adjacent visual clips and one caption item whose third word crosses the
/// clip boundary at 600.
fn word_project(edited: bool, locked: bool) -> Session {
    let mut data = project().into_data();
    word_asset(&mut data, 1200);
    let main = data.scene().timeline.main.id;
    data.scene_mut()
        .timeline
        .iter_mut()
        .find(|track| track.id == main)
        .unwrap()
        .items = vec![clip_item(5, 0, (0, 600)), clip_item(6, 600, (600, 1200))];
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked,
        items: vec![caption_words_item(
            4,
            0,
            (0, 1200),
            vec![word_cue(
                "câu-1",
                0,
                1200,
                &[(50, 150), (150, 250), (550, 650), (800, 900)],
                edited,
            )],
        )],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

/// One source-language cue with an explicit text and measured word ranges.
fn merge_cue(
    id: &str,
    start: i64,
    end: i64,
    text: &str,
    words: &[(i64, i64)],
    edited: bool,
) -> captions::Cue {
    let language = captions::LanguageTag::parse("vi").unwrap();
    captions::Cue {
        id: id.into(),
        range: range(start, end),
        source_lang: language.clone(),
        texts: std::collections::BTreeMap::from([(
            language,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        word_ranges: words.iter().map(|&(a, b)| range(a, b)).collect(),
        style: None,
    }
}

/// One caption item with three adjacent measured-word cues and an English
/// translation on each. The first cue's source text is optionally protected and
/// the caption track is optionally locked.
fn merge_project(edited: bool, locked: bool) -> Session {
    let mut data = project().into_data();
    word_asset(&mut data, 1200);
    let mut cues = vec![
        merge_cue("câu-1", 0, 400, "Một hai", &[(0, 100), (100, 200)], edited),
        merge_cue(
            "câu-2",
            400,
            900,
            "ba bốn",
            &[(400, 500), (500, 600)],
            false,
        ),
        merge_cue("câu-3", 900, 1200, "năm", &[(900, 1000)], false),
    ];
    let english = captions::LanguageTag::parse("en").unwrap();
    let chinese = captions::LanguageTag::parse("zh").unwrap();
    let japanese = captions::LanguageTag::parse("ja").unwrap();
    for (index, cue) in cues.iter_mut().enumerate() {
        let text = match index {
            0 => "one two",
            1 => "three four",
            _ => "five",
        };
        cue.texts.insert(
            english.clone(),
            captions::CueText {
                text: text.into(),
                edited: index == 1,
                translation_ref: Some("translation/1".into()),
            },
        );
        let text = match index {
            0 => "你好",
            1 => "世界",
            _ => "第三",
        };
        cue.texts.insert(
            chinese.clone(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        );
        let text = match index {
            0 => "こんにちは",
            1 => "世界",
            _ => "三",
        };
        cue.texts.insert(
            japanese.clone(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        );
    }
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked,
        items: vec![caption_words_item(4, 0, (0, 1200), cues)],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

/// Word output ranges of the active scene keyed by cue word index.
fn word_layout(session: &Session) -> Vec<(usize, i64, i64)> {
    let mut words: Vec<(usize, i64, i64)> = session
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .map(|word| (word.id.word, word.output.start().0, word.output.end().0))
        .collect();
    words.sort_unstable();
    words
}

fn assert_unchanged(s: &Session, bytes: &[u8], revision: &engine::control::Revision) {
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(&s.revision(), revision);
    assert_eq!(s.history_len(), (0, 0));
}

#[test]
fn transcript_delete_cuts_a_word_across_a_clip_boundary_as_one_undo_step() {
    let mut s = word_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let expected_texts = word_cue(
        "câu-1",
        0,
        1200,
        &[(50, 150), (150, 250), (550, 650), (800, 900)],
        false,
    )
    .texts;
    assert_eq!(
        word_layout(&s),
        vec![(0, 50, 150), (1, 150, 250), (2, 550, 650), (3, 800, 900)]
    );
    let cue = s.transcript().unwrap().cues[0].clone();
    let outcome = s
        .execute(
            "transcript.delete_range",
            json!({"word_ids":[{"item":cue.item,"cue":cue.id,"word":2}]}),
        )
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "one undo step per command");
    assert_ne!(s.revision(), revision);
    // The span crosses the 600 boundary: both clips are cut and the tails close.
    let main = s.project().data().scene().timeline.main.id;
    let layout: Vec<(Tick, Tick)> = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.id == main)
        .unwrap()
        .items
        .iter()
        .map(|item| (item.start, item.end().unwrap()))
        .collect();
    assert_eq!(layout, vec![(Tick(0), Tick(550)), (Tick(550), Tick(1100))]);
    assert_eq!(
        word_layout(&s),
        vec![(0, 50, 150), (1, 150, 250), (3, 700, 800)]
    );

    // Caption text, language and edit flags survive the cut; no cue is rewritten.
    let view = s.transcript().unwrap();
    for cue in &view.cues {
        assert_eq!(cue.texts, expected_texts);
        assert!(!cue.protected);
        assert!(!cue.locked);
    }

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    let edited = project::encode(s.project(), false).unwrap();
    assert_eq!(
        word_layout(&s),
        vec![(0, 50, 150), (1, 150, 250), (3, 700, 800)]
    );

    // A nested batch child still cuts through the reviewed-span path.
    let mut batched = word_project(false, false);
    batched
        .execute(
            "batch",
            json!({"commands":[{"command":"transcript.delete_range",
                "params":{"word_ids":[{"item":4,"cue":"câu-1","word":2}]}}]}),
        )
        .unwrap();
    assert_eq!(project::encode(batched.project(), false).unwrap(), edited);
}

#[test]
fn transcript_delete_keeps_repeated_spans_and_trimmed_items_distinct() {
    let mut data = project().into_data();
    word_asset(&mut data, 1200);
    let cues = || vec![word_cue("câu-1", 0, 1200, &[(50, 150), (150, 250)], false)];
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            caption_words_item(4, 0, (0, 1200), cues()),
            caption_words_item(8, 2000, (0, 1200), cues()),
            caption_words_item(
                12,
                4000,
                (100, 1200),
                vec![word_cue("câu-1", 100, 1200, &[(100, 150)], false)],
            ),
        ],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    s.execute(
        "transcript.delete_range",
        json!({"word_ids":[{"item":4,"cue":"câu-1","word":1}]}),
    )
    .unwrap();
    let mut words: Vec<(u64, usize, i64, i64)> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .map(|word| {
            (
                word.id.item.0,
                word.id.word,
                word.output.start().0,
                word.output.end().0,
            )
        })
        .collect();
    words.sort_unstable();
    assert_eq!(
        words,
        vec![
            (4, 0, 50, 150),
            // The repeated source span in another item keeps both words, shifted
            // left with the tail the cut vacated on this track.
            (8, 0, 1950, 2050),
            (8, 1, 2050, 2150),
            // A trimmed item clips its word instead of duplicating it.
            (12, 0, 3900, 3950),
        ]
    );
    let mut outputs: Vec<(i64, i64)> = words.iter().map(|&(_, _, a, b)| (a, b)).collect();
    outputs.dedup();
    assert_eq!(outputs.len(), words.len(), "no output range is duplicated");
}

#[test]
fn transcript_delete_refuses_protected_locked_and_foreign_words_atomically() {
    let word = |item: u64, cue: &str, index: usize| json!({"item":item,"cue":cue,"word":index});
    let cases = vec![
        json!([]),
        json!([word(4, "câu-1", 0), word(4, "câu-1", 0)]),
        json!([word(4, "câu-1", 9)]),
        json!([word(4, "khác", 0)]),
        json!([word(77, "câu-1", 0)]),
        json!([word(4, "câu-1", usize::MAX)]),
        json!([word(6, "câu-1", 0)]),
    ];

    let mut plain = word_project(false, false);
    let before = project::encode(plain.project(), false).unwrap();
    let revision = plain.revision();
    for word_ids in cases {
        let params = json!({"word_ids": word_ids});
        assert!(
            plain
                .execute("transcript.delete_range", params.clone())
                .is_err(),
            "{params}"
        );
        assert_unchanged(&plain, &before, &revision);
    }

    let many: Vec<Value> = (0..=engine::control::MAX_COLLECTION)
        .map(|index| word(4, "câu-1", index))
        .collect();
    assert!(matches!(
        plain.execute("transcript.delete_range", json!({"word_ids":many})),
        Err(EditError::Limit)
    ));
    assert_unchanged(&plain, &before, &revision);

    // An edited cue text protects every one of its words.
    let mut protected = word_project(true, false);
    let before = project::encode(protected.project(), false).unwrap();
    let revision = protected.revision();
    assert!(matches!(
        protected.execute(
            "transcript.delete_range",
            json!({"word_ids":[{"item":4,"cue":"câu-1","word":0}]})
        ),
        Err(EditError::Protected)
    ));
    assert_unchanged(&protected, &before, &revision);

    // A locked caption track refuses the whole edit.
    let mut locked = word_project(false, true);
    let before = project::encode(locked.project(), false).unwrap();
    let revision = locked.revision();
    assert!(matches!(
        locked.execute(
            "transcript.delete_range",
            json!({"word_ids":[{"item":4,"cue":"câu-1","word":0}]})
        ),
        Err(EditError::Locked(_))
    ));
    assert_unchanged(&locked, &before, &revision);

    // A word that lives in another scene refuses with the typed scene boundary.
    let mut data = word_project(false, false).project().data().clone();
    let mut other = data.scene().clone();
    other.id = project::SceneId(7);
    other.name = "Khác".into();
    other.is_main = false;
    other.timeline = Default::default();
    other.timeline.main.id = TrackId(30);
    other.timeline.insert_track(Track {
        id: TrackId(20),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            90,
            0,
            (0, 1200),
            vec![word_cue("câu-2", 0, 1200, &[(0, 100)], false)],
        )],
    });
    let main = data.scene().clone();
    data.scenes = project::Scenes::try_from(vec![main, other]).unwrap();
    let mut two_scenes = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = project::encode(two_scenes.project(), false).unwrap();
    let revision = two_scenes.revision();
    assert!(matches!(
        two_scenes.execute(
            "transcript.delete_range",
            json!({"word_ids":[{"item":90,"cue":"câu-2","word":0}]})
        ),
        Err(EditError::SceneBoundary)
    ));
    assert_unchanged(&two_scenes, &before, &revision);
    // The same words are editable while their scene is active.
    two_scenes
        .apply(Command::SwitchScene {
            scene: project::SceneId(7),
        })
        .unwrap();
    two_scenes
        .execute(
            "transcript.delete_range",
            json!({"word_ids":[{"item":90,"cue":"câu-2","word":0}]}),
        )
        .unwrap();
}

proptest! {
    #[test]
    fn transcript_delete_mapping_round_trips_within_a_frame(
        length in 500_i64..5_000,
        first_start in 0_i64..400,
        first_len in 1_i64..100,
        gap in 0_i64..300,
        second_len in 1_i64..100,
    ) {
        let first_end = first_start + first_len;
        let second_start = first_end + gap;
        let second_end = (second_start + second_len).min(length);
        prop_assume!(second_end > second_start && first_end <= second_start);
        let mut data = project().into_data();
        word_asset(&mut data, length);
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(9),
            name: "Captions".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![caption_words_item(
                4,
                0,
                (0, length),
                vec![word_cue(
                    "câu-1",
                    0,
                    length,
                    &[(first_start, first_end), (second_start, second_end)],
                    false,
                )],
            )],
        });
        let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
        s.execute(
            "transcript.delete_range",
            json!({"word_ids":[{"item":4,"cue":"câu-1","word":0}]}),
        ).unwrap();
        let frame = time::FrameRate::FPS_30.tick_of(1).unwrap();
        for track in s.project().data().scene().timeline.iter() {
            for item in &track.items {
                for word in item.word_spans().unwrap() {
                    prop_assert_eq!(
                        item.output_time_of(word.source.start()).unwrap(),
                        word.output.start()
                    );
                    let back = item.source_time_at(word.output.start()).unwrap();
                    prop_assert!((back.0 - word.source.start().0).abs() <= frame.0);
                    let back = item.source_time_at(word.output.end()).unwrap();
                    prop_assert!((back.0 - word.source.end().0).abs() <= frame.0);
                }
            }
        }
        // The second word survives with its source duration and its text.
        let view = s.transcript().unwrap();
        let words: Vec<_> = view.cues.iter().flat_map(|cue| &cue.words).collect();
        prop_assert_eq!(words.len(), 1);
        prop_assert_eq!(words[0].id.word, 1);
        prop_assert_eq!(words[0].output.duration().0, second_end - second_start);
        prop_assert_eq!(words[0].text.as_ref().unwrap().as_str(), "hai");
    }
}

// --- Transcript cue merges ------------------------------------------------

#[test]
fn transcript_merge_joins_adjacent_cues_as_one_undo_step() {
    let mut s = merge_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    // Input order is irrelevant: the selected cues sort by time.
    let outcome = s
        .execute(
            "transcript.merge_cues",
            json!({"cue_ids":[{"item":4,"cue":"câu-2"},{"item":4,"cue":"câu-1"}]}),
        )
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "one undo step per merge");
    assert_ne!(s.revision(), revision);

    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 2, "cue-3 stays separate");
    let merged = &view.cues[0];
    assert_eq!(merged.id.as_str(), "câu-1", "the earliest cue keeps its id");
    assert_eq!(merged.output, range(0, 900));
    assert_eq!(
        merged.texts[&captions::LanguageTag::parse("vi").unwrap()]
            .text
            .as_str(),
        "Một hai ba bốn"
    );
    let english = &merged.texts[&captions::LanguageTag::parse("en").unwrap()];
    assert_eq!(english.text.as_str(), "one two three four");
    assert!(english.edited, "an edited translation stays edited");
    assert_eq!(
        english.translation_ref.as_ref().unwrap().as_str(),
        "translation/1"
    );
    assert_eq!(
        merged
            .words
            .iter()
            .map(|word| (
                word.id.word,
                word.text.as_ref().unwrap().as_str(),
                word.output
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, "Một", range(0, 100)),
            (1, "hai", range(100, 200)),
            (2, "ba", range(400, 500)),
            (3, "bốn", range(500, 600)),
        ],
        "word timings concatenate in cue order"
    );
    assert_eq!(view.cues[1].id.as_str(), "câu-3");
    assert_eq!(view.cues[1].output, range(900, 1200));

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    assert_eq!(
        s.transcript().unwrap().cues[0].output,
        range(0, 900),
        "redo restores the merge"
    );

    // A run of more than two adjacent cues is one merge.
    s.execute(
        "transcript.merge_cues",
        json!({"cue_ids":[{"item":4,"cue":"câu-3"},{"item":4,"cue":"câu-1"}]}),
    )
    .unwrap();
    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 1);
    assert_eq!(view.cues[0].output, range(0, 1200));
    assert_eq!(view.cues[0].words.len(), 5);
    assert_eq!(
        view.cues[0].texts[&captions::LanguageTag::parse("vi").unwrap()]
            .text
            .as_str(),
        "Một hai ba bốn năm"
    );
}

#[test]
fn transcript_merge_joins_text_by_language_word_spacing() {
    let mut s = merge_project(false, false);
    s.execute(
        "transcript.merge_cues",
        json!({"cue_ids":[{"item":4,"cue":"câu-2"},{"item":4,"cue":"câu-1"}]}),
    )
    .unwrap();
    // The merged cue carries several scripts; each language keeps its own rule.
    let merged = s
        .transcript()
        .unwrap()
        .cues
        .first()
        .cloned()
        .expect("one merged cue");
    for (language, expected) in [
        ("vi", "Một hai ba bốn"),
        ("en", "one two three four"),
        ("zh", "你好世界"),
        ("ja", "こんにちは世界"),
    ] {
        let tag = captions::LanguageTag::parse(language).unwrap();
        assert_eq!(
            merged.texts.get(&tag).map(|text| text.text.as_str()),
            Some(expected),
            "{language} join"
        );
    }
}

#[test]
fn transcript_merge_after_a_trim_maps_through_the_retained_source() {
    let mut s = merge_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let outcome = s
        .execute(
            "batch",
            json!({"commands":[
                {"command":"trim_start","params":{"item":4,"at":400}},
                {"command":"transcript.merge_cues","params":{"cue_ids":[
                    {"item":4,"cue":"câu-3"},{"item":4,"cue":"câu-2"}
                ]}}
            ]}),
        )
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "the batch is one undo step");
    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 1, "the trim drops cue-1 before the merge");
    assert_eq!(view.cues[0].id.as_str(), "câu-2");
    assert_eq!(view.cues[0].output, range(400, 1200));
    assert_eq!(
        view.cues[0].texts[&captions::LanguageTag::parse("vi").unwrap()]
            .text
            .as_str(),
        "ba bốn năm"
    );
    assert_eq!(
        view.cues[0]
            .words
            .iter()
            .map(|word| (word.id.word, word.output))
            .collect::<Vec<_>>(),
        vec![
            (0, range(400, 500)),
            (1, range(500, 600)),
            (2, range(900, 1000))
        ]
    );
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}

#[test]
fn transcript_merge_refuses_non_adjacent_cross_track_cross_item_and_protected_atomically() {
    let cue = |item: u64, cue: &str| json!({"item":item,"cue":cue});
    let mut plain = merge_project(false, false);
    let before = project::encode(plain.project(), false).unwrap();
    let revision = plain.revision();
    let mut refused = |cue_ids: Value| {
        let error = plain
            .execute("transcript.merge_cues", json!({"cue_ids": cue_ids}))
            .unwrap_err();
        assert_unchanged(&plain, &before, &revision);
        error
    };
    assert!(matches!(
        refused(json!([])),
        EditError::Invalid("merge cue count")
    ));
    assert!(matches!(
        refused(json!([cue(4, "câu-1")])),
        EditError::Invalid("merge cue count")
    ));
    assert!(matches!(
        refused(json!([cue(4, "câu-1"), cue(4, "câu-1")])),
        EditError::Invalid("duplicate merge cue")
    ));
    assert!(matches!(
        refused(json!([cue(4, "câu-1"), cue(4, "khác")])),
        EditError::Invalid("merge cue")
    ));
    assert!(matches!(
        refused(json!([cue(77, "câu-1"), cue(4, "câu-1")])),
        EditError::NoItem(ItemId(77))
    ));
    assert!(matches!(
        refused(json!([cue(4, "câu-1"), cue(4, "câu-3")])),
        EditError::CuesNotAdjacent
    ));

    // Over-budget selections refuse before resolving any cue.
    let many: Vec<Value> = (0..=engine::control::MAX_COLLECTION)
        .map(|_| cue(4, "câu-1"))
        .collect();
    assert!(matches!(
        plain.execute("transcript.merge_cues", json!({"cue_ids":many})),
        Err(EditError::Limit)
    ));
    assert_unchanged(&plain, &before, &revision);

    // Cues on different tracks refuse with a typed reason.
    let mut data = plain.project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(11),
        name: "Other captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            8,
            0,
            (0, 100),
            vec![word_cue("khác-1", 0, 100, &[], false)],
        )],
    });
    let mut cross_track = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(cross_track.project(), false).unwrap();
    let revision = cross_track.revision();
    assert!(matches!(
        cross_track.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(4, "câu-1"), cue(8, "khác-1")]})
        ),
        Err(EditError::CuesAcrossTracks)
    ));
    assert_unchanged(&cross_track, &bytes, &revision);

    // Cues of two items on one track refuse too.
    let mut data = plain.project().data().clone();
    let item = caption_words_item(
        8,
        1200,
        (0, 100),
        vec![word_cue("khác-1", 0, 100, &[], false)],
    );
    data.scene_mut()
        .timeline
        .iter_mut()
        .find(|track| track.id == TrackId(9))
        .unwrap()
        .items
        .push(item);
    let mut cross_item = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(cross_item.project(), false).unwrap();
    let revision = cross_item.revision();
    assert!(matches!(
        cross_item.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(4, "câu-1"), cue(8, "khác-1")]})
        ),
        Err(EditError::CuesAcrossItems)
    ));
    assert_unchanged(&cross_item, &bytes, &revision);

    // An edited source text protects every selected cue.
    let mut protected = merge_project(true, false);
    let bytes = project::encode(protected.project(), false).unwrap();
    let revision = protected.revision();
    assert!(matches!(
        protected.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(4, "câu-1"), cue(4, "câu-2")]})
        ),
        Err(EditError::Protected)
    ));
    assert_unchanged(&protected, &bytes, &revision);

    // A locked caption track refuses the edit.
    let mut locked = merge_project(false, true);
    let bytes = project::encode(locked.project(), false).unwrap();
    let revision = locked.revision();
    assert!(matches!(
        locked.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(4, "câu-1"), cue(4, "câu-2")]})
        ),
        Err(EditError::Locked(_))
    ));
    assert_unchanged(&locked, &bytes, &revision);

    // Mixed or broken word timing cannot be merged without losing alignment.
    let mut data = merge_project(false, false).project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if let ItemParams::Caption(params) = &mut item.content {
                params.cues.cues[2].word_ranges.clear();
            }
        }
    }
    let mut untimed = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(untimed.project(), false).unwrap();
    let revision = untimed.revision();
    assert!(matches!(
        untimed.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(4, "câu-2"), cue(4, "câu-3")]})
        ),
        Err(EditError::Invalid("merge cue word timings"))
    ));
    assert_unchanged(&untimed, &bytes, &revision);

    // Different source languages cannot share one merged source text.
    let mut data = merge_project(false, false).project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if let ItemParams::Caption(params) = &mut item.content {
                params.cues.cues[1].source_lang = captions::LanguageTag::parse("zh").unwrap();
            }
        }
    }
    let mut mixed = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(mixed.project(), false).unwrap();
    let revision = mixed.revision();
    assert!(matches!(
        mixed.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(4, "câu-1"), cue(4, "câu-2")]})
        ),
        Err(EditError::Invalid("merge cue source language"))
    ));
    assert_unchanged(&mixed, &bytes, &revision);

    // A cue is addressable only while its scene is active.
    let mut data = merge_project(false, false).project().data().clone();
    let mut other = data.scene().clone();
    other.id = project::SceneId(7);
    other.name = "Khác".into();
    other.is_main = false;
    other.timeline = Default::default();
    other.timeline.main.id = TrackId(30);
    other.timeline.insert_track(Track {
        id: TrackId(20),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            90,
            0,
            (0, 100),
            vec![word_cue("khác-2", 0, 100, &[], false)],
        )],
    });
    let main = data.scene().clone();
    data.scenes = project::Scenes::try_from(vec![main, other]).unwrap();
    let mut two_scenes = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(two_scenes.project(), false).unwrap();
    let revision = two_scenes.revision();
    assert!(matches!(
        two_scenes.execute(
            "transcript.merge_cues",
            json!({"cue_ids":[cue(90, "khác-2"), cue(4, "câu-1")]})
        ),
        Err(EditError::SceneBoundary)
    ));
    assert_unchanged(&two_scenes, &bytes, &revision);
}

// --- Lexicon filler cuts --------------------------------------------------

/// One English cue with an explicit text and measured word ranges.
fn english_cue(
    id: &str,
    start: i64,
    end: i64,
    text: &str,
    words: &[(i64, i64)],
    edited: bool,
) -> captions::Cue {
    let language = captions::LanguageTag::parse("en").unwrap();
    captions::Cue {
        id: id.into(),
        range: range(start, end),
        source_lang: language.clone(),
        texts: std::collections::BTreeMap::from([(
            language,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        word_ranges: words.iter().map(|&(a, b)| range(a, b)).collect(),
        style: None,
    }
}

/// One video clip and a caption item whose English text carries filled pauses and
/// a discourse marker. Cue-2's source text is optionally protected.
fn filler_project(edited: bool, locked: bool) -> Session {
    let mut data = project().into_data();
    word_asset(&mut data, 2400);
    data.scene_mut().timeline.main.items = vec![clip_item(5, 0, (0, 2400))];
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked,
        items: vec![caption_words_item(
            4,
            0,
            (0, 2400),
            vec![
                english_cue(
                    "câu-1",
                    0,
                    800,
                    "um hello uh there",
                    &[(0, 100), (100, 200), (200, 300), (300, 400)],
                    false,
                ),
                english_cue(
                    "câu-2",
                    800,
                    1600,
                    "like world",
                    &[(800, 900), (900, 1000)],
                    edited,
                ),
            ],
        )],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

#[test]
fn remove_fillers_cuts_selected_candidates_as_one_undo_step() {
    let mut s = filler_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let outcome = s
        .execute(
            "timeline.remove_fillers",
            json!({"ids":["filler:4:câu-1:0","filler:4:câu-2:800"],"confirm_edited":false}),
        )
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "one undo step per removal");
    assert_ne!(s.revision(), revision);

    // Both spans leave the single clip cut twice and the tails closed.
    let layout: Vec<(Tick, Tick)> = s
        .project()
        .data()
        .scene()
        .timeline
        .main
        .items
        .iter()
        .map(|item| (item.start, item.end().unwrap()))
        .collect();
    assert_eq!(layout, vec![(Tick(0), Tick(700)), (Tick(700), Tick(2200))]);

    // The removed words are gone; the words beside them keep their text and order.
    let view = s.transcript().unwrap();
    let words: Vec<(&str, usize, Tick, Tick)> = view
        .cues
        .iter()
        .flat_map(|cue| {
            cue.words.iter().map(move |word| {
                (
                    cue.id.as_str(),
                    word.id.word,
                    word.output.start(),
                    word.output.end(),
                )
            })
        })
        .collect();
    assert_eq!(
        words,
        vec![
            ("câu-1", 1, Tick(0), Tick(100)),
            ("câu-1", 2, Tick(100), Tick(200)),
            ("câu-1", 3, Tick(200), Tick(300)),
            ("câu-2", 1, Tick(700), Tick(800)),
        ]
    );

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    let edited = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&edited).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());

    // A nested batch child still cuts through the ripple path as one step.
    let mut batched = filler_project(false, false);
    batched
        .execute(
            "batch",
            json!({"commands":[{"command":"timeline.remove_fillers",
                "params":{"ids":["filler:4:câu-1:0","filler:4:câu-2:800"],"confirm_edited":false}}]}),
        )
        .unwrap();
    assert_eq!(project::encode(batched.project(), false).unwrap(), edited);
}

#[test]
fn remove_fillers_refuses_edited_unknown_stale_and_hostile_ids_atomically() {
    let mut plain = filler_project(false, false);
    let before = project::encode(plain.project(), false).unwrap();
    let revision = plain.revision();
    let mut refused = |ids: Value| {
        let error = plain
            .execute(
                "timeline.remove_fillers",
                json!({"ids": ids, "confirm_edited": false}),
            )
            .unwrap_err();
        assert_unchanged(&plain, &before, &revision);
        error
    };
    assert!(matches!(
        refused(json!([])),
        EditError::Invalid("empty filler selection")
    ));
    assert!(matches!(
        refused(json!(["filler:4:câu-1:0", "filler:4:câu-1:0"])),
        EditError::Invalid("duplicate filler")
    ));
    // A start that is not a measured word, an unknown cue, item and shape.
    for id in [
        "filler:4:câu-1:50",
        "filler:4:khác:0",
        "filler:77:câu-1:0",
        "nope",
        "",
    ] {
        assert!(
            matches!(
                refused(json!([id])),
                EditError::Invalid("stale filler candidate")
            ),
            "{id}"
        );
    }
    let many: Vec<Value> = (0..=engine::control::MAX_COLLECTION)
        .map(|_| json!("filler:4:câu-1:0"))
        .collect();
    assert!(matches!(
        plain.execute(
            "timeline.remove_fillers",
            json!({"ids": many, "confirm_edited": false})
        ),
        Err(EditError::Limit)
    ));
    assert_unchanged(&plain, &before, &revision);

    // An edited cue's candidate needs confirmation.
    let mut protected = filler_project(true, false);
    let before = project::encode(protected.project(), false).unwrap();
    let revision = protected.revision();
    assert!(matches!(
        protected.execute(
            "timeline.remove_fillers",
            json!({"ids":["filler:4:câu-2:800"],"confirm_edited":false})
        ),
        Err(EditError::Protected)
    ));
    assert_unchanged(&protected, &before, &revision);
    protected
        .execute(
            "timeline.remove_fillers",
            json!({"ids":["filler:4:câu-2:800"],"confirm_edited":true}),
        )
        .unwrap();
    assert_eq!(protected.history_len(), (1, 0));
    assert_eq!(
        protected
            .transcript()
            .unwrap()
            .cues
            .iter()
            .flat_map(|cue| &cue.words)
            .filter(|word| word.id.cue.as_str() == "câu-2")
            .count(),
        1,
        "only like is removed; world stays"
    );

    // A trim removes the word from the visible output, so its old id is stale.
    let mut trimmed = filler_project(false, false);
    trimmed
        .execute("trim_start", json!({"item":4,"at":150}))
        .unwrap();
    let bytes = project::encode(trimmed.project(), false).unwrap();
    let revision = trimmed.revision();
    assert!(matches!(
        trimmed.execute(
            "timeline.remove_fillers",
            json!({"ids":["filler:4:câu-1:0"],"confirm_edited":false})
        ),
        Err(EditError::Invalid("stale filler candidate"))
    ));
    assert_eq!(project::encode(trimmed.project(), false).unwrap(), bytes);
    assert_eq!(trimmed.revision(), revision);
    assert_eq!(trimmed.history_len(), (1, 0), "the trim stays one step");
    // The visible part of the word keeps the same id.
    let mut clipped = filler_project(false, false);
    clipped
        .execute("trim_start", json!({"item":4,"at":50}))
        .unwrap();
    clipped
        .execute(
            "timeline.remove_fillers",
            json!({"ids":["filler:4:câu-1:0"],"confirm_edited":false}),
        )
        .unwrap();

    // A locked caption track refuses the whole edit.
    let mut locked = filler_project(false, true);
    let before = project::encode(locked.project(), false).unwrap();
    let revision = locked.revision();
    assert!(matches!(
        locked.execute(
            "timeline.remove_fillers",
            json!({"ids":["filler:4:câu-1:0"],"confirm_edited":false})
        ),
        Err(EditError::Locked(_))
    ));
    assert_unchanged(&locked, &before, &revision);
}

#[test]
fn remove_fillers_accepts_a_classified_non_lexicon_word() {
    let mut s = filler_project(false, false);
    // `hello` is a visible measured word but not a lexicon filler; LLM
    // classification may return it and removal must apply that reviewed id.
    s.execute(
        "timeline.remove_fillers",
        json!({"ids":["filler:4:câu-1:100"],"confirm_edited":false}),
    )
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));
    let words: Vec<String> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .filter_map(|word| word.text.as_ref().map(|text| text.as_str().to_owned()))
        .collect();
    assert_eq!(words, ["um", "uh", "there", "like", "world"]);
}

#[test]
fn remove_fillers_keeps_five_clip_audio_video_aligned() {
    let second = time::TICKS_PER_SECOND;
    let tenth = second / 10;
    let mut data = five_clip_pairs().project().data().clone();
    word_asset(&mut data, 12 * second);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(20),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            40,
            0,
            (0, 12 * second),
            vec![english_cue(
                "câu-1",
                0,
                12 * second,
                "um hello",
                &[
                    (second, second + tenth),
                    (second + tenth, second + 2 * tenth),
                ],
                false,
            )],
        )],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = project::encode(s.project(), false).unwrap();
    let id = format!("filler:40:câu-1:{second}");
    s.execute(
        "timeline.remove_fillers",
        json!({"ids":[id],"confirm_edited":false}),
    )
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));

    let starts = |kind: TrackKind| -> Vec<Tick> {
        let mut starts: Vec<Tick> = s
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == kind)
            .flat_map(|track| track.items.iter().map(|item| item.start))
            .collect();
        starts.sort_unstable();
        starts
    };
    assert_eq!(starts(TrackKind::Video), starts(TrackKind::Audio));
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .last()
            .unwrap()
            .end()
            .unwrap(),
        Tick(12 * second - tenth)
    );
    let words: Vec<usize> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .map(|word| word.id.word)
        .collect();
    assert_eq!(words, vec![1], "the filler word alone is cut");

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}
