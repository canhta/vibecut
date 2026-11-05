#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "validated animation fixtures"
)]
use project::*;
use proptest::prelude::*;
use time::{Rounding, Tick};
fn scalar(v: f64) -> ParamValue {
    ParamValue::Scalar(v)
}
fn value(v: ParamValue) -> f64 {
    if let ParamValue::Scalar(v) = v {
        v
    } else {
        panic!("scalar expected")
    }
}
fn key(t: i64, v: f64, s: Segment) -> Keyframe {
    let mut k = Keyframe::new(Tick(t), scalar(v));
    k.segment = s;
    k
}
fn channel(keys: Vec<Keyframe>, before: Extrapolation, after: Extrapolation) -> Channel {
    Channel::new(scalar(-1.0), keys, before, after).unwrap()
}
fn param(keys: Vec<Keyframe>) -> Param {
    Param::Value(channel(keys, Extrapolation::Hold, Extrapolation::Hold))
}
fn path() -> AnimationPath {
    AnimationPath::new("opacity").unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}
#[test]
fn source_handle_fixtures_modes_and_absolute_value_overshoot() {
    for mode in [
        TangentMode::Auto,
        TangentMode::Aligned,
        TangentMode::Broken,
        TangentMode::Flat,
    ] {
        let mut a = key(0, 0.0, Segment::Bezier);
        a.tangent = mode;
        let mut b = key(1200, 1.0, Segment::Bezier);
        b.tangent = mode;
        let c = channel(
            vec![a.clone(), b.clone()],
            Extrapolation::Hold,
            Extrapolation::Hold,
        );
        near(value(c.value_at(Tick(300))), 0.25); // approved source secants, independent of label
        a.outgoing = Some(CurveHandle {
            dt: Tick(400),
            dv: 0.0,
        });
        b.incoming = Some(CurveHandle {
            dt: Tick(-400),
            dv: 0.0,
        });
        let c = channel(vec![a, b], Extrapolation::Hold, Extrapolation::Hold);
        near(value(c.value_at(Tick(300))), 0.15625);
        near(value(c.value_at(Tick(600))), 0.5);
    }
    let mut a = key(0, 1.0, Segment::Bezier);
    let mut b = key(1200, 1.0, Segment::Linear);
    a.outgoing = Some(CurveHandle {
        dt: Tick(400),
        dv: 1.0,
    });
    b.incoming = Some(CurveHandle {
        dt: Tick(-400),
        dv: 1.0,
    });
    near(
        value(channel(vec![a, b], Extrapolation::Hold, Extrapolation::Hold).value_at(Tick(600))),
        1.75,
    );
}
#[test]
fn independent_linear_rgba_components_and_discrete_fallback_hold() {
    let mut rgba = Param::new(ParamValue::Color([0.0, 0.5, 0.25, 1.0])).unwrap();
    let red = rgba.channel_mut(Some(ColorComponent::R)).unwrap();
    red.put(key(0, 0.0, Segment::Linear)).unwrap();
    red.put(key(100, 1.0, Segment::Linear)).unwrap();
    let alpha = rgba.channel_mut(Some(ColorComponent::A)).unwrap();
    alpha.put(key(0, 1.0, Segment::Step)).unwrap();
    alpha.put(key(75, 0.0, Segment::Step)).unwrap();
    assert_eq!(
        rgba.value_at(Tick(50)).unwrap(),
        ParamValue::Color([0.5, 0.5, 0.25, 1.0])
    );
    assert_eq!(
        rgba.value_at(Tick(100)).unwrap(),
        ParamValue::Color([1.0, 0.5, 0.25, 0.0])
    );
    for (base, a, b) in [
        (
            ParamValue::Bool(false),
            ParamValue::Bool(true),
            ParamValue::Bool(false),
        ),
        (
            ParamValue::Text("base".into()),
            ParamValue::Text("đỏ".into()),
            ParamValue::Text("xanh".into()),
        ),
    ] {
        let mut k = Keyframe::new(Tick(10), a.clone());
        k.segment = Segment::Step;
        let mut j = Keyframe::new(Tick(20), b.clone());
        j.segment = Segment::Step;
        let c = Channel::new(
            base.clone(),
            vec![k, j],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap();
        assert_eq!(c.value_at(Tick(-5)), base);
        assert_eq!(c.value_at(Tick(19)), a);
        assert_eq!(c.value_at(Tick(20)), b);
    }
    assert_eq!(
        AnimationPath::scale_group(),
        ["transform.scaleX", "transform.scaleY"]
    );
}
#[test]
fn hostile_inputs_and_mutation_failures_are_bounded_and_atomic() {
    for name in [
        "",
        "transform.scale",
        "effects..params.a",
        "effects.id.params.",
        "params.a.b",
        "background.unknown",
        "../opacity",
    ] {
        assert!(AnimationPath::new(name).is_err());
    }
    assert!(AnimationPath::new(".".repeat(1 << 20)).is_err());
    for name in [
        "params.custom",
        "effects.id-1.params.amount",
        "background.color",
        "volume",
    ] {
        assert!(AnimationPath::new(name).is_ok());
    }
    for keys in [
        vec![key(10, 0.0, Segment::Linear), key(10, 1.0, Segment::Linear)],
        vec![key(10, 0.0, Segment::Linear), key(0, 1.0, Segment::Linear)],
    ] {
        assert!(Channel::new(scalar(0.0), keys, Extrapolation::Hold, Extrapolation::Hold).is_err());
    }
    for v in [f64::NAN, f64::INFINITY, 1e13] {
        assert!(Param::new(scalar(v)).is_err());
    }
    assert!(
        Channel::new(
            scalar(0.0),
            (0..=MAX_CHANNEL_KEYS)
                .map(|i| key(i as i64, 0.0, Segment::Linear))
                .collect(),
            Extrapolation::Hold,
            Extrapolation::Hold
        )
        .is_err()
    );
    let mut c = channel(
        vec![key(0, 0.0, Segment::Linear), key(1, 1.0, Segment::Linear)],
        Extrapolation::Hold,
        Extrapolation::Hold,
    );
    let old = c.clone();
    assert!(c.retime_key(Tick(0), Tick(1)).is_err());
    assert_eq!(c, old);
    assert!(
        c.set_handle(
            Tick(0),
            HandleSide::Out,
            Some(CurveHandle {
                dt: Tick(-1),
                dv: 0.0
            }),
            TangentMode::Broken
        )
        .is_err()
    );
    assert_eq!(c, old);
    let mut animation = Animations::default();
    animation.set(path(), Param::Value(c)).unwrap();
    let old = animation.clone();
    assert!(animation.retime(1, 10).is_err());
    assert_eq!(animation, old);
    assert!(animation.retime(0, 1).is_err());
    assert_eq!(animation, old);
    let mut edge = Animations::default();
    edge.set(path(), param(vec![key(i64::MAX, 0.0, Segment::Linear)]))
        .unwrap();
    let old = edge.clone();
    assert!(edge.shift(Tick(1)).is_err());
    assert_eq!(edge, old);
    assert!(
        animation
            .set(path(), Param::new(ParamValue::Bool(true)).unwrap())
            .is_err()
    );
}
#[test]
fn serde_retains_modes_extrapolation_and_independent_components_and_rejects_hostile_lists() {
    let mut animations = Animations::default();
    for (i, mode) in [
        TangentMode::Auto,
        TangentMode::Aligned,
        TangentMode::Broken,
        TangentMode::Flat,
    ]
    .into_iter()
    .enumerate()
    {
        let mut k = key(0, 0.0, Segment::Bezier);
        k.tangent = mode;
        k.outgoing = Some(CurveHandle {
            dt: Tick(10),
            dv: 0.4,
        });
        let c = Channel::new(
            scalar(0.0),
            vec![k, key(100, 1.0, Segment::Step)],
            Extrapolation::Linear,
            Extrapolation::Hold,
        )
        .unwrap();
        animations
            .set(
                AnimationPath::new(format!("params.v{i}")).unwrap(),
                Param::Value(c),
            )
            .unwrap();
    }
    animations
        .set(
            AnimationPath::new("color").unwrap(),
            Param::new(ParamValue::Color([0.1, 0.2, 0.3, 0.4])).unwrap(),
        )
        .unwrap();
    let bytes = serde_json::to_vec(&animations).unwrap();
    let decoded: Animations = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, animations);
    let c = channel(
        vec![key(0, 0.0, Segment::Linear)],
        Extrapolation::Hold,
        Extrapolation::Hold,
    );
    let mut wire = serde_json::to_value(&c).unwrap();
    wire["keys"] = serde_json::json!([key(0, 0.0, Segment::Linear), key(0, 1.0, Segment::Linear)]);
    assert!(serde_json::from_value::<Channel>(wire.clone()).is_err());
    wire["keys"] = serde_json::json!([key(0, 0.0, Segment::Linear)]);
    wire["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Channel>(wire).is_err());
    let one = serde_json::to_string(&c).unwrap();
    assert!(serde_json::from_str::<Animations>(&format!("{{\"opacity\":{{\"kind\":\"value\",\"channels\":{one}}},\"opacity\":{{\"kind\":\"value\",\"channels\":{one}}}}}")).is_err());
    let mut wire = serde_json::to_value(&c).unwrap();
    wire["keys"] = serde_json::Value::Array(
        (0..=MAX_CHANNEL_KEYS)
            .map(|i| serde_json::to_value(key(i as i64, 0.0, Segment::Linear)).unwrap())
            .collect(),
    );
    assert!(serde_json::from_value::<Channel>(wire).is_err());
}
#[test]
fn extreme_ticks_extrapolation_clamped_handles_and_constant_channels_remain_finite() {
    let c = channel(
        vec![
            key(i64::MIN, 0.0, Segment::Linear),
            key(i64::MAX, 1.0, Segment::Linear),
        ],
        Extrapolation::Linear,
        Extrapolation::Linear,
    );
    near(value(c.value_at(Tick(0))), 0.5);
    assert_eq!(c.value_at(Tick(i64::MAX)), scalar(1.0));
    let mut a = key(0, 0.0, Segment::Bezier);
    a.outgoing = Some(CurveHandle {
        dt: Tick(i64::MAX),
        dv: 0.0,
    });
    let mut b = key(10, 1.0, Segment::Linear);
    b.incoming = Some(CurveHandle {
        dt: Tick(i64::MIN),
        dv: 0.0,
    });
    let c = channel(vec![a, b], Extrapolation::Linear, Extrapolation::Linear);
    for t in [i64::MIN, -1, 0, 1, 5, 9, 10, 11, i64::MAX] {
        assert!(value(c.value_at(Tick(t))).is_finite());
    }
    let c = channel(
        vec![key(5, 3.0, Segment::Linear)],
        Extrapolation::Linear,
        Extrapolation::Linear,
    );
    assert_eq!(c.value_at(Tick(i64::MIN)), scalar(3.0));
}
proptest! {
    #![proptest_config(proptest::test_runner::Config {
        rng_seed: proptest::test_runner::RngSeed::Fixed(0x38),
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct("tests/animation.proptest-regressions"))),
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn endpoints_linear_monotonic_step_and_extrapolation(a in -10000f64..10000.0,b in -10000f64..10000.0,span in 1i64..1_000_000,offset in 0i64..1_000_000) {
        let c=channel(vec![key(0,a,Segment::Linear),key(span,b,Segment::Linear)],Extrapolation::Linear,Extrapolation::Linear);
        prop_assert_eq!(c.value_at(Tick(0)),scalar(a));prop_assert_eq!(c.value_at(Tick(span)),scalar(b));
        let t=offset%span;let v=value(c.value_at(Tick(t)));prop_assert!(v>=a.min(b)-1e-8 && v<=a.max(b)+1e-8);
        let next=value(c.value_at(Tick((t+1).min(span))));prop_assert!((next-v)*(b-a)>=-1e-6);
        let hold=channel(vec![key(0,a,Segment::Step),key(span,b,Segment::Linear)],Extrapolation::Hold,Extrapolation::Hold);
        prop_assert_eq!(hold.value_at(Tick(t)),scalar(a));prop_assert_eq!(hold.value_at(Tick(-1)),scalar(a));prop_assert_eq!(hold.value_at(Tick(span+1)),scalar(b));
        prop_assert!((value(c.value_at(Tick(-span)))-(2.0*a-b)).abs()<1e-7);
    }
    #[test]
    fn translation_and_integer_retime_preserve_bezier_values(a in -100f64..100.0,b in -100f64..100.0,t in 0i64..1000,shift in -10000i64..10000,scale in 1i64..100) {
        let mut k=key(0,a,Segment::Bezier);k.outgoing=Some(CurveHandle{dt:Tick(200),dv:1.5});
        let mut j=key(1000,b,Segment::Linear);j.incoming=Some(CurveHandle{dt:Tick(-400),dv:-2.0});
        let mut animation=Animations::default();animation.set(path(),param(vec![k,j])).unwrap();
        let expected=animation.value_at(&path(),Tick(t)).unwrap();animation.shift(Tick(shift)).unwrap();prop_assert_eq!(animation.value_at(&path(),Tick(t+shift)).unwrap(),expected.clone());
        animation.retime(scale,1).unwrap();let actual=animation.value_at(&path(),Tick((t+shift)*scale)).unwrap();
        prop_assert!((value(actual.unwrap())-value(expected.unwrap())).abs()<1e-8);
    }
    #[test]
    fn rational_tick_scaling_uses_shared_rounding(t in -1_000_000i64..1_000_000,n in 1i64..100,d in 1i64..100) {
        let v=Tick(t).checked_scale(n,d,Rounding::Nearest).unwrap();prop_assert!(((v.0 as f64)-(t as f64*n as f64/d as f64)).abs()<=0.500001);
        prop_assert!(Tick(t).checked_scale(n,d,Rounding::Floor).unwrap()<=v);prop_assert!(Tick(t).checked_scale(n,d,Rounding::Ceil).unwrap()>=v);
    }
}

#[test]
fn recorded_cubic_reference_fixtures_match_within_source_solver_precision() {
    type Fixture = (f64, f64, Option<(i64, f64)>, Option<(i64, f64)>, i64, f64);
    let fixtures: &[Fixture] = &[
        (0.0, 1.0, None, None, 1, 0.0008330345153808593),
        (0.0, 1.0, None, None, 100, 0.08333349227905273),
        (0.0, 1.0, None, None, 300, 0.2499995231628418),
        (0.0, 1.0, None, None, 600, 0.4999995231628418),
        (0.0, 1.0, None, None, 900, 0.7499995231628418),
        (0.0, 1.0, None, None, 1100, 0.9166665077209473),
        (0.0, 1.0, None, None, 1199, 0.9991669654846191),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            1,
            2.080683348668457e-06,
        ),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            100,
            0.019675998776110478,
        ),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            300,
            0.15624946355853808,
        ),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            600,
            0.4999992847442627,
        ),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            900,
            0.843749463557856,
        ),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            1100,
            0.9803240012238894,
        ),
        (
            0.0,
            1.0,
            Some((400, 0.0)),
            Some((-400, 0.0)),
            1199,
            0.9999979193166514,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            1,
            1.0024970217066311,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            100,
            1.2291670640308892,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            300,
            1.5624992847435806,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            600,
            1.7499999999993179,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            900,
            1.5625007152550552,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            1100,
            1.2291670640308894,
        ),
        (
            1.0,
            1.0,
            Some((400, 1.0)),
            Some((-400, 1.0)),
            1199,
            1.0024970217066311,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            1,
            0.00011164482443794782,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            100,
            0.016582981758436154,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            300,
            0.08711144818293937,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            600,
            0.31947330529197704,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            900,
            0.8124988198282039,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            1100,
            1.462375010519171,
        ),
        (
            0.0,
            2.0,
            Some((900, 0.1)),
            Some((-100, -0.6)),
            1199,
            1.9939970689234547,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            1,
            2.3180246543964313e-07,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            100,
            0.002556376754789314,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            300,
            0.029724690930991893,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            600,
            0.49999642372131353,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            900,
            0.970275309069008,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            1100,
            0.9974436232452107,
        ),
        (
            0.0,
            1.0,
            Some((1200, 0.0)),
            Some((-1200, 0.0)),
            1199,
            0.9999997681975346,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            1,
            0.02472386808697131,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            100,
            0.22458223488771414,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            300,
            0.3645275824482036,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            600,
            0.49999964237213135,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            900,
            0.6354724175517964,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            1100,
            0.7754177651122859,
        ),
        (
            0.0,
            1.0,
            Some((0, 0.5)),
            Some((0, -0.5)),
            1199,
            0.9752761319130286,
        ),
    ];
    let mut max_error = 0.0f64;
    for &(a, b, out, incoming, time, expected) in fixtures {
        let mut left = key(0, a, Segment::Bezier);
        left.outgoing = out.map(|(dt, dv)| CurveHandle { dt: Tick(dt), dv });
        let mut right = key(1200, b, Segment::Linear);
        right.incoming = incoming.map(|(dt, dv)| CurveHandle { dt: Tick(dt), dv });
        let actual = value(
            channel(vec![left, right], Extrapolation::Hold, Extrapolation::Hold)
                .value_at(Tick(time)),
        );
        max_error = max_error.max((actual - expected).abs());
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
    println!(
        "{} reference samples, maximum absolute value error {max_error}",
        fixtures.len()
    );
}

#[test]
fn project_aggregate_key_budget_and_item_schema_are_enforced() {
    let mut animations = Animations::default();
    let c = channel(
        (0..MAX_CHANNEL_KEYS)
            .map(|i| key(i as i64, 0.0, Segment::Linear))
            .collect(),
        Extrapolation::Hold,
        Extrapolation::Hold,
    );
    for i in 0..13 {
        animations
            .set(
                AnimationPath::new(format!("params.p{i}")).unwrap(),
                Param::Value(c.clone()),
            )
            .unwrap();
    }
    let mut data = Project::empty(ProjectId(1), "".into()).unwrap().into_data();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
        name: "".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: (0..2)
            .map(|i| Item {
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                id: ItemId(i),
                source_asset: None,
                start: Tick(i as i64 * 10),
                source_range: time::SourceRange::new(Tick(0), Tick(10)).unwrap(),
                content: ItemParams::Overlay(OverlayParams {
                    color: Color([0; 4]),
                }),
                animations: animations.clone(),
            })
            .collect(),
    });
    assert!(matches!(
        Project::new(data),
        Err(Error::TooLarge("project keys"))
    ));
    let mut a = Animations::default();
    for i in 0..MAX_ANIMATION_PATHS {
        a.set(
            AnimationPath::new(format!("params.p{i}")).unwrap(),
            Param::new(scalar(0.0)).unwrap(),
        )
        .unwrap();
    }
    let old = a.clone();
    assert!(
        a.set(
            AnimationPath::new("params.overflow").unwrap(),
            Param::new(scalar(0.0)).unwrap()
        )
        .is_err()
    );
    assert_eq!(a, old);
    assert!(Animations::default().retime(1, 0).is_err());
}

#[test]
fn adjacent_large_tick_anchors_and_hostile_composite_wires_keep_invariants() {
    for start in [i64::MIN, i64::MAX - 1200] {
        let c = channel(
            vec![
                key(start, 0.0, Segment::Bezier),
                key(start + 1200, 1.0, Segment::Linear),
            ],
            Extrapolation::Hold,
            Extrapolation::Hold,
        );
        near(value(c.value_at(Tick(start + 300))), 0.25);
    }
    let color = Param::new(ParamValue::Color([0.0, 0.0, 0.0, 1.0])).unwrap();
    let mut wire = serde_json::to_value(&color).unwrap();
    wire["channels"][0]["value"] = serde_json::json!({"kind":"bool","value":false});
    assert!(serde_json::from_value::<Param>(wire).is_err());
    let mut wire = serde_json::to_value(&color).unwrap();
    wire["channels"][0]["value"] = serde_json::json!({"kind":"scalar","value":2.0});
    assert!(serde_json::from_value::<Param>(wire).is_err());
    let c = channel(
        vec![key(0, 0.0, Segment::Linear)],
        Extrapolation::Hold,
        Extrapolation::Hold,
    );
    let mut wire = serde_json::to_value(c).unwrap();
    wire["keys"][0]["outgoing"] = serde_json::json!({"dt":1,"dv":null});
    assert!(serde_json::from_value::<Channel>(wire).is_err());
}

#[test]
fn crossed_handles_resolve_single_ticks_near_stationary_time_midpoint() {
    let span = 2_540_160_000_000i64;
    let mut a = key(0, 0.0, Segment::Bezier);
    a.outgoing = Some(CurveHandle {
        dt: Tick(span),
        dv: 0.0,
    });
    let mut b = key(span, 1.0, Segment::Linear);
    b.incoming = Some(CurveHandle {
        dt: Tick(-span),
        dv: 0.0,
    });
    let c = channel(vec![a, b], Extrapolation::Hold, Extrapolation::Hold);
    assert_eq!(c.value_at(Tick(span / 2)), scalar(0.5));
    for offset in [-1, 1] {
        let t = Tick(span / 2 + offset);
        // For crossed time controls (1, 0), x = 1/2 + 4*(s - 1/2)^3.
        let s = 0.5 + (offset as f64 / span as f64 / 4.0).cbrt();
        let expected = 3.0 * s * s - 2.0 * s * s * s;
        let actual = value(c.value_at(t));
        assert!((actual - expected).abs() < 2e-8, "{actual} != {expected}");
    }
}
