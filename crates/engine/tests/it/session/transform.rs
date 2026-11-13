#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{get, project, range, session};
use engine::{Command, ItemParamPatch, Session};
use media::{AssetId, ContentHash};
use project::{Color, Item, ItemId, ItemParams, Project};
use time::Tick;

fn transform_patch(field: project::TransformField, value: f64) -> ItemParamPatch {
    match field {
        project::TransformField::PositionX => ItemParamPatch::PositionX(value),
        project::TransformField::PositionY => ItemParamPatch::PositionY(value),
        project::TransformField::ScaleX => ItemParamPatch::ScaleX(value),
        project::TransformField::ScaleY => ItemParamPatch::ScaleY(value),
        project::TransformField::Rotate => ItemParamPatch::Rotate(value),
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
                    .apply(Command::SetItemField {
                        item: ItemId(10),
                        patch: transform_patch(field, invalid),
                    })
                    .is_err()
            );
            assert_eq!(session.project(), &before);
        }
        session
            .apply(Command::SetItemField {
                item: ItemId(10),
                patch: transform_patch(field, max),
            })
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
        .apply(Command::SetBackground {
            color: Color([12, 34, 56, 78]),
        })
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
            cleanup: Default::default(),
            background: project::ClipBackground::None,
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
    s.apply(Command::SetItemField {
        item: ItemId(20),
        patch: ItemParamPatch::Crop(Some(centred)),
    })
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
    s.apply(Command::TrimStart {
        item: ItemId(20),
        at: Tick(50),
    })
    .unwrap();
    assert_eq!(crop(&s), Some(centred));
    s.apply(Command::TrimEnd {
        item: ItemId(20),
        at: Tick(400),
    })
    .unwrap();
    assert_eq!(crop(&s), Some(centred));
    s.apply(Command::SetSpeed {
        items: vec![ItemId(20)],
        speed: time::Speed::from_f64(2.0).unwrap(),
        maintain_pitch: true,
    })
    .unwrap();
    assert_eq!(crop(&s), Some(centred));
    s.apply(Command::SetItemField {
        item: ItemId(20),
        patch: ItemParamPatch::Crop(None),
    })
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
