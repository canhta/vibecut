#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use project::*;

const OPTIONS: &[&str] = &["off", "low", "high"];
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
                !param.animatable || param.scalar_range().is_some(),
                "{}.{} animates a non-scalar",
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
