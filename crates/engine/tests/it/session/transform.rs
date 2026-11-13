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
            fit: Default::default(),
            cleanup: Default::default(),
            background: project::ClipBackground::None,
            reduce_noise: false,
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

/// A 1000x1000 canvas holding one 2000x1000 video item (id 20) with the given
/// animation channels; the shared fixture keeps its non-video item 10.
fn fit_session(animations: project::Animations) -> Session {
    let mut data = project().into_data();
    data.canvas.width = 1000;
    data.canvas.height = 1000;
    let asset = testkit::source::asset(7, std::env::temp_dir().join("fit-controls.mp4"))
        .format("mp4")
        .duration(Tick(1000))
        .content_hash(ContentHash::from_bytes(&[7]))
        .stream(
            testkit::source::video()
                .duration(Tick(1000))
                .start(Tick::ZERO)
                .size(2000, 1000)
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
        animations,
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
            fit: Default::default(),
            cleanup: Default::default(),
            reduce_noise: false,
            background: project::ClipBackground::None,
        }),
    }];
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
fn fit_of(s: &Session) -> project::FitMode {
    match &get(s, 20).content {
        ItemParams::Video(p) => p.fit,
        _ => panic!("video fixture"),
    }
}
fn set_fit(s: &mut Session, mode: project::FitMode) -> Result<(), engine::EditError> {
    s.apply(Command::SetItemField {
        item: ItemId(20),
        patch: ItemParamPatch::Fit(mode),
    })
    .map(|_| ())
}
fn scale_of(s: &Session) -> [f64; 2] {
    [
        project::TransformField::ScaleX,
        project::TransformField::ScaleY,
    ]
    .map(|field| field.value_at(&get(s, 20).animations, Tick::ZERO).unwrap())
}

#[test]
fn fit_patch_is_undoable_persisted_video_only_and_rejects_unknown_values() {
    use project::FitMode as M;
    let mut s = fit_session(Default::default());
    let initial = s.project().clone();
    assert_eq!(fit_of(&s), M::Fill);
    for (index, mode) in [M::Fit, M::Stretch, M::None, M::Fill]
        .into_iter()
        .enumerate()
    {
        set_fit(&mut s, mode).unwrap();
        assert_eq!(fit_of(&s), mode);
        let saved = project::encode(s.project(), false).unwrap();
        assert_eq!(
            &project::decode(&saved).unwrap().project,
            s.project().as_ref(),
            "{mode:?}"
        );
        assert_eq!(s.history_len().0, index + 1);
        // The fit is policy: the transform channels stay untouched.
        assert_eq!(scale_of(&s), [1.0, 1.0]);
    }
    s.undo().unwrap();
    assert_eq!(fit_of(&s), M::None);
    s.redo().unwrap();
    assert_eq!(fit_of(&s), M::Fill);
    while s.history_len().0 > 0 {
        s.undo().unwrap();
    }
    assert_eq!(s.project(), &initial);

    // The shared edit applies the same patch to a list of items.
    s.apply(Command::SetItemParamMany {
        items: vec![ItemId(20)],
        patch: ItemParamPatch::Fit(M::Stretch),
    })
    .unwrap();
    assert_eq!(fit_of(&s), M::Stretch);

    // Text, audio and other items have no fit mode; unknown names fail to parse.
    let before = s.project().clone();
    assert!(
        s.apply(Command::SetItemField {
            item: ItemId(10),
            patch: ItemParamPatch::Fit(M::Fit),
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    let parsed: ItemParamPatch =
        serde_json::from_value(serde_json::json!({"field":"fit","value":"stretch"})).unwrap();
    assert_eq!(parsed, ItemParamPatch::Fit(M::Stretch));
    for name in ["zoom", "Fit", "", "contain"] {
        assert!(
            serde_json::from_value::<ItemParamPatch>(
                serde_json::json!({"field":"fit","value":name})
            )
            .is_err(),
            "{name}"
        );
    }
}

#[test]
fn manual_fit_records_the_computed_fit_into_the_scale_and_refuses_what_cannot_hold_it() {
    use project::FitMode as M;
    // A 2:1 source in a square canvas: Fit shows it at full width, half height.
    let mut s = fit_session(Default::default());
    let initial = s.project().clone();
    set_fit(&mut s, M::Fit).unwrap();
    set_fit(&mut s, M::Manual).unwrap();
    assert_eq!(fit_of(&s), M::Manual);
    assert_eq!(scale_of(&s), [1.0, 0.5]);
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.project(), &initial);

    // Fill covers the square: the 2:1 source is twice as wide as the canvas.
    // An authored scale multiplies the recorded fit.
    s.apply(Command::SetItemField {
        item: ItemId(20),
        patch: ItemParamPatch::ScaleX(1.5),
    })
    .unwrap();
    set_fit(&mut s, M::Manual).unwrap();
    assert_eq!(scale_of(&s), [3.0, 1.0]);

    // A recorded scale beyond the transform bound refuses the whole edit.
    let mut s = fit_session(Default::default());
    s.apply(Command::SetItemField {
        item: ItemId(20),
        patch: ItemParamPatch::ScaleX(6.0),
    })
    .unwrap();
    let before = s.project().clone();
    assert!(set_fit(&mut s, M::Manual).is_err());
    assert_eq!(s.project(), &before);
    assert_eq!(fit_of(&s), M::Fill);

    // A keyed scale cannot take the recorded fit.
    let mut keyed = project::Animations::default();
    let mut key = project::Keyframe::new(Tick::ZERO, project::ParamValue::Scalar(1.0));
    key.segment = project::Segment::Linear;
    let channel = project::Channel::new(
        project::ParamValue::Scalar(1.0),
        vec![key],
        project::Extrapolation::Hold,
        project::Extrapolation::Hold,
    )
    .unwrap();
    keyed
        .set(
            project::AnimationPath::from(project::BuiltinPath::TransformScaleX),
            project::Param::Value(channel),
        )
        .unwrap();
    let mut s = fit_session(keyed);
    let before = s.project().clone();
    assert!(set_fit(&mut s, M::Manual).is_err());
    assert_eq!(s.project(), &before);
    // Other modes still apply to an item with keyed scale.
    set_fit(&mut s, M::Fit).unwrap();
    assert_eq!(fit_of(&s), M::Fit);
}
