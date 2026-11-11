#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use project::*;
use time::{SourceRange, Tick};

const OPTIONS: &[&str] = &["off", "low", "high"];

#[test]
fn lut_model_refuses_unregistered_references_and_keeps_current_wire_domains() {
    let id = LutId::new(media::ContentHash::from_bytes(b"canonical"));
    for value in [
        serde_json::json!({}),
        serde_json::json!({"lut":"abc"}),
        serde_json::json!({"lut":id.hex().to_uppercase()}),
        serde_json::json!({"lut":null,"extra":1}),
        serde_json::json!("4"),
    ] {
        assert!(serde_json::from_value::<EffectParamUpdate>(value).is_err());
    }
    assert_eq!(
        serde_json::from_str::<EffectParamUpdate>("4").unwrap(),
        EffectParamUpdate::Scalar(4.)
    );
    assert_eq!(
        serde_json::from_str::<EffectParamUpdate>("{\"lut\":null}").unwrap(),
        EffectParamUpdate::LutRef(LutReference { lut: None })
    );
    let effect = effect_definition("color_grade")
        .unwrap()
        .instance("grade")
        .with("lut", EffectValue::Lut(Some(id)))
        .unwrap();
    assert!(matches!(
        project_with_effects(vec![effect.clone()]),
        Err(Error::Invalid("unregistered LUT"))
    ));
    let mut data = project_with_effects(vec![]).unwrap().into_data();
    let record = LutRecord {
        id,
        name: "Look".into(),
        licence: Some("User licence".into()),
        source_name: "look.cube".into(),
        source_hash: media::ContentHash::from_bytes(b"raw"),
        directory: std::env::temp_dir().join("immutable-lut"),
    };
    data.luts.push(record.clone());
    data.scene_mut().timeline.overlays[0].items[0]
        .effects
        .push(effect);
    let project = Project::new(data.clone()).unwrap();
    assert_eq!(
        project::decode(&project::encode(&project, false).unwrap())
            .unwrap()
            .project,
        project
    );
    data.luts.push(record);
    assert!(matches!(
        Project::new(data),
        Err(Error::Invalid("duplicate LUT"))
    ));
}

/// A real project candidate carrying exactly these effects on one text item.
fn project_with_effects(effects: Vec<EffectInstance>) -> project::Result<Project> {
    let mut data = Project::empty(ProjectId(1), "effects".into())?.into_data();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Text".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects,
            animations: Default::default(),
            id: ItemId(1),
            hidden: false,
            source_asset: None,
            source_range: SourceRange::new(Tick(0), Tick(10)).unwrap(),
            start: Tick(0),
            retime: Default::default(),
            content: ItemParams::Text(TextParams::new("id")),
        }],
    });
    Project::new(data)
}
fn param(kind: EffectParamKind, default: EffectValue) -> EffectParamDefinition {
    EffectParamDefinition {
        id: "p",
        name: "P",
        kind,
        default,
        animatable: false,
    }
}

#[test]
fn every_parameter_kind_accepts_its_range_and_refuses_the_rest() {
    let float = EffectParamKind::Float {
        min: -2.0,
        max: 4.0,
        soft_min: 0.0,
        soft_max: 1.0,
    };
    let s = EffectValue::Scalar;
    let color = EffectValue::Color(Color([1, 2, 3, 255]));
    // (kind, value, accepted)
    let rows = [
        (float, s(-2.0), true),
        (float, s(4.0), true),
        (float, s(4.5), false),
        (float, s(-2.5), false),
        (float, s(f64::NAN), false),
        (float, s(f64::INFINITY), false),
        (float, EffectValue::Bool(true), false),
        (EffectParamKind::Angle, s(-ANGLE_MAX), true),
        (EffectParamKind::Angle, s(ANGLE_MAX), true),
        (EffectParamKind::Angle, s(ANGLE_MAX + 0.5), false),
        (
            EffectParamKind::Choice { options: OPTIONS },
            EffectValue::Choice(2),
            true,
        ),
        (
            EffectParamKind::Choice { options: OPTIONS },
            EffectValue::Choice(3),
            false,
        ),
        (EffectParamKind::Choice { options: OPTIONS }, s(1.0), false),
        (EffectParamKind::Bool, EffectValue::Bool(false), true),
        (EffectParamKind::Bool, EffectValue::Choice(0), false),
        (EffectParamKind::Color, color, true),
        (EffectParamKind::Color, s(0.0), false),
        (
            EffectParamKind::Point,
            EffectValue::Point([0.0, POINT_MAX]),
            true,
        ),
        (
            EffectParamKind::Point,
            EffectValue::Point([0.5, 1.5]),
            false,
        ),
        (
            EffectParamKind::Point,
            EffectValue::Point([f64::NAN, 0.5]),
            false,
        ),
        (EffectParamKind::Point, color, false),
    ];
    for (kind, value, accepted) in rows {
        let result = param(kind, value).check(&value);
        assert_eq!(result.is_ok(), accepted, "{kind:?} {value:?}");
    }
}

#[test]
fn definitions_are_well_formed_and_their_defaults_validate() {
    let mut kinds = std::collections::BTreeSet::new();
    for definition in effect_definitions() {
        assert!(
            kinds.insert(definition.id),
            "duplicate kind {}",
            definition.id
        );
        let mut ids = std::collections::BTreeSet::new();
        for param in definition.params {
            assert!(
                ids.insert(param.id),
                "{} repeats {}",
                definition.id,
                param.id
            );
            param.check(&param.default).unwrap();
            assert!(
                !param.animatable
                    || param.scalar_range().is_some()
                    || param.kind == EffectParamKind::Color,
                "{}.{} animates an unsupported kind",
                definition.id,
                param.id
            );
        }
        let instance = definition.instance("one");
        instance.validate().unwrap();
        assert_eq!(effect_definition(definition.id).unwrap().id, definition.id);
        for id in definition.animated_ids() {
            instance.param_path(id).unwrap();
        }
    }
    assert!(effect_definition("nope").is_err());
}

#[test]
fn pixel_catalogue_keeps_source_numeric_admission_and_typed_colours() {
    // Source effect.rs f/fs definitions: hard ranges are independent of slider ranges.
    let mut rows = vec![
        ("color_pass", "similarity", 10.0, (0.0, 100.0), (0.0, 100.0)),
        (
            "color_replace",
            "similarity",
            10.0,
            (0.0, 100.0),
            (0.0, 100.0),
        ),
        ("extract", "black", 0.0, (0.0, 255.0), (0.0, 255.0)),
        ("extract", "white", 255.0, (0.0, 255.0), (0.0, 255.0)),
        ("extract", "softness", 0.0, (0.0, 100.0), (0.0, 100.0)),
        ("find_edges", "blend", 0.0, (0.0, 100.0), (0.0, 100.0)),
        ("color_emboss", "relief", 1.0, (0.0, 10.0), (0.0, 10.0)),
        (
            "color_emboss",
            "contrast",
            100.0,
            (0.0, 500.0),
            (0.0, 500.0),
        ),
        ("color_emboss", "blend", 0.0, (0.0, 100.0), (0.0, 100.0)),
        ("gamma_correction", "gamma", 10.0, (1.0, 28.0), (1.0, 28.0)),
        (
            "directional_blur",
            "length",
            0.0,
            (0.0, 1000.0),
            (0.0, 20.0),
        ),
        ("invert", "blend", 0.0, (0.0, 100.0), (0.0, 100.0)),
        ("levels", "in_black", 0.0, (0.0, 255.0), (0.0, 255.0)),
        ("levels", "in_white", 255.0, (0.0, 255.0), (0.0, 255.0)),
        ("levels", "out_black", 0.0, (0.0, 255.0), (0.0, 255.0)),
        ("levels", "out_white", 255.0, (0.0, 255.0), (0.0, 255.0)),
        ("levels", "gamma", 100.0, (10.0, 1000.0), (10.0, 300.0)),
        ("posterize", "levels", 7.0, (2.0, 255.0), (2.0, 32.0)),
        ("strobe", "blend", 0.0, (0.0, 100.0), (0.0, 100.0)),
        ("strobe", "duration", 0.05, (0.0, 30.0), (0.0, 1.0)),
        ("strobe", "period", 0.5, (0.0, 30.0), (0.0, 2.0)),
        ("tint", "amount", 100.0, (0.0, 100.0), (0.0, 100.0)),
    ];
    for id in [
        "rr", "rg", "rb", "rc", "gr", "gg", "gb", "gc", "br", "bg", "bb", "bc",
    ] {
        let default = if matches!(id, "rr" | "gg" | "bb") {
            100.0
        } else {
            0.0
        };
        rows.push(("channel_mix", id, default, (-200.0, 200.0), (-200.0, 200.0)));
    }
    for (kind, id, default, hard, soft) in rows {
        let def = effect_definition(kind).unwrap();
        let p = def.param(id).unwrap();
        assert_eq!(p.default, EffectValue::Scalar(default), "{kind}.{id}");
        assert_eq!(
            p.kind,
            EffectParamKind::Float {
                min: hard.0,
                max: hard.1,
                soft_min: soft.0,
                soft_max: soft.1
            },
            "{kind}.{id}"
        );
        assert!(p.animatable, "{kind}.{id}");
        for value in [hard.0, hard.1] {
            def.instance("one")
                .with(id, EffectValue::Scalar(value))
                .unwrap();
        }
        for value in [hard.0 - 0.01, hard.1 + 0.01] {
            assert!(
                def.instance("one")
                    .with(id, EffectValue::Scalar(value))
                    .is_err(),
                "{kind}.{id} {value}"
            );
        }
    }
    for (kind, id, value) in [
        ("color_pass", "reverse", EffectValue::Bool(false)),
        ("color_replace", "solid", EffectValue::Bool(false)),
        ("extract", "invert", EffectValue::Bool(false)),
        ("find_edges", "invert", EffectValue::Bool(false)),
        ("invert", "channel", EffectValue::Choice(0)),
        (
            "color_pass",
            "color",
            EffectValue::Color(Color([255, 0, 0, 255])),
        ),
        (
            "color_replace",
            "target",
            EffectValue::Color(Color([255, 0, 0, 255])),
        ),
        (
            "color_replace",
            "replace",
            EffectValue::Color(Color([0, 0, 255, 255])),
        ),
        ("strobe", "color", EffectValue::Color(Color([255; 4]))),
        ("tint", "black", EffectValue::Color(Color([0, 0, 0, 255]))),
        ("tint", "white", EffectValue::Color(Color([255; 4]))),
    ] {
        let def = effect_definition(kind).unwrap();
        let p = def.param(id).unwrap();
        assert_eq!(p.default, value, "{kind}.{id}");
        assert!(
            p.animatable == matches!(p.kind, EffectParamKind::Color),
            "source animation kind, {kind}.{id}"
        );
        def.instance("one").with(id, value).unwrap();
    }
    let channel = effect_definition("invert")
        .unwrap()
        .param("channel")
        .unwrap();
    assert_eq!(
        channel.kind,
        EffectParamKind::Choice {
            options: &["RGB", "Red", "Green", "Blue", "Alpha"]
        }
    );
    assert!(effect_definition("black_white").unwrap().params.is_empty());
    let relief = effect_definition("color_emboss").unwrap();
    let direction = relief.param("direction").unwrap();
    assert_eq!(
        direction.default,
        EffectValue::Scalar(45.0),
        "Relief default direction"
    );
    assert_eq!(direction.kind, EffectParamKind::Angle);
    assert!(direction.animatable);
    for (id, value) in [("relief", 0.0), ("relief", 10.0), ("contrast", 500.0)] {
        relief
            .instance("one")
            .with(id, EffectValue::Scalar(value))
            .unwrap();
    }
    for (id, value) in [("relief", -0.01), ("relief", 10.01), ("contrast", 500.01)] {
        assert!(
            relief
                .instance("one")
                .with(id, EffectValue::Scalar(value))
                .is_err(),
            "Relief {id} {value}"
        );
    }
}

#[test]
fn instance_ids_follow_the_shared_identifier_grammar_statically_and_animated() {
    let max = "a".repeat(64);
    let over = "a".repeat(65);
    // (id, accepted)
    let rows = [
        ("", false),
        (" ", false),
        ("bad id", false),
        ("a.b", false),
        ("a/b", false),
        ("é", false),
        (over.as_str(), false),
        ("a", true),
        (max.as_str(), true),
        ("1leading", true),
        ("_leading", true),
        ("-leading", true),
    ];
    for kind in ["color_grade", "blur"] {
        for (id, accepted) in rows {
            let instance = effect_definition(kind).unwrap().instance(id);
            let validated = instance.validate();
            if accepted {
                validated.unwrap();
                project_with_effects(vec![instance]).unwrap();
            } else {
                assert!(
                    matches!(validated, Err(Error::Invalid("effect instance id"))),
                    "{kind} {id:?}"
                );
                assert!(
                    matches!(
                        project_with_effects(vec![instance]),
                        Err(Error::Invalid("effect instance id"))
                    ),
                    "{kind} {id:?}"
                );
            }
        }
    }
}

#[test]
fn instance_id_errors_preserve_kind_and_duplicate_guards() {
    let mut unknown = effect_definition("color_grade").unwrap().instance("bad id");
    unknown.kind = "nope".into();
    assert!(matches!(
        unknown.validate(),
        Err(Error::Invalid("effect kind"))
    ));
    assert!(matches!(
        project_with_effects(vec![
            effect_definition("color_grade").unwrap().instance("same"),
            effect_definition("color_grade").unwrap().instance("same"),
        ]),
        Err(Error::Invalid("duplicate effect id"))
    ));
}

#[test]
fn instances_refuse_wrong_parameter_sets_and_set_scalar_is_atomic() {
    let blur = effect_definition("blur").unwrap().instance("blur1");
    let mut unknown_kind = blur.clone();
    unknown_kind.kind = "nope".into();
    let mut extra = blur.clone();
    extra
        .params
        .insert("extra".into(), EffectValue::Scalar(1.0));
    let mut missing = blur.clone();
    missing.params.clear();
    let mut wrong_type = blur.clone();
    wrong_type
        .params
        .insert("intensity".into(), EffectValue::Bool(true));
    let mut hostile = blur.clone();
    hostile
        .params
        .insert("intensity".into(), EffectValue::Scalar(f64::NAN));
    for (name, bad) in [
        ("unknown kind", unknown_kind),
        ("extra parameter", extra),
        ("missing parameter", missing),
        ("wrong type", wrong_type),
        ("hostile value", hostile),
    ] {
        assert!(bad.validate().is_err(), "{name}");
    }

    let mut chroma = effect_definition("chroma_key").unwrap().instance("key");
    let before = chroma.clone();
    for (id, value) in [
        ("intensity", 100.5),
        ("intensity", f64::NAN),
        ("key_color", 1.5),
        ("key_color", -1.0),
        ("key_color", PACKED_COLOR_MAX + 1.0),
        ("nope", 1.0),
    ] {
        assert!(chroma.set_scalar(id, value).is_err(), "{id} {value}");
        assert_eq!(chroma, before, "{id} {value} left the instance unchanged");
    }
    chroma.set_scalar("intensity", 100.0).unwrap();
    chroma
        .set_scalar("key_color", f64::from(0x11_22_33_u32))
        .unwrap();
    assert_eq!(chroma.value("intensity"), Some(&EffectValue::Scalar(100.0)));
    assert_eq!(
        chroma.value("key_color"),
        Some(&EffectValue::Color(Color([0x11, 0x22, 0x33, 255])))
    );
    chroma.validate().unwrap();
}

#[test]
fn instances_round_trip_through_json_with_typed_values() {
    for definition in effect_definitions() {
        let instance = definition.instance("one");
        let json = serde_json::to_value(&instance).unwrap();
        assert_eq!(json["kind"], definition.id);
        let back: EffectInstance = serde_json::from_value(json).unwrap();
        assert_eq!(back, instance);
    }
    let parsed: std::result::Result<EffectInstance, serde_json::Error> =
        serde_json::from_value(serde_json::json!({
            "id": "x", "kind": "blur", "enabled": true,
            "params": {"intensity": {"kind": "scalar", "value": 15.0}},
            "legacy": true
        }));
    assert!(parsed.is_err(), "unknown fields refuse");
}

#[test]
fn effect_colour_animation_requires_linear_composites() {
    use time::{SourceRange, Tick};
    let candidate = |effect: EffectInstance, path: AnimationPath, param: Param| {
        let mut data = super::project::empty().into_data();
        let mut animations = Animations::default();
        animations.set(path, param).unwrap();
        data.scene_mut().timeline.main.items.push(Item {
            id: ItemId(1),
            start: Tick::ZERO,
            source_range: SourceRange::new(Tick::ZERO, Tick(100)).unwrap(),
            source_asset: None,
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: vec![],
            effects: vec![effect],
            animations,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([255; 4]),
            }),
        });
        data
    };
    for (kind, key) in [
        ("color_pass", "color"),
        ("color_replace", "target"),
        ("color_replace", "replace"),
        ("tint", "black"),
        ("tint", "white"),
        ("strobe", "color"),
    ] {
        let effect = effect_definition(kind).unwrap().instance("one");
        let path = effect.param_path(key).unwrap();
        for (value, accepted) in [
            (ParamValue::Color([0.5, 0.0, 0.5, 0.25]), true),
            (ParamValue::Scalar(0.5), false),
            (ParamValue::Bool(true), false),
            (ParamValue::Text("#FF0000".into()), false),
        ] {
            let data = candidate(
                effect.clone(),
                path.clone(),
                Param::new(value.clone()).unwrap(),
            );
            let result = Project::new(data);
            assert_eq!(result.is_ok(), accepted, "{kind}.{key}: {value:?}");
            if !accepted {
                assert!(matches!(result, Err(Error::Invalid(_))));
            }
        }
        assert!(effect.definition().unwrap().param(key).unwrap().animatable);
        // The same composite remains valid after its final independent key is deleted.
        let mut param = Param::new(ParamValue::Color([0.2, 0.3, 0.4, 0.5])).unwrap();
        let channel = param.channel_mut(Some(ColorComponent::R)).unwrap();
        channel
            .put(Keyframe::new(Tick(1), ParamValue::Scalar(0.9)))
            .unwrap();
        channel.remove(Tick(1)).unwrap();
        let project = Project::new(candidate(effect, path.clone(), param)).unwrap();
        let item = &project.data().scene().timeline.main.items[0];
        assert_eq!(
            item.animations.value_at(&path, Tick(90)).unwrap(),
            Some(ParamValue::Color([0.2, 0.3, 0.4, 0.5]))
        );
    }
    let blur = effect_definition("blur").unwrap().instance("one");
    assert!(matches!(
        Project::new(candidate(
            blur.clone(),
            blur.param_path("intensity").unwrap(),
            Param::new(ParamValue::Color([0.5; 4])).unwrap()
        )),
        Err(Error::Invalid(_))
    ));
    let key = effect_definition("chroma_key").unwrap().instance("one");
    assert!(
        !key.definition()
            .unwrap()
            .param("key_color")
            .unwrap()
            .animatable
    );
    // Unresolved extension namespaces retain their admission instead of becoming effect parsers.
    Project::new(candidate(
        key,
        AnimationPath::new("params.unrelated").unwrap(),
        Param::new(ParamValue::Bool(true)).unwrap(),
    ))
    .unwrap();
    for raw in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, 1.01] {
        assert!(matches!(
            Param::new(ParamValue::Color([raw, 0.0, 0.0, 1.0])),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn streak_angle_and_scalar_animation_refuse_wrong_types_atomically() {
    let d = effect_definition("directional_blur").unwrap();
    assert_eq!(d.shelf, EffectShelf::BlurDetail);
    assert_eq!(d.param("direction").unwrap().kind, EffectParamKind::Angle);
    assert!(d.param("direction").unwrap().animatable);
    for (key, good, bad) in [
        ("direction", 360.0, 360.01),
        ("direction", -360.0, -360.01),
        ("length", 1000.0, -0.01),
    ] {
        let mut effect = d.instance("streak");
        effect.set_scalar(key, good).unwrap();
        let before = effect.clone();
        for value in [bad, f64::NAN, f64::INFINITY] {
            assert!(effect.set_scalar(key, value).is_err());
            assert_eq!(effect, before);
        }
        let mut data = project_with_effects(vec![effect.clone()])
            .unwrap()
            .into_data();
        data.scene_mut()
            .timeline
            .iter_mut()
            .find(|track| !track.items.is_empty())
            .unwrap()
            .items[0]
            .animations
            .set(
                effect.param_path(key).unwrap(),
                Param::new(ParamValue::Color([0.5; 4])).unwrap(),
            )
            .unwrap();
        assert!(matches!(
            Project::new(data),
            Err(Error::Invalid("effect animation type"))
        ));
    }
}
