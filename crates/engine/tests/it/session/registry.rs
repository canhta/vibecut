#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{content, get, new_item, project, range, session};
use engine::{Command, CreatedId, EditError, NewItem, Placement, Session, registry};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use project::{Color, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind};
use proptest::prelude::*;
use serde_json::json;
use std::sync::Arc;
use time::Tick;

#[test]
fn all_commands_share_registry_and_strict_json_dispatch() {
    let mut s = Session::new(Project::empty(ProjectId(0), "".into()).unwrap(), 10).unwrap();
    let r = s
        .apply(Command::AddTrack {
            name: "Tiếng".into(),
            kind: TrackKind::Video,
            placement: Default::default(),
        })
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
        .apply(Command::InsertItem {
            placement: Placement::Track { track: TrackId(1) },
            item: new_item(0, 10),
        })
        .unwrap();
    assert_eq!(r.created, vec![CreatedId::Item(ItemId(2))]);
    s.apply(Command::MoveItem {
        item: ItemId(2),
        track: TrackId(1),
        start: Tick(10),
    })
    .unwrap();
    s.apply(Command::SetItemParam {
        item: ItemId(2),
        content: content(),
    })
    .unwrap();
    s.apply(Command::BookmarkAdd {
        start: Tick(1),
        duration: Tick(0),
        note: "ghi chú".into(),
        color: Color([0, 0, 0, 255]),
    })
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
    s.apply(Command::AddAsset { asset }).unwrap();
    s.apply(Command::Delete {
        items: vec![ItemId(2)],
    })
    .unwrap();
    s.apply(Command::SetTrackFlags {
        track: TrackId(1),
        muted: true,
        hidden: true,
        locked: true,
    })
    .unwrap();
    assert!(matches!(
        s.apply(Command::RemoveTrack { track: TrackId(1) }),
        Err(EditError::Locked(_))
    ));
    s.apply(Command::SetTrackFlags {
        track: TrackId(1),
        muted: false,
        hidden: false,
        locked: false,
    })
    .unwrap();
    s.apply(Command::RemoveTrack { track: TrackId(1) }).unwrap();
    for spec in registry() {
        assert!(!spec.label.is_empty());
        assert_eq!(registry().iter().filter(|r| r.id == spec.id).count(), 1);
        assert_eq!(spec.params_schema["type"], "object");
    }
    let undo = registry().iter().find(|r| r.id == "undo").unwrap();
    assert!(undo.enabled(&s));
    s.apply(Command::Undo {}).unwrap();
    assert!(
        registry()
            .iter()
            .find(|r| r.id == "redo")
            .unwrap()
            .enabled(&s)
    );
    s.apply(Command::Redo {}).unwrap();
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
        assert!(crate::wire::run(&mut s, id, params).is_err(), "{id}");
        assert!(Arc::ptr_eq(&before, s.project()));
        assert_eq!(s.history_len(), (0, 0));
    }
    let mut nested = json!({});
    for _ in 0..40 {
        nested = json!([nested]);
    }
    assert!(matches!(
        crate::wire::run(&mut s, "batch", nested),
        Err(engine::control::Error::Control(
            engine::control::ControlError::Limit
        ))
    ));
    let command = Command::Batch {
        commands: vec![Command::Delete { items: vec![] }; 1025],
    };
    assert!(matches!(
        s.apply(command),
        Err(EditError::Control(engine::control::ControlError::Limit))
    ));
    assert!(Session::new(project(), 0).is_err());
    assert!(Session::new(project(), 1001).is_err());
    for last_id_available in [false, true] {
        let mut data = project().into_data();
        data.scene_mut().timeline.overlays.first_mut().unwrap().id =
            TrackId(if last_id_available {
                u64::MAX - 1
            } else {
                u64::MAX
            });
        let mut s = Session::new(Project::new(data).unwrap(), 1).unwrap();
        let add = || Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "".into(),
            kind: TrackKind::Video,
        };
        if last_id_available {
            let result = s.apply(add()).unwrap();
            assert_eq!(result.created, vec![CreatedId::Track(TrackId(u64::MAX))]);
            s.undo().unwrap();
        }
        let before = (s.project().clone(), s.history_len(), s.revision());
        s.take_change();
        assert!(matches!(
            s.apply(add()),
            Err(EditError::Session(engine::SessionError::IdExhausted))
        ));
        assert_eq!((s.project().clone(), s.history_len(), s.revision()), before);
        assert!(s.take_change().is_none());
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn arbitrary_hostile_json_never_changes_state_on_error(id in ".{0,20}", text in ".{0,100}", value in any::<i64>()) {
        let mut s=session(); let before=s.project().clone();
        let params=json!({"item":value,"start":text,"track":value});
        prop_assert!(crate::wire::run(&mut s, &id, params).is_err());
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
                    fit: Default::default(),
                    cleanup: Default::default(),
                    background: project::ClipBackground::None,
                    reduce_noise: false,
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
                cleanup: Default::default(),
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
                    cleanup: Default::default(),
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
                    cleanup: Default::default(),
                }),
            },
        },
    ] {
        assert!(s.apply(command).is_err());
        assert!(Arc::ptr_eq(&snapshot, s.project()));
        assert_eq!(s.history_len(), history);
    }
}
