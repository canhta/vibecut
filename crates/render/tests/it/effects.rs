#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::{Hooks, tick};
use project::{ANGLE_MAX, Color, EffectValue};
use render::{effects::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[test]
fn gaussian_scaling_bounds_and_channel_energy() {
    assert_eq!(blur_sigma(15.0, 1920, 1080).unwrap(), 3.0);
    assert_eq!(blur_sigma(15.0, 3840, 2160).unwrap(), 6.0);
    for intensity in [0.0, f64::MIN_POSITIVE, 15.0, 100.0] {
        for (w, h) in [(1, 1), (1920, 1080), (3840, 2160), (8192, 8192)] {
            let passes = blur_passes(intensity, w, h).unwrap();
            assert!(passes.len() <= 512);
            for pass in passes {
                validate_pass(&pass, w, h).unwrap();
            }
        }
    }
    for bad in [f64::NAN, f64::INFINITY, -1.0, 100.1] {
        assert!(blur_passes(bad, 20, 20).is_err());
    }
    assert!(blur_passes(15.0, 0, 20).is_err());
    let hooks = Hooks::default();
    let image = Image::from_working_pixels(
        33,
        33,
        (0..1089).map(|i| {
            if i == 544 {
                [0.5, 0.25, 0.0, 0.5]
            } else {
                [0.0; 4]
            }
        }),
        &hooks,
    )
    .unwrap();
    let passes = blur_passes(100.0, 256, 144).unwrap();
    let output = CpuEffects
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    let energy: f32 = output.pixels().iter().map(|p| p[0]).sum();
    assert!((energy - 0.5).abs() < 1e-5);
    assert!(output.pixels()[544][0] < 0.5);
    for p in output.pixels() {
        assert_eq!(p[0], p[3]);
        assert_eq!(p[0], p[1] * 2.0);
        assert_eq!(p[2], 0.0);
    }
    let zero = CpuEffects
        .apply(
            &image,
            &[],
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    assert_eq!(zero.pixels(), image.pixels());
}

#[test]
fn effects_refuse_budget_cancel_and_invalid_pass() {
    let hooks = Hooks::default();
    let image = Image::new(16, 16, [0.2; 4], &hooks).unwrap();
    let passes = blur_passes(100.0, 1920, 1080).unwrap();
    let tiny = Hooks {
        limit: 0,
        ..Default::default()
    };
    assert!(matches!(
        CpuEffects.apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image
            },
            &tiny
        ),
        Err(Error::Budget { .. })
    ));
    let cancelled = Hooks {
        state: Arc::new(AtomicU8::new(1)),
        ..Default::default()
    };
    assert!(matches!(
        CpuEffects.apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image
            },
            &cancelled
        ),
        Err(Error::Cancelled)
    ));
    cancelled.state.store(0, Ordering::SeqCst);
    let bad = EffectPass::Convolve {
        entry_point: "unknown",
        horizontal: true,
        weights: vec![1.0],
    };
    // The shared scene/thumbnail entry admits geometry/passes before reserving
    // its retained input copy, even when the working budget is zero.
    tiny.requested.store(0, Ordering::SeqCst);
    let owned = || Image::new(16, 16, [0.2; 4], &hooks).unwrap();
    let below = Image::new(1, 1, [0.0; 4], &hooks).unwrap();
    assert!(matches!(
        apply_effect(owned(), &[], &below, &CpuEffects, &tiny),
        Err(Error::Invalid("effect input dimensions"))
    ));
    assert!(matches!(
        apply_effect(
            owned(),
            std::slice::from_ref(&bad),
            &image,
            &CpuEffects,
            &tiny
        ),
        Err(Error::Unsupported("effect pass"))
    ));
    assert_eq!(tiny.requested.load(Ordering::SeqCst), 0);
    let empty = apply_effect(owned(), &[], &image, &CpuEffects, &hooks).unwrap();
    assert_eq!(empty.pixels(), image.pixels());
    assert!(
        CpuEffects
            .apply(
                &image,
                &[bad],
                EffectInputs {
                    input: &image,
                    coverage: None,
                    below: &image
                },
                &hooks
            )
            .is_err()
    );
}

#[test]
fn item_blur_is_isolated_from_track_below_and_uses_animation() {
    use project::*;
    use time::{FrameRate, SourceRange, Tick};
    let hooks = Hooks::default();
    let mut data = Project::empty(ProjectId(1), "blur".into())
        .unwrap()
        .into_data();
    data.canvas.width = 256;
    data.canvas.height = 144;
    let item = Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: vec![],
        effects: vec![
            effect_definition("blur")
                .unwrap()
                .instance("b")
                .with("intensity", EffectValue::Scalar(100.0))
                .unwrap(),
        ],
        animations: Default::default(),
        id: ItemId(1),
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, FrameRate::FPS_30.tick_of(30).unwrap()).unwrap(),
        start: Tick::ZERO,
        content: ItemParams::Overlay(OverlayParams {
            color: Color([255, 0, 0, 255]),
        }),
    };
    data.canvas.background = Color([0, 0, 255, 255]);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![item],
    });
    let project = Project::new(data.clone()).unwrap();
    let mut settings = RenderSettings::from_project(&project);
    settings.placements.insert(
        ItemId(1),
        Placement {
            destination: Rect {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            ..Default::default()
        },
    );
    let blurred = Renderer::new(&project, settings.clone())
        .unwrap()
        .render_frame(Tick::ZERO, &mut FileSources::default(), &hooks)
        .unwrap();
    // Blur spreads the red item across its geometric boundary, preserving the blue below.
    let p = blurred.pixels()[72 * 256 + 63];
    assert!(p[0] > 0.1 && p[0] < 0.5);
    assert!(p[2] > 0.5);
    assert_eq!(p[3], 1.0);
    data.scene_mut().timeline.overlays[0].items[0]
        .animations
        .set(
            AnimationPath::new("effects.b.params.intensity").unwrap(),
            Param::new(ParamValue::Scalar(0.0)).unwrap(),
        )
        .unwrap();
    let sharp = Project::new(data).unwrap();
    let output = Renderer::new(&sharp, settings)
        .unwrap()
        .render_frame(Tick::ZERO, &mut FileSources::default(), &hooks)
        .unwrap();
    assert_eq!(output.pixels()[72 * 256 + 63], [0.0, 0.0, 1.0, 1.0]);
}

/// Colour chart in the premultiplied linear working space: grey ramp, the
/// primaries/secondaries, a skin-like tone and a half-transparent patch.
fn chart() -> Vec<[f32; 4]> {
    let mut px: Vec<[f32; 4]> = [0.0, 0.05, 0.18, 0.5, 1.0]
        .iter()
        .map(|g| [*g, *g, *g, 1.0])
        .collect();
    px.extend([
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0, 1.0],
        [0.6, 0.35, 0.25, 1.0],
        [0.3, 0.15, 0.05, 0.5],
    ]);
    px
}
fn graded(grade: [f64; 4]) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let hooks = Hooks::default();
    let source = chart();
    let image =
        Image::from_working_pixels(source.len() as u32, 1, source.iter().copied(), &hooks).unwrap();
    let passes = grade_passes(grade[0], grade[1], grade[2], grade[3]).unwrap();
    let out = CpuEffects
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    (source, out.pixels().to_vec())
}
fn luma(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}
/// Run a full colour-grade instance (every stage plus its LUT) through the
/// public evaluation and CPU execution path; returns source, output and the
/// ordered pass entries.
fn graded_on(
    params: &[(&'static str, EffectValue)],
    source: &[[f32; 4]],
) -> (Vec<[f32; 4]>, Vec<&'static str>) {
    let hooks = Hooks::default();
    let image =
        Image::from_working_pixels(source.len() as u32, 1, source.iter().copied(), &hooks).unwrap();
    let mut effect = project::effect_definition("color_grade")
        .unwrap()
        .instance("grade");
    for (id, value) in params {
        effect = effect.with(id, *value).unwrap();
    }
    let passes = evaluate_effect(
        &project::Animations::default(),
        &effect,
        time::Tick::ZERO,
        EffectGeometry::new([source.len() as u32, 1], [1.0, 1.0]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    let entries = passes.iter().map(EffectPass::entry_point).collect();
    let out = CpuEffects
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    (out.pixels().to_vec(), entries)
}
fn graded_with(
    params: &[(&'static str, EffectValue)],
) -> (Vec<[f32; 4]>, Vec<[f32; 4]>, Vec<&'static str>) {
    let source = chart();
    let (out, entries) = graded_on(params, &source);
    (source, out, entries)
}
fn curve(points: &[[f64; 2]]) -> EffectValue {
    EffectValue::Curve(project::ToneCurve::new(points).unwrap())
}

#[test]
fn color_grade_controls_on_the_chart() {
    // Neutral (and any value that rounds to neutral) is identity with no pass.
    assert!(grade_passes(0.0, 0.0, 0.0, 0.0).unwrap().is_empty());
    let (source, out) = graded([0.0; 4]);
    assert_eq!(source, out);
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    // brightness: +-1 stop of linear gain, clamped to the gamut.
    let (_, up) = graded([50.0, 0.0, 0.0, 0.0]);
    let (_, down) = graded([-50.0, 0.0, 0.0, 0.0]);
    assert!(near(up[2][0], 0.36) && near(up[3][0], 1.0) && near(up[1][0], 0.1));
    assert!(near(down[2][0], 0.09) && near(down[3][0], 0.25));
    // contrast: power about linear mid grey 0.18, which stays put.
    let (_, high) = graded([0.0, 50.0, 0.0, 0.0]);
    let (_, low) = graded([0.0, -50.0, 0.0, 0.0]);
    assert!(near(high[2][0], 0.18) && near(low[2][0], 0.18));
    assert!(near(high[3][0], 0.18 * (0.5f32 / 0.18).powf(1.5)));
    assert!(near(low[3][0], 0.18 * (0.5f32 / 0.18).powf(0.5)));
    assert!(high[1][0] < 0.05 && low[1][0] > 0.05);
    // saturation: -50 removes chroma at constant luma, +50 doubles it.
    let (_, grey) = graded([0.0, 0.0, -50.0, 0.0]);
    for (i, p) in grey.iter().copied().enumerate().take(12).skip(5) {
        assert!(near(p[0], p[1]) && near(p[1], p[2]), "{i}");
        assert!(near(p[0], luma(chart()[i])), "{i}");
    }
    let (_, vivid) = graded([0.0, 0.0, 50.0, 0.0]);
    assert!(near(
        vivid[11][0],
        (0.6 * 2.0 - luma(chart()[11])).clamp(0.0, 1.0)
    ));
    for (v, c) in vivid.iter().zip(chart()).take(5) {
        assert!(near(v[0], c[0]), "grey stays grey");
    }
    // hue: +50 is 180 degrees about the grey axis; red maps to a cyan-ish tone.
    let (_, half) = graded([0.0, 0.0, 0.0, 50.0]);
    assert!(near(half[5][0], 0.0) && near(half[5][1], 2.0 / 3.0) && near(half[5][2], 2.0 / 3.0));
    let (_, back) = graded([0.0, 0.0, 0.0, -50.0]);
    assert!(
        near(back[5][1], half[5][1]) && near(back[5][2], half[5][2]),
        "+-180 coincide"
    );
    for (h, c) in half.iter().zip(chart()).take(5) {
        assert!(near(h[0], c[0]), "grey is invariant under hue");
    }
    // combined grade, and every output stays a valid premultiplied pixel.
    let (_, mixed) = graded([20.0, -30.0, 40.0, -25.0]);
    let (_, again) = graded([20.0, -30.0, 40.0, -25.0]);
    assert_eq!(mixed, again);
    assert_ne!(mixed, chart());
    for (grade, out) in [
        ([50.0, 50.0, 50.0, 50.0], graded([50.0, 50.0, 50.0, 50.0]).1),
        ([-50.0; 4], graded([-50.0; 4]).1),
        ([20.0, -30.0, 40.0, -25.0], mixed),
    ] {
        for (p, s) in out.iter().zip(chart()) {
            assert_eq!(p[3], s[3], "{grade:?} keeps alpha");
            assert!(
                p[..3]
                    .iter()
                    .all(|c| c.is_finite() && *c >= 0.0 && *c <= p[3] + 1e-6)
            );
        }
    }
    // the half-transparent patch grades on its straight colour.
    let (_, half_alpha) = graded([50.0, 0.0, 0.0, 0.0]);
    assert!(near(half_alpha[12][0], (0.3 / 0.5 * 2.0f32).min(1.0) * 0.5));

    // The whole family at neutral is exact identity with no pass.
    let (source, neutral, entries) = graded_with(&[]);
    assert_eq!(source, neutral);
    assert!(entries.is_empty());
    // Temperature is a red/blue linear gain pair around neutral.
    let (_, warm, entries) = graded_with(&[("temperature", EffectValue::Scalar(50.0))]);
    assert_eq!(entries, ["tone"]);
    assert!(near(warm[3][0], 0.5 * 2.0f32.powf(0.25)));
    assert!(near(warm[3][2], 0.5 * 2.0f32.powf(-0.25)));
    // Highlight and shine act on bright linear values; shadow acts on dark ones.
    let (_, lifted_high, _) = graded_with(&[("highlight", EffectValue::Scalar(50.0))]);
    assert!(near(lifted_high[3][0], 0.5625));
    let (_, lifted_low, _) = graded_with(&[("shadow", EffectValue::Scalar(50.0))]);
    assert!(near(lifted_low[0][0], 0.25));
    let (_, shiny, _) = graded_with(&[("shine", EffectValue::Scalar(50.0))]);
    assert!(near(shiny[3][0], 0.53125));
    // Fade mixes linear light toward the mid-grey pivot.
    let (_, faded, _) = graded_with(&[("fade", EffectValue::Scalar(50.0))]);
    assert!(near(faded[3][0], 0.34));
    // The effect entries append in the documented order after the primary grade.
    let (_, _, ordered) = graded_with(&[
        ("brightness", EffectValue::Scalar(10.0)),
        ("temperature", EffectValue::Scalar(10.0)),
        ("lift_r", EffectValue::Scalar(10.0)),
        ("curve_red", curve(&[[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]])),
        ("hsl_red_hue", EffectValue::Scalar(10.0)),
        ("vignette", EffectValue::Scalar(10.0)),
        ("grain", EffectValue::Scalar(10.0)),
    ]);
    assert_eq!(
        ordered,
        [
            "grade",
            "tone",
            "lift",
            "curve_red",
            "hsl_bands",
            "vignette",
            "grain"
        ]
    );
    // Sharpness is an unsharp mask; a negative value softens with a gaussian.
    let smooth = [
        [0.0, 0.0, 0.0, 1.0],
        [0.1, 0.1, 0.1, 1.0],
        [0.2, 0.2, 0.2, 1.0],
        [0.4, 0.4, 0.4, 1.0],
        [0.8, 0.8, 0.8, 1.0],
    ];
    let (sharp, entries) = graded_on(&[("sharpness", EffectValue::Scalar(50.0))], &smooth);
    assert_eq!(entries, ["gaussian", "gaussian", "detail_boost"]);
    assert_ne!(sharp, smooth);
    let (soft, entries) = graded_on(&[("sharpness", EffectValue::Scalar(-50.0))], &smooth);
    assert_eq!(entries, ["gaussian", "gaussian"]);
    assert_ne!(soft, smooth);
    // Vignette leaves the frame centre alone and darkens the corner pixel.
    let (_, vignetted, entries) = graded_with(&[("vignette", EffectValue::Scalar(50.0))]);
    assert_eq!(entries, ["vignette"]);
    assert!(near(vignetted[6][0], source[6][0]));
    assert!(vignetted[12][0] < source[12][0]);
    // Grain is deterministic coordinate noise and changes the picture.
    let (_, grained, entries) = graded_with(&[("grain", EffectValue::Scalar(50.0))]);
    assert_eq!(entries, ["grain"]);
    assert!(grained.iter().zip(&source).any(|(a, b)| a != b));
    let (_, again, _) = graded_with(&[("grain", EffectValue::Scalar(50.0))]);
    assert_eq!(grained, again);
    // HSL bands address one hue: the red patch takes the red band's change and
    // the blue band leaves it untouched.
    let (_, shifted, entries) = graded_with(&[("hsl_red_hue", EffectValue::Scalar(50.0))]);
    assert_eq!(entries, ["hsl_bands"]);
    assert!(near(shifted[5][1], 0.214_04));
    let (_, desaturated, _) = graded_with(&[("hsl_red_saturation", EffectValue::Scalar(-50.0))]);
    assert!(desaturated[5][0] < source[5][0] && desaturated[5][1] > source[5][1]);
    let (_, isolated, _) = graded_with(&[("hsl_blue_hue", EffectValue::Scalar(50.0))]);
    assert_eq!(isolated[5], source[5]);
    // Lift is x + L*(1-x) on straight colour, then clamped and premultiplied.
    let (_, lifted, entries) = graded_with(&[
        ("lift_r", EffectValue::Scalar(50.0)),
        ("lift_g", EffectValue::Scalar(50.0)),
        ("lift_b", EffectValue::Scalar(50.0)),
    ]);
    assert_eq!(entries, ["lift"]);
    for (index, expected) in [(0, 0.5), (2, 0.59), (3, 0.75), (4, 1.0)] {
        assert!(near(lifted[index][0], expected), "lift {index}");
    }
    let (_, lowered, _) = graded_with(&[
        ("lift_r", EffectValue::Scalar(-50.0)),
        ("lift_g", EffectValue::Scalar(-50.0)),
        ("lift_b", EffectValue::Scalar(-50.0)),
    ]);
    assert!(near(lowered[2][0], 0.0) && near(lowered[3][0], 0.25));
    let (_, black, _) = graded_with(&[
        ("lift_r", EffectValue::Scalar(-100.0)),
        ("lift_g", EffectValue::Scalar(-100.0)),
        ("lift_b", EffectValue::Scalar(-100.0)),
    ]);
    let (_, white, _) = graded_with(&[
        ("lift_r", EffectValue::Scalar(100.0)),
        ("lift_g", EffectValue::Scalar(100.0)),
        ("lift_b", EffectValue::Scalar(100.0)),
    ]);
    // Lift -100 maps straight x to x - (1 - x) = 2x - 1, so everything at or
    // below middle grey goes black and brighter values keep their excess.
    for (pixel, source) in black.iter().zip(&source) {
        assert_eq!(pixel[3], source[3]);
        for k in 0..3 {
            let straight = if source[3] > 0.0 {
                source[k] / source[3]
            } else {
                0.0
            };
            let expected = (2.0 * straight - 1.0).clamp(0.0, 1.0) * source[3];
            assert!(
                near(pixel[k], expected),
                "lift -100 {pixel:?} from {source:?}"
            );
        }
    }
    for (pixel, source) in white.iter().zip(&source) {
        assert_eq!(pixel[3], source[3]);
        assert!(pixel[..3].iter().all(|v| (*v - pixel[3]).abs() < 1e-6));
    }
    // Soft alpha keeps the straight arithmetic: [.1,.4,.8] with [.2,-.2,.5].
    let (mixed, _) = graded_on(
        &[
            ("lift_r", EffectValue::Scalar(20.0)),
            ("lift_g", EffectValue::Scalar(-20.0)),
            ("lift_b", EffectValue::Scalar(50.0)),
        ],
        &[[0.025, 0.1, 0.2, 0.25]],
    );
    assert_eq!(mixed[0][3], 0.25);
    for (actual, expected) in mixed[0].iter().zip([0.07f32, 0.07, 0.225, 0.25]) {
        assert!((actual - expected).abs() <= 1e-6, "{mixed:?}");
    }
    // Gamma is x^(1/g) on straight colour, gain is g*x.
    let (_, gamma_low, entries) = graded_with(&[
        ("gamma_r", EffectValue::Scalar(25.0)),
        ("gamma_g", EffectValue::Scalar(25.0)),
        ("gamma_b", EffectValue::Scalar(25.0)),
    ]);
    assert_eq!(entries, ["tone_gamma"]);
    assert!(near(gamma_low[3][0], 0.0625));
    let (mid, _) = graded_on(
        &[("gamma_r", EffectValue::Scalar(400.0))],
        &[[0.0625, 0.0625, 0.0625, 1.0]],
    );
    assert!(near(mid[0][0], 0.5) && near(mid[0][1], 0.0625));
    let (_, gained, entries) = graded_with(&[("gain_r", EffectValue::Scalar(400.0))]);
    assert_eq!(entries, ["gain"]);
    assert!(near(gained[3][0], 1.0));
    let (zeroed, _) = graded_on(
        &[("gain_g", EffectValue::Scalar(0.0))],
        &[[0.4, 0.4, 0.4, 1.0]],
    );
    assert!(near(zeroed[0][1], 0.0) && near(zeroed[0][0], 0.4));
    // Basic grade stays before lift: -.5 stop then lift.5 on .2 is .55, not .3.
    let (order, entries) = graded_on(
        &[
            ("brightness", EffectValue::Scalar(-50.0)),
            ("lift_r", EffectValue::Scalar(50.0)),
            ("lift_g", EffectValue::Scalar(50.0)),
            ("lift_b", EffectValue::Scalar(50.0)),
        ],
        &[[0.2, 0.2, 0.2, 1.0]],
    );
    assert_eq!(entries, ["grade", "lift"]);
    assert!(near(order[0][0], 0.55), "{order:?}");
    // Curves map their channel independently; the luma curve keeps chroma.
    let ramp = curve(&[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]);
    let (_, red_curve, entries) = graded_with(&[("curve_red", ramp)]);
    assert_eq!(entries, ["curve_red"]);
    assert!(near(red_curve[3][0], 0.25) && near(red_curve[3][1], 0.5));
    assert!(near(red_curve[5][0], 1.0) && near(red_curve[5][1], 0.0));
    let (_, luma_curve, _) = graded_with(&[("curve_luma", ramp)]);
    assert!(
        near(luma_curve[3][0], 0.25)
            && near(luma_curve[3][1], 0.25)
            && near(luma_curve[3][2], 0.25)
    );
    assert!(near(luma_curve[0][0], 0.0), "black stays black");
    assert!(
        near(luma_curve[5][0], 0.5) && near(luma_curve[5][1], 0.0),
        "chroma scales before clipping"
    );
    let (_, bright_luma, _) =
        graded_with(&[("curve_luma", curve(&[[0.0, 0.0], [0.25, 1.0], [1.0, 1.0]]))]);
    assert!(near(bright_luma[5][0], 1.0) && near(bright_luma[5][1], 0.0));
    // Identity curves add no pass, and a combined extreme grade stays valid.
    let identity = curve(&[[0.0, 0.0], [1.0, 1.0]]);
    let (_, unchanged, entries) = graded_with(&[("curve_red", identity)]);
    assert!(entries.is_empty());
    assert_eq!(unchanged, source);
    for params in [
        [
            ("lift_r", EffectValue::Scalar(100.0)),
            ("gamma_b", EffectValue::Scalar(400.0)),
            ("gain_g", EffectValue::Scalar(400.0)),
            ("curve_luma", curve(&[[0.0, 0.0], [0.25, 1.0], [1.0, 1.0]])),
        ],
        [
            ("lift_g", EffectValue::Scalar(-100.0)),
            ("gamma_r", EffectValue::Scalar(25.0)),
            ("gain_b", EffectValue::Scalar(0.0)),
            ("curve_red", curve(&[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]])),
        ],
    ] {
        let (_, out, _) = graded_with(&params);
        for (pixel, source) in out.iter().zip(&source) {
            assert_eq!(pixel[3], source[3]);
            assert!(
                pixel[..3]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0 && *v <= pixel[3] + 1e-6)
            );
        }
    }

    // Colour-chart goldens for the whole family: neutral, every stage at its
    // minimum, every stage at its maximum and a combined grade with RGB/luma
    // curves. The recorded chart is an independent f64 application of the
    // documented stage equations (basic grade, lift, tone gamma, gain, curves),
    // not a call into the production pixel ops. `2e-5` is the recorded working
    // tolerance used for the native parity rows.
    let golden_chart = [
        [0.0, 0.0, 0.0, 1.0],
        [0.18, 0.18, 0.18, 1.0],
        [0.5, 0.5, 0.5, 1.0],
        [1.0, 1.0, 1.0, 1.0],
        [0.25, 0.5, 0.75, 1.0],
        [0.1, 0.2, 0.3, 0.5],
    ];
    type Golden = (
        &'static str,
        Vec<(&'static str, EffectValue)>,
        [[f64; 4]; 6],
    );
    let goldens: [Golden; 4] = [
        (
            "neutral",
            Vec::new(),
            [
                [0.0, 0.0, 0.0, 1.0],
                [0.18, 0.18, 0.18, 1.0],
                [0.5, 0.5, 0.5, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                [0.25, 0.5, 0.75, 1.0],
                [0.1, 0.2, 0.3, 0.5],
            ],
        ),
        (
            "minimum",
            vec![
                ("lift_r", EffectValue::Scalar(-100.0)),
                ("lift_g", EffectValue::Scalar(-100.0)),
                ("lift_b", EffectValue::Scalar(-100.0)),
                ("gamma_r", EffectValue::Scalar(25.0)),
                ("gamma_g", EffectValue::Scalar(25.0)),
                ("gamma_b", EffectValue::Scalar(25.0)),
                ("gain_r", EffectValue::Scalar(0.0)),
                ("gain_g", EffectValue::Scalar(0.0)),
                ("gain_b", EffectValue::Scalar(0.0)),
            ],
            [
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 0.5],
            ],
        ),
        (
            "maximum",
            vec![
                ("lift_r", EffectValue::Scalar(100.0)),
                ("lift_g", EffectValue::Scalar(100.0)),
                ("lift_b", EffectValue::Scalar(100.0)),
                ("gamma_r", EffectValue::Scalar(400.0)),
                ("gamma_g", EffectValue::Scalar(400.0)),
                ("gamma_b", EffectValue::Scalar(400.0)),
                ("gain_r", EffectValue::Scalar(400.0)),
                ("gain_g", EffectValue::Scalar(400.0)),
                ("gain_b", EffectValue::Scalar(400.0)),
            ],
            [
                [1.0, 1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                [0.5, 0.5, 0.5, 0.5],
            ],
        ),
        (
            "combined",
            vec![
                ("brightness", EffectValue::Scalar(20.0)),
                ("contrast", EffectValue::Scalar(-30.0)),
                ("saturation", EffectValue::Scalar(40.0)),
                ("hue", EffectValue::Scalar(-25.0)),
                ("lift_r", EffectValue::Scalar(20.0)),
                ("lift_g", EffectValue::Scalar(-20.0)),
                ("lift_b", EffectValue::Scalar(50.0)),
                ("gamma_r", EffectValue::Scalar(100.0)),
                ("gamma_g", EffectValue::Scalar(200.0)),
                ("gamma_b", EffectValue::Scalar(50.0)),
                ("gain_r", EffectValue::Scalar(150.0)),
                ("gain_g", EffectValue::Scalar(100.0)),
                ("gain_b", EffectValue::Scalar(75.0)),
                ("curve_red", curve(&[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]])),
                (
                    "curve_blue",
                    curve(&[[0.0, 0.0], [0.25, 0.125], [0.75, 0.875], [1.0, 1.0]]),
                ),
                ("curve_luma", curve(&[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]])),
            ],
            [
                [0.225, 0.0, 0.140_625, 1.0],
                [0.515_098_7, 0.374_297_6, 0.251_432_3, 1.0],
                [1.0, 0.773_496_3, 0.451_894_8, 1.0],
                [1.0, 0.896_671_9, 0.643_352_1, 1.0],
                [0.578_819_1, 1.0, 0.243_821_3, 1.0],
                [0.259_957_1, 0.490_356, 0.112_628_9, 0.5],
            ],
        ),
    ];
    for (name, params, expected) in goldens {
        let (out, entries) = graded_on(&params, &golden_chart);
        if name == "neutral" {
            assert!(entries.is_empty(), "neutral emits no pass");
        } else {
            assert!(!entries.is_empty(), "{name} emits its stages");
        }
        for (index, (pixel, want)) in out.iter().zip(expected).enumerate() {
            for channel in 0..4 {
                let actual = f64::from(pixel[channel]);
                assert!(
                    (actual - want[channel]).abs() <= 2e-5,
                    "{name} chart pixel {index} channel {channel}: {actual} != {}",
                    want[channel]
                );
            }
        }
    }
}

#[test]
fn color_grade_refuses_hostile_values_and_bad_passes() {
    for bad in [f64::NAN, f64::INFINITY, -50.1, 50.1] {
        assert!(grade_passes(bad, 0.0, 0.0, 0.0).is_err());
        assert!(grade_passes(0.0, 0.0, 0.0, bad).is_err());
    }
    for weights in [
        vec![1.0; 3],
        vec![1.0, 0.0, 1.0, 0.0],
        vec![1.0, 1.0, f32::NAN, 0.0],
    ] {
        assert!(
            validate_pass(
                &EffectPass::Pixel {
                    entry_point: "grade",
                    uniforms: weights.into_iter().map(Uniform::Float).collect(),
                },
                8,
                4
            )
            .is_err()
        );
    }
    for bad in [f64::NAN, f64::INFINITY, -100.1, 100.1] {
        assert!(lift_passes(bad, 0.0, 0.0).is_err());
    }
    for bad in [f64::NAN, 24.9, 400.1] {
        assert!(gamma_passes(bad, 100.0, 100.0).is_err());
    }
    for bad in [f64::NAN, -0.1, 400.1] {
        assert!(gain_passes(100.0, bad, 100.0).is_err());
    }
    for (entry, weights) in [
        ("lift", vec![0.0, 0.0]),
        ("lift", vec![0.0, 1.5, 0.0]),
        ("lift", vec![f32::NAN, 0.0, 0.0]),
        ("tone_gamma", vec![1.0, 0.24, 1.0]),
        ("gain", vec![1.0, 1.0, 4.1]),
    ] {
        assert!(
            validate_pass(
                &EffectPass::Pixel {
                    entry_point: entry,
                    uniforms: weights.into_iter().map(Uniform::Float).collect(),
                },
                8,
                4
            )
            .is_err(),
            "{entry}"
        );
    }
    // Curve passes flatten to 65 values; padding and endpoints are checked.
    let mut curve = vec![0.0f32; 1 + 2 * project::CURVE_MAX_POINTS];
    curve[0] = 2.0;
    curve[3] = 1.0;
    curve[4] = 1.0;
    assert!(
        validate_pass(
            &EffectPass::Pixel {
                entry_point: "curve_red",
                uniforms: curve.iter().copied().map(Uniform::Float).collect(),
            },
            8,
            4
        )
        .is_ok()
    );
    for (index, value) in [
        (0, 1.0),
        (0, 33.0),
        (0, 2.5),
        (1, 1.0 / 8192.0),
        (3, 0.9),
        (4, -0.1),
        (63, 0.5),
    ] {
        let mut hostile = curve.clone();
        if let Some(slot) = hostile.get_mut(index) {
            *slot = value;
        }
        assert!(
            validate_pass(
                &EffectPass::Pixel {
                    entry_point: "curve_red",
                    uniforms: hostile.into_iter().map(Uniform::Float).collect(),
                },
                8,
                4
            )
            .is_err(),
            "{index} {value}"
        );
    }
    assert!(
        validate_pass(
            &EffectPass::Pixel {
                entry_point: "curve_red",
                uniforms: curve.iter().copied().take(64).map(Uniform::Float).collect(),
            },
            8,
            4
        )
        .is_err()
    );
}

#[test]
fn colour_grade_stages_precede_the_lut_and_a_later_chroma_key() {
    use project::{ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind};
    use std::fs;
    let hooks = Hooks::default();
    let dir = tempfile::tempdir().unwrap();
    // Encoded (r,g,b) -> (g,b,r): a linear table, so trilinear sampling is exact.
    let cube = b"LUT_3D_SIZE 2\n0 0 0\n0 0 1\n1 0 0\n1 0 1\n0 1 0\n0 1 1\n1 1 0\n1 1 1\n";
    let path = dir.path().join("canonical.cube");
    fs::write(&path, cube).unwrap();
    let id = project::LutId::new(media::ContentHash::from_bytes(cube));
    let record = project::LutRecord {
        id,
        name: "Swap".into(),
        licence: None,
        source_name: "swap.cube".into(),
        source_hash: media::ContentHash::from_bytes(cube),
        directory: dir.path().to_path_buf(),
    };
    let mut grade = project::effect_definition("color_grade")
        .unwrap()
        .instance("grade")
        .with("brightness", EffectValue::Scalar(-50.0))
        .unwrap()
        .with("saturation", EffectValue::Scalar(50.0))
        .unwrap()
        .with("lift_r", EffectValue::Scalar(50.0))
        .unwrap()
        .with("lift_g", EffectValue::Scalar(50.0))
        .unwrap()
        .with("lift_b", EffectValue::Scalar(50.0))
        .unwrap()
        .with("lut", EffectValue::Lut(Some(id)))
        .unwrap();
    grade
        .params
        .insert("lut_strength".into(), EffectValue::Scalar(100.0));
    let key = project::effect_definition("chroma_key")
        .unwrap()
        .instance("key")
        .with("key_color", EffectValue::Color(Color([0, 255, 0, 255])))
        .unwrap();
    let mut data = Project::empty(ProjectId(1), "order".into())
        .unwrap()
        .into_data();
    data.canvas.width = 4;
    data.canvas.height = 1;
    data.luts.push(record);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: vec![],
            effects: vec![grade.clone(), key.clone()],
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: None,
            source_range: time::SourceRange::new(
                time::Tick::ZERO,
                time::FrameRate::FPS_30.tick_of(30).unwrap(),
            )
            .unwrap(),
            start: time::Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([0; 4]),
            }),
        }],
    });
    let project = Project::new(data).unwrap();
    // The overlay lives on the inserted track 1, not on the main track.
    let item = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == project::ItemId(1))
        .unwrap();
    let resources =
        render::lut::LutResources::resolve(&project, item.effects.iter(), &hooks).unwrap();
    let geometry = EffectGeometry::new([4, 1], [1.0, 1.0]).unwrap();
    let grade_passes = evaluate_effect(
        &item.animations,
        &item.effects[0],
        time::Tick::ZERO,
        geometry,
        true,
        &resources,
    )
    .unwrap();
    let entries: Vec<_> = grade_passes.iter().map(EffectPass::entry_point).collect();
    assert_eq!(entries, ["grade", "lift", "lut_3d"]);
    let key_passes = evaluate_effect(
        &item.animations,
        &item.effects[1],
        time::Tick::ZERO,
        geometry,
        true,
        &resources,
    )
    .unwrap();
    assert_eq!(key_passes.len(), 1);
    assert_eq!(key_passes[0].entry_point(), "chroma_key");
    // The item's effect vector is applied in stack order: the whole grade
    // instance (including its LUT) precedes the later key instance.
    let stack: Vec<_> = grade_passes
        .iter()
        .chain(&key_passes)
        .map(EffectPass::entry_point)
        .collect();
    assert_eq!(stack, ["grade", "lift", "lut_3d", "chroma_key"]);
    let pixel = || Image::from_working_pixels(1, 1, [[0.1, 0.3, 0.6, 1.0]].into_iter(), &hooks);
    let below = pixel().unwrap();
    let graded =
        apply_effect(pixel().unwrap(), &grade_passes, &below, &CpuEffects, &hooks).unwrap();
    // The LUT follows the grade stages, not the other way round.
    let oetf = |v: f64| {
        if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    let eotf = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    // The basic grade's saturation couples channels, so permuting before or
    // after is observable (an affine per-channel map would commute).
    let grade_rgb = |c: [f64; 3]| {
        let x = c.map(|v| v * 0.5);
        let l = 0.2126 * x[0] + 0.7152 * x[1] + 0.0722 * x[2];
        x.map(|v| {
            let saturated = l + (v - l) * 2.0;
            (0.5 + 0.5 * saturated.clamp(0.0, 1.0)).clamp(0.0, 1.0)
        })
    };
    let straight = [0.1f64, 0.3, 0.6];
    let graded_rgb = grade_rgb(straight);
    let after = [
        eotf(oetf(graded_rgb[1])),
        eotf(oetf(graded_rgb[2])),
        eotf(oetf(graded_rgb[0])),
    ];
    let permuted = [oetf(straight[1]), oetf(straight[2]), oetf(straight[0])].map(eotf);
    let before = grade_rgb(permuted);
    for (actual, expected) in graded.pixels()[0].iter().zip(after) {
        assert!(
            (f64::from(*actual) - expected).abs() <= 2e-5,
            "grade-before-LUT {actual} != {expected}"
        );
    }
    assert!(
        after
            .iter()
            .zip(before)
            .any(|(after, before)| (after - before).abs() > 1e-3),
        "the ordering witness must differ"
    );
}

/// sRGB-encoded fixture component to the linear working space.
fn linear(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb_pixel(rgb: [f32; 3], alpha: f32) -> [f32; 4] {
    let l = rgb.map(linear);
    [l[0] * alpha, l[1] * alpha, l[2] * alpha, alpha]
}
fn keyed(key: [u8; 4], intensity: f64, shadow: f64, pixel: [f32; 4]) -> [f32; 4] {
    let passes = chroma_key_passes(key.map(|v| f64::from(v) / 255.0), intensity, shadow).unwrap();
    passes
        .first()
        .map_or(pixel, |pass| chroma_key_pixel(pixel, &pass.values()))
}
const GREEN_KEY: [u8; 4] = [0, 255, 0, 255];

#[test]
fn chroma_key_removes_a_green_screen_at_several_intensities() {
    // Generated fixture: shaded green screen, skin and grey foreground, a
    // linear-light edge between them and a half-transparent green patch.
    let pure = srgb_pixel([0.0, 1.0, 0.0], 1.0);
    let dark = srgb_pixel([0.0, 0.5, 0.0], 1.0);
    let skin = srgb_pixel([0.76, 0.55, 0.42], 1.0);
    let grey = srgb_pixel([0.5, 0.5, 0.5], 1.0);
    let edge = std::array::from_fn(|i| pure[i] * 0.5 + skin[i] * 0.5);
    let translucent = srgb_pixel([0.0, 1.0, 0.0], 0.5);
    for intensity in [0.0, 40.0, 100.0] {
        for pass in
            chroma_key_passes(GREEN_KEY.map(|v| f64::from(v) / 255.0), intensity, 0.0).unwrap()
        {
            validate_pass(&pass, 8, 4).unwrap();
        }
    }
    // Intensity 0 is the reference's no-op and emits no pass.
    assert!(
        chroma_key_passes(GREEN_KEY.map(|v| f64::from(v) / 255.0), 0.0, 0.0)
            .unwrap()
            .is_empty()
    );
    let identity = keyed(GREEN_KEY, 0.0, 0.0, pure);
    assert_eq!(identity, pure);
    // The exact key colour is fully removed at every keying intensity.
    for intensity in [20.0, 40.0, 100.0] {
        assert_eq!(
            keyed(GREEN_KEY, intensity, 0.0, pure)[3],
            0.0,
            "{intensity}"
        );
        assert_eq!(keyed(GREEN_KEY, intensity, 0.0, translucent)[3], 0.0);
    }
    // A shaded, darker screen keys more as intensity rises but stays soft.
    let shaded = |intensity| keyed(GREEN_KEY, intensity, 0.0, dark)[3];
    assert!(shaded(20.0) > shaded(40.0));
    assert!(shaded(40.0) > shaded(100.0));
    assert!(shaded(100.0) < 0.01);
    // Foreground colours far from the key keep full alpha.
    assert_eq!(keyed(GREEN_KEY, 40.0, 0.0, skin)[3], 1.0);
    assert_eq!(keyed(GREEN_KEY, 40.0, 0.0, grey)[3], 1.0);
    // A linear-light half-and-half edge keeps partial alpha (edge softness).
    let soft = keyed(GREEN_KEY, 40.0, 0.0, edge)[3];
    assert!(soft > 0.05 && soft < 0.95, "edge alpha {soft}");
    // A non-matching key colour leaves green alone.
    assert_eq!(keyed([255, 0, 0, 255], 100.0, 0.0, pure)[3], 1.0);
}

#[test]
fn chroma_key_suppresses_spill_without_a_control() {
    // Light green cast over skin: mostly kept, but the dominant channel loses
    // its excess over the other two (the reference's spill stage at full strength).
    let cast = srgb_pixel([0.55, 0.60, 0.5], 1.0);
    let out = keyed(GREEN_KEY, 40.0, 0.0, cast);
    assert!(out[3] > 0.8, "cast should stay foreground: {}", out[3]);
    let encoded = |v: f32| {
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    let green = encoded(out[1] / out[3]);
    assert!((green - 0.55).abs() < 1e-4, "spill green {green}");
    assert!((out[1] - out[0]).abs() < 1e-5, "green no longer dominates");
    // Shadow restores pixels darker than the key, as the reference shows.
    let dark = srgb_pixel([0.0, 0.35, 0.0], 1.0);
    let plain = keyed(GREEN_KEY, 40.0, 0.0, dark)[3];
    let restored = keyed(GREEN_KEY, 40.0, 100.0, dark)[3];
    assert!(restored > plain, "shadow {restored} vs {plain}");
}

#[test]
fn chroma_key_refuses_hostile_values_and_bad_passes() {
    for raw in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001, 1.001] {
        for intensity in [0.0, 40.0] {
            assert!(matches!(
                chroma_key_passes([raw, 0.0, 0.0, 1.0], intensity, 0.0),
                Err(render::Error::Invalid(_))
            ));
        }
    }
    for bad in [f64::NAN, f64::INFINITY, -0.1, 100.1] {
        assert!(chroma_key_passes(GREEN_KEY.map(|v| f64::from(v) / 255.0), bad, 0.0).is_err());
        assert!(chroma_key_passes(GREEN_KEY.map(|v| f64::from(v) / 255.0), 40.0, bad).is_err());
    }
    for weights in [
        vec![0.0, 1.0, 0.0, 0.4],
        vec![0.0, 1.0, 0.0, 0.4, 0.0, 0.0],
        vec![0.0, 1.0, 0.0, f32::NAN, 0.0],
        vec![0.0, 1.0, 2.0, 0.4, 0.0],
        vec![0.0, 1.0, 0.0, 0.4, 1.5],
    ] {
        assert!(
            validate_pass(
                &EffectPass::Pixel {
                    entry_point: "chroma_key",
                    uniforms: weights.into_iter().map(Uniform::Float).collect(),
                },
                8,
                4
            )
            .is_err()
        );
    }
    // A grey key has no chroma direction and leaves every pixel unchanged.
    let skin = srgb_pixel([0.76, 0.55, 0.42], 1.0);
    let untouched = keyed([128, 128, 128, 255], 100.0, 100.0, skin);
    for (a, b) in untouched.iter().zip(skin) {
        assert!((a - b).abs() < 1e-6, "{untouched:?}");
    }
}

#[test]
fn masked_item_chroma_key_composes_with_track_below() {
    use project::*;
    let hooks = Hooks::default();
    let mut data = Project::empty(ProjectId(1), "key".into())
        .unwrap()
        .into_data();
    data.canvas.width = 40;
    data.canvas.height = 40;
    data.canvas.background = Color([0, 0, 255, 255]);
    let mut mask = Mask::new("m", MaskShape::Rectangle);
    mask.width = 0.5;
    mask.height = 1.0;
    mask.center_x = -0.25;
    let item = |effects: Vec<EffectInstance>| Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        id: ItemId(1),
        masks: vec![mask.clone()],
        effects,
        animations: Default::default(),
        source_asset: None,
        source_range: time::SourceRange::new(
            time::Tick::ZERO,
            time::FrameRate::FPS_30.tick_of(30).unwrap(),
        )
        .unwrap(),
        start: time::Tick::ZERO,
        content: ItemParams::Overlay(OverlayParams {
            color: Color([0, 255, 0, 255]),
        }),
    };
    let render = |effects| {
        let mut data = data.clone();
        data.scene_mut().timeline.main.items.push(item(effects));
        let project = Project::new(data).unwrap();
        Renderer::new(&project, RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &hooks)
            .unwrap()
    };
    // An exact-keyed item contributes no pixels, so the track below survives.
    let keyed_out = render(vec![
        effect_definition("chroma_key")
            .unwrap()
            .instance("chroma_key")
            .with("key_color", EffectValue::Color(Color(GREEN_KEY)))
            .unwrap()
            .with("intensity", EffectValue::Scalar(40.0))
            .unwrap()
            .with("shadow", EffectValue::Scalar(0.0))
            .unwrap(),
    ]);
    let at = |image: &Image, x: usize| image.pixels()[20 * 40 + x];
    for x in [5, 35] {
        assert_eq!(at(&keyed_out, x), [0.0, 0.0, 1.0, 1.0], "x={x}");
    }
    // A near key keeps partial alpha: inside the mask it blends, outside it is
    // fully masked out by the coverage stage after the effect.
    let partial = render(vec![
        effect_definition("chroma_key")
            .unwrap()
            .instance("chroma_key")
            .with("key_color", EffectValue::Color(Color([0, 255, 60, 255])))
            .unwrap()
            .with("intensity", EffectValue::Scalar(40.0))
            .unwrap()
            .with("shadow", EffectValue::Scalar(0.0))
            .unwrap(),
    ]);
    let inside = at(&partial, 5);
    let outside = at(&partial, 35);
    // Inside: the partly keyed green keeps some transparency, so the blue
    // backdrop shows through; spill leaves no green cast.
    assert!(
        inside[0] == 0.0 && inside[2] > 0.2 && inside[2] < 1.0 && inside[3] == 1.0,
        "inside {inside:?}"
    );
    assert_eq!(outside, [0.0, 0.0, 1.0, 1.0], "outside {outside:?}");
}

#[test]
fn masked_item_grade_applies_only_inside_the_mask() {
    use project::*;
    let hooks = Hooks::default();
    let mut data = Project::empty(ProjectId(1), "grade".into())
        .unwrap()
        .into_data();
    data.canvas.width = 40;
    data.canvas.height = 40;
    data.canvas.background = Color([0; 4]);
    let mut mask = Mask::new("m", MaskShape::Rectangle);
    mask.width = 0.5;
    mask.height = 1.0;
    mask.center_x = -0.25;
    let item = |effects: Vec<EffectInstance>| Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        id: ItemId(1),
        masks: vec![mask.clone()],
        effects,
        animations: Default::default(),
        source_asset: None,
        source_range: time::SourceRange::new(
            time::Tick::ZERO,
            time::FrameRate::FPS_30.tick_of(30).unwrap(),
        )
        .unwrap(),
        start: time::Tick::ZERO,
        content: ItemParams::Overlay(OverlayParams {
            color: Color([128, 128, 128, 255]),
        }),
    };
    let render = |effects| {
        let mut data = data.clone();
        data.scene_mut().timeline.main.items.push(item(effects));
        let project = Project::new(data).unwrap();
        Renderer::new(&project, RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &hooks)
            .unwrap()
    };
    let plain = render(vec![]);
    let bright = render(vec![
        effect_definition("color_grade")
            .unwrap()
            .instance("color_grade")
            .with("brightness", EffectValue::Scalar(50.0))
            .unwrap()
            .with("contrast", EffectValue::Scalar(0.0))
            .unwrap()
            .with("saturation", EffectValue::Scalar(0.0))
            .unwrap()
            .with("hue", EffectValue::Scalar(0.0))
            .unwrap(),
    ]);
    let at = |image: &Image, x: usize| image.pixels()[20 * 40 + x];
    // Inside the mask the grade doubles the linear value; outside is transparent.
    assert!(at(&bright, 5)[0] > at(&plain, 5)[0] * 1.5);
    assert_eq!(at(&bright, 35)[3], 0.0);
    assert_eq!(at(&plain, 35)[3], 0.0);
}

#[test]
fn every_catalog_kind_has_an_evaluator_and_frames_carry_item_local_time() {
    use project::*;
    use time::Tick;
    let none = Animations::default();
    for definition in effect_definitions() {
        let evaluate = evaluator(definition.id)
            .unwrap_or_else(|| panic!("{} has no evaluator", definition.id));
        let frame =
            EffectFrame::new(&none, &definition.instance("one"), Tick::ZERO, 64, 36).unwrap();
        for pass in evaluate(&frame, &EffectGeometry::new([64, 36], [1.0, 1.0]).unwrap()).unwrap() {
            validate_pass(&pass, 8, 4).unwrap();
        }
    }
    assert!(evaluator("nope").is_none());

    // The frame hands time-driven effects the item-local time and resolves a
    // keyframed scalar to its hard range, whatever the curve says.
    let mut animations = Animations::default();
    animations
        .set(
            AnimationPath::new("effects.b.params.intensity").unwrap(),
            Param::new(ParamValue::Scalar(1000.0)).unwrap(),
        )
        .unwrap();
    let blur = effect_definition("blur").unwrap().instance("b");
    let at = Tick(254_016_000_000 / 2);
    let frame = EffectFrame::new(&animations, &blur, at, 64, 36).unwrap();
    assert_eq!(frame.local, at);
    assert_eq!(frame.seconds, 0.5);
    assert_eq!((frame.width, frame.height), (64, 36));
    assert_eq!(frame.scalar("intensity").unwrap(), 100.0);
    assert!(
        frame.color("intensity").is_err(),
        "typed accessors refuse other kinds"
    );
    assert!(frame.scalar("nope").is_err());
    let unkeyed = EffectFrame::new(&none, &blur, at, 64, 36).unwrap();
    assert_eq!(unkeyed.scalar("intensity").unwrap(), 15.0);
}

#[test]
fn an_adapter_that_lacks_a_pass_falls_back_to_cpu_for_it() {
    use project::*;
    use time::{FrameRate, SourceRange, Tick};
    struct GaussianOnly(std::sync::atomic::AtomicUsize);
    impl EffectCompositor for GaussianOnly {
        fn frame<'a>(
            &'a self,
            _: &'a dyn render::ExecutionHooks,
        ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
            Ok(None)
        }
        fn release_idle(&self) -> render::Result<()> {
            Ok(())
        }
        fn supports(&self, _: &str) -> bool {
            true
        }
        fn apply(
            &self,
            item: &Image,
            passes: &[EffectPass],
            inputs: EffectInputs<'_>,
            hooks: &dyn render::ExecutionHooks,
        ) -> render::Result<Image> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if passes
                .iter()
                .any(|pass| !matches!(pass, EffectPass::Convolve { .. }))
            {
                return Err(render::Error::Unsupported("GPU shader entry"));
            }
            CpuEffects.apply(item, passes, inputs, hooks)
        }
    }
    let hooks = Hooks::default();
    let mut data = Project::empty(ProjectId(1), "grade".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 36;
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: vec![],
            effects: vec![
                effect_definition("color_grade")
                    .unwrap()
                    .instance("g")
                    .with("brightness", EffectValue::Scalar(50.0))
                    .unwrap(),
            ],
            animations: Default::default(),
            id: ItemId(1),
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, FrameRate::FPS_30.tick_of(30).unwrap())
                .unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([100, 50, 20, 255]),
            }),
        }],
    });
    let project = Project::new(data).unwrap();
    let render = |settings: RenderSettings| {
        Renderer::new(&project, settings)
            .unwrap()
            .render_frame(Tick::ZERO, &mut FileSources::default(), &hooks)
            .unwrap()
    };
    let reference = render(RenderSettings::from_project(&project));
    let adapter = Arc::new(GaussianOnly(Default::default()));
    let mut settings = RenderSettings::from_project(&project);
    settings.effects = Some(adapter.clone());
    let fallback = render(settings);
    assert_eq!(adapter.0.load(Ordering::SeqCst), 1, "the adapter was asked");
    assert_eq!(fallback.pixels(), reference.pixels());
}

#[test]
fn per_run_supports_keeps_supported_runs_on_the_adapter() {
    use project::*;
    use std::sync::Mutex;
    use time::Tick;
    /// Claims the gaussian entry only, so a Pixel run in the same item must go to CPU.
    struct GaussianOnly(Mutex<Vec<&'static str>>);
    impl EffectCompositor for GaussianOnly {
        fn frame<'a>(
            &'a self,
            _: &'a dyn render::ExecutionHooks,
        ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
            Ok(None)
        }
        fn release_idle(&self) -> render::Result<()> {
            Ok(())
        }
        fn supports(&self, entry_point: &str) -> bool {
            entry_point == "gaussian"
        }
        fn apply(
            &self,
            item: &Image,
            passes: &[EffectPass],
            inputs: EffectInputs<'_>,
            hooks: &dyn ExecutionHooks,
        ) -> render::Result<Image> {
            for pass in passes {
                self.0.lock().unwrap().push(pass.entry_point());
            }
            CpuEffects.apply(item, passes, inputs, hooks)
        }
    }
    let mut data = crate::support::solid_project(1, 64, 36, [100, 50, 20, 255]).into_data();
    let item = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .next()
        .unwrap();
    item.effects.push(
        effect_definition("blur")
            .unwrap()
            .instance("b")
            .with("intensity", EffectValue::Scalar(100.0))
            .unwrap(),
    );
    item.effects.push(
        effect_definition("color_grade")
            .unwrap()
            .instance("g")
            .with("brightness", EffectValue::Scalar(50.0))
            .unwrap(),
    );
    let project = Project::new(data).unwrap();
    let hooks = Hooks::default();
    let render = |settings: RenderSettings| {
        Renderer::new(&project, settings)
            .unwrap()
            .render_frame(Tick::ZERO, &mut crate::support::Empty, &hooks)
            .unwrap()
    };
    let reference = render(RenderSettings::from_project(&project));
    let adapter = Arc::new(GaussianOnly(Default::default()));
    let mut settings = RenderSettings::from_project(&project);
    settings.effects = Some(adapter.clone());
    let mixed = render(settings);
    assert_eq!(mixed.pixels(), reference.pixels());
    let asked = adapter.0.lock().unwrap();
    assert!(
        !asked.is_empty() && asked.iter().all(|entry| *entry == "gaussian"),
        "the adapter saw {asked:?}"
    );
}

/// Opaque frames that differ only with the source tick: red before 0.05 s, green after.
struct TimeFrames;
impl SourceProvider for TimeFrames {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        at: time::Tick,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        let colour = if at.seconds() < 0.05 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        };
        let bytes: Vec<u8> = colour.iter().copied().cycle().take(16 * 4).collect();
        Ok(Some(Image::from_rgba8(4, 4, &bytes, false, 13, hooks)?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, hooks)
    }
}

/// A 4x4, 2-second video item whose effects are `effects`.
fn content_scene(effects: Vec<project::EffectInstance>) -> project::Project {
    use project::*;
    use time::{SourceRange, Tick};
    let asset = testkit::source::asset(1, std::env::temp_dir().join("render-content-time"))
        .format("synthetic")
        .duration(tick(2, 1))
        .stream(
            testkit::source::video()
                .codec("rgba")
                .time_base(media::TimeBase::new(1, 30).unwrap())
                .duration(tick(2, 1))
                .start(Tick::ZERO)
                .frame_count(60)
                .variable_frame_rate(false)
                .size(4, 4)
                .build(),
        )
        .build();
    let mut data = Project::empty(ProjectId(7), "content".into())
        .unwrap()
        .into_data();
    data.canvas.width = 4;
    data.canvas.height = 4;
    data.assets.push(asset);
    data.scene_mut().timeline.main.items.push(Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: vec![],
        effects,
        animations: Default::default(),
        id: ItemId(1),
        source_asset: Some(media::AssetId(1)),
        source_range: SourceRange::new(Tick::ZERO, tick(2, 1)).unwrap(),
        start: Tick::ZERO,
        content: ItemParams::Video(VideoParams {
            crop: None,
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            cleanup: Default::default(),
        }),
    });
    Project::new(data).unwrap()
}

#[test]
fn content_time_holds_the_media_frame_at_the_effect_rate() {
    use project::*;
    use time::Tick;
    let hooks = Hooks::default();
    let hold = effect_definition("hold_time")
        .unwrap()
        .instance("hold")
        .with("rate", EffectValue::Scalar(2.0))
        .unwrap();
    let held = content_scene(vec![hold]);
    let plain = content_scene(vec![]);
    let render = |project: &Project, at: Tick| {
        Renderer::new(project, RenderSettings::from_project(project))
            .unwrap()
            .render_frame(at, &mut TimeFrames, &hooks)
            .unwrap()
    };
    let tenth = tick(1, 10);
    // Without the effect the content follows the item clock.
    assert_eq!(render(&plain, Tick::ZERO).pixels()[0], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(render(&plain, tenth).pixels()[0], [0.0, 1.0, 0.0, 1.0]);
    // With a 2 fps hold both ticks sample the content at 0: the picture is held
    // while the item clock (and so every parameter) keeps running.
    let first = render(&held, Tick::ZERO);
    assert_eq!(first.pixels()[0], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(render(&held, tenth).pixels(), first.pixels());
    // The remap is a registry lookup, not a special case in the renderer.
    assert!(content_remap("hold_time").is_some());
    assert!(content_remap("blur").is_none());
}

#[test]
fn preview_quality_reaches_the_evaluator_and_stays_close_to_fine() {
    use project::*;
    use time::Tick;
    let hooks = Hooks::default();
    let view = effect_definition("hold_time").unwrap().instance("probe");
    let passes = |fine| {
        evaluate_effect(
            &Animations::default(),
            &view,
            Tick::ZERO,
            render::effects::EffectGeometry::new([64, 36], [1.0, 1.0]).unwrap(),
            fine,
            &render::lut::LutResources::default(),
        )
        .unwrap()
    };
    // EffectFrame is fine by default, and the evaluator sees the renderer's flag.
    let frame = EffectFrame::new(&Animations::default(), &view, Tick::ZERO, 64, 36).unwrap();
    assert!(frame.fine);
    let hue = |passes: &[EffectPass]| match passes.first() {
        Some(EffectPass::Pixel { uniforms, .. }) => uniforms.get(3).copied(),
        _ => None,
    };
    assert_eq!(hue(&passes(true)), Some(Uniform::Float(0.0)));
    assert_eq!(hue(&passes(false)), Some(Uniform::Float(1e-3)));
    // Coarse stays within a stated tolerance of fine on the colour chart.
    let source = chart();
    let image =
        Image::from_working_pixels(source.len() as u32, 1, source.iter().copied(), &hooks).unwrap();
    let apply = |passes: Vec<EffectPass>| {
        CpuEffects
            .apply(
                &image,
                &passes,
                EffectInputs {
                    input: &image,
                    coverage: None,
                    below: &image,
                },
                &hooks,
            )
            .unwrap()
    };
    let fine = apply(passes(true));
    let coarse = apply(passes(false));
    for (a, b) in fine.pixels().iter().zip(coarse.pixels()) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() <= 2e-3, "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn sample_pass_mirrors_through_the_bilinear_helper() {
    let hooks = Hooks::default();
    // Checker plus impulse: exact integer reflections must not blur columns,
    // mix rows or lose alpha at the final row/column.
    let source = (0..32).map(|i| {
        [
            ((i % 8) % 2) as f32,
            (i / 8 % 2) as f32,
            f32::from(i == 3),
            1.0,
        ]
    });
    let image = Image::from_working_pixels(8, 4, source, &hooks).unwrap();
    let mirror = |values: [f32; 4]| EffectPass::Sample {
        entry_point: "mirror",
        uniforms: values.map(Uniform::Float).to_vec(),
    };
    // Reflect about the vertical line through the frame centre.
    let pass = mirror([4.0, 2.0, 1.0, 0.0]);
    validate_pass(&pass, 8, 4).unwrap();
    let at = |image: &Image, x: usize, y: usize| image.pixels()[y * 8 + x];
    let output = CpuEffects
        .apply(
            &image,
            &[pass],
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    assert_eq!(output.width(), 8);
    assert_eq!(output.height(), 4);
    for y in 0..4 {
        for (x, sx) in [0, 1, 2, 3, 3, 2, 1, 0].into_iter().enumerate() {
            assert_eq!(at(&output, x, y), at(&image, sx, y), "x={x} y={y}");
        }
    }
    for (values, expected_y) in [([4.0, 2.0, 0.0, 1.0], [0, 1, 1, 0])] {
        let reflected = CpuEffects
            .apply(
                &image,
                &[mirror(values)],
                EffectInputs {
                    input: &image,
                    coverage: None,
                    below: &image,
                },
                &hooks,
            )
            .unwrap();
        for (y, sy) in expected_y.into_iter().enumerate() {
            for x in 0..8 {
                assert_eq!(at(&reflected, x, y), at(&image, x, sy));
            }
        }
    }
    let fractional = CpuEffects
        .apply(
            &image,
            &[mirror([4.25, 2.0, 1.0, 0.0])],
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    // x=4 reflects to edge coordinate 4.0, halfway between texels 3 and 4.
    for y in 0..4 {
        assert_eq!(
            at(&fractional, 4, y),
            [0.5, (y % 2) as f32, if y == 0 { 0.5 } else { 0.0 }, 1.0]
        );
    }
    // Same input, same output: no clock, no global state.
    let again = CpuEffects
        .apply(
            &image,
            &[mirror([4.0, 2.0, 1.0, 0.0])],
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    assert_eq!(again.pixels(), output.pixels());
    // Hostile uniforms and an unknown op are refused before any pixel is written.
    for bad in [
        mirror([4.0, 2.0, 0.0, 0.0]),
        mirror([f32::NAN, 2.0, 1.0, 0.0]),
        mirror([4.0, 2.0, f32::INFINITY, 0.0]),
        EffectPass::Sample {
            entry_point: "nope",
            uniforms: vec![Uniform::Float(0.0)],
        },
    ] {
        assert!(validate_pass(&bad, 8, 4).is_err(), "{bad:?}");
    }
    // An adapter without a pipeline for the pass answers Unsupported; the
    // renderer then runs the rest of the list on the CPU reference above.
    struct PixelOnly;
    impl EffectCompositor for PixelOnly {
        fn frame<'a>(
            &'a self,
            _: &'a dyn render::ExecutionHooks,
        ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
            Ok(None)
        }
        fn release_idle(&self) -> render::Result<()> {
            Ok(())
        }
        fn supports(&self, entry_point: &str) -> bool {
            entry_point != "mirror"
        }
        fn apply(
            &self,
            item: &Image,
            passes: &[EffectPass],
            inputs: EffectInputs<'_>,
            hooks: &dyn ExecutionHooks,
        ) -> Result<Image> {
            if passes
                .iter()
                .any(|pass| matches!(pass, EffectPass::Sample { .. }))
            {
                return Err(Error::Unsupported("GPU shader entry"));
            }
            CpuEffects.apply(item, passes, inputs, hooks)
        }
    }
    assert!(matches!(
        PixelOnly.apply(
            &image,
            &[mirror([4.0, 2.0, 1.0, 0.0])],
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image
            },
            &hooks
        ),
        Err(Error::Unsupported(_))
    ));
    // The pass lists flatten their uniforms like a pixel op does.
    let pass = mirror([4.0, 2.0, 1.0, 0.0]);
    assert_eq!(pass.values().as_ref(), [4.0, 2.0, 1.0, 0.0]);
}

#[test]
fn chained_remaps_sample_content_in_order_but_all_parameters_keep_real_time() {
    use project::*;
    use time::Tick;
    struct Recording(Vec<Tick>);
    impl SourceProvider for Recording {
        fn status(
            &mut self,
            _: &media::SourceAsset,
            _: bool,
            _: &dyn ExecutionHooks,
        ) -> render::Result<Option<UnavailableReason>> {
            Ok(None)
        }
        fn video(
            &mut self,
            _: &media::SourceAsset,
            _: ItemId,
            at: Tick,
            hooks: &dyn ExecutionHooks,
        ) -> render::Result<Option<Image>> {
            self.0.push(at);
            Ok(Some(Image::new(4, 4, [0.0, 0.25, 0.0, 1.0], hooks)?))
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: ItemId,
            _: i64,
            n: usize,
            rate: u32,
            hooks: &dyn ExecutionHooks,
        ) -> render::Result<AudioBlock> {
            AudioBlock::new(rate, 2, n, MemoryKind::Decoded, hooks)
        }
    }
    let hold = |id, rate| {
        effect_definition("hold_time")
            .unwrap()
            .instance(id)
            .with("rate", EffectValue::Scalar(rate))
            .unwrap()
    };
    let key = effect_definition("chroma_key").unwrap().instance("key");
    let mut data = content_scene(vec![hold("first", 2.0), hold("later", 3.0), key]).into_data();
    let item = &mut data.scene_mut().timeline.main.items[0];
    for (path, before, after) in [
        ("effects.later.params.rate", 3.0, 4.0),
        ("effects.key.params.intensity", 100.0, 0.0),
        ("transform.positionX", 0.0, 1.0),
        ("opacity", 1.0, 0.5),
    ] {
        let mut first = Keyframe::new(Tick::ZERO, ParamValue::Scalar(before));
        first.segment = Segment::Step;
        item.animations
            .set(
                AnimationPath::new(path).unwrap(),
                Param::Value(
                    Channel::new(
                        ParamValue::Scalar(before),
                        vec![first, Keyframe::new(tick(3, 5), ParamValue::Scalar(after))],
                        Extrapolation::Hold,
                        Extrapolation::Hold,
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
    }
    let project = Project::new(data).unwrap();
    let item = &project.data().scene().timeline.main.items[0];
    let real = tick(4, 5);
    // First holds .8 to .5; later rate is 4 at real .8 (3 at held .5).
    // Using the real rate leaves .5, whereas the old code produced 1/3.
    assert_eq!(content_time(item, real, 4, 4).unwrap(), tick(1, 2));
    let later = EffectFrame::new(&item.animations, &item.effects[1], real, 4, 4).unwrap();
    assert_eq!(later.local, real);
    assert_eq!(later.scalar("rate").unwrap(), 4.0);
    assert_eq!(
        EffectFrame::new(&item.animations, &item.effects[1], tick(1, 2), 4, 4)
            .unwrap()
            .scalar("rate")
            .unwrap(),
        3.0
    );
    let mut sources = Recording(vec![]);
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let frame = renderer
        .render_frame(real, &mut sources, &Hooks::default())
        .unwrap();
    assert_eq!(sources.0, [tick(1, 2)]);
    // Actual pixels prove transform x=1, key intensity=0 and opacity=.5 on the real clock.
    assert_eq!(frame.pixels()[0], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(frame.pixels()[2], [0.0, 0.125, 0.0, 1.0]);
    // A second actual render keeps BOTH colour endpoints on real time, too.
    let mut coloured = project.data().clone();
    let target = &mut coloured.scene_mut().timeline.main.items[0];
    let tint = effect_definition("tint").unwrap().instance("tone");
    for name in ["black", "white"] {
        let mut colour = colour_curve([1.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0]);
        for (component, value) in [(ColorComponent::R, 0.0), (ColorComponent::B, 1.0)] {
            let channel = colour.channel_mut(Some(component)).unwrap();
            let mut first = channel.keys()[0].clone();
            first.segment = Segment::Step;
            channel.put(first).unwrap();
            let mut next = Keyframe::new(tick(3, 5), ParamValue::Scalar(value));
            next.segment = Segment::Step;
            channel.put(next).unwrap();
        }
        target
            .animations
            .set(tint.param_path(name).unwrap(), colour)
            .unwrap();
    }
    target.effects.push(tint);
    let coloured = Project::new(coloured).unwrap();
    let mut sources = Recording(vec![]);
    let mut renderer = Renderer::new(&coloured, RenderSettings::from_project(&coloured)).unwrap();
    let actual = renderer
        .render_frame(real, &mut sources, &Hooks::default())
        .unwrap();
    assert_eq!(sources.0, [tick(1, 2)]);
    assert_eq!(actual.pixels()[0], [0.0, 0.0, 0.0, 1.0]);
    assert_working_pixel(
        actual.pixels()[2],
        [0.0, 0.0, 0.5, 1.0],
        "colour uses real .8, not held .5",
    );
    let mut plain = item.clone();
    plain.effects.clear();
    for handle in [tick(-1, 2), tick(5, 2)] {
        assert_eq!(content_time(&plain, handle, 4, 4).unwrap(), handle);
    }
    assert_eq!(content_time(item, tick(-1, 2), 4, 4).unwrap(), Tick::ZERO);
    assert_eq!(content_time(item, tick(5, 2), 4, 4).unwrap(), tick(2, 1));
}

#[test]
fn mirror_refuses_extreme_geometry_before_allocating_and_keeps_admitted_results_bounded() {
    let hooks = Hooks::default();
    let image = Image::new(8, 4, [0.25, 0.0, 0.0, 1.0], &hooks).unwrap();
    let mirror = |v: [f32; 4]| EffectPass::Sample {
        entry_point: "mirror",
        uniforms: v.map(Uniform::Float).to_vec(),
    };
    let inputs = || EffectInputs {
        input: &image,
        coverage: None,
        below: &image,
    };
    let tiny = Hooks {
        limit: 0,
        ..Default::default()
    };
    for values in [
        [f32::MAX, 2.0, -1.0, 0.0],
        [-f32::MAX, 2.0, 1.0, 0.0],
        [4.0, f32::MAX, 0.0, -1.0],
        [4.0, -f32::MAX, 0.0, 1.0],
        [-0.25, 2.0, 1.0, 0.0],
        [8.25, 2.0, 1.0, 0.0],
        [4.0, -0.25, 0.0, 1.0],
        [4.0, 4.25, 0.0, 1.0],
        [4.0, 2.0, 2.0, 0.0],
        [4.0, 2.0, 0.5, 0.0],
        [4.0, 2.0, 1e-30, 0.0],
        [4.0, 2.0, f32::MAX, f32::MAX],
        [f32::NAN, 2.0, 1.0, 0.0],
        [4.0, f32::INFINITY, 1.0, 0.0],
        [4.0, 2.0, f32::NEG_INFINITY, 0.0],
    ] {
        for execution in [&hooks, &tiny] {
            assert!(
                matches!(
                    CpuEffects.apply(&image, &[mirror(values)], inputs(), execution),
                    Err(Error::Unsupported(_))
                ),
                "{values:?}"
            );
        }
        assert_eq!(
            tiny.requested.load(Ordering::SeqCst),
            0,
            "refusal before allocation"
        );
    }
    for values in [
        [0.0, 0.0, -1.0, 0.0],
        [8.0, 4.0, 0.0, -1.0],
        [8.0, 4.0, 0.6, 0.8],
        [0.0, 4.0, -0.6, -0.8],
    ] {
        let output = CpuEffects
            .apply(&image, &[mirror(values)], inputs(), &hooks)
            .unwrap();
        assert!(
            output
                .pixels()
                .iter()
                .all(|p| p.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= p[3]))
        );
    }
    let valid = mirror([4.0, 2.0, 1.0, 0.0]);
    assert!(matches!(
        CpuEffects.apply(&image, std::slice::from_ref(&valid), inputs(), &tiny),
        Err(Error::Budget { .. })
    ));
    let cancelled = Hooks {
        state: Arc::new(AtomicU8::new(1)),
        ..Default::default()
    };
    assert!(matches!(
        CpuEffects.apply(&image, &[valid], inputs(), &cancelled),
        Err(Error::Cancelled)
    ));
    let mismatch = Image::new(1, 1, [0.0; 4], &hooks).unwrap();
    for (input, below, coverage) in [
        (&mismatch, &image, None),
        (&image, &mismatch, None),
        (&image, &image, Some(&mismatch)),
    ] {
        assert!(matches!(
            CpuEffects.apply(
                &image,
                &[],
                EffectInputs {
                    input,
                    below,
                    coverage
                },
                &tiny
            ),
            Err(Error::Invalid("effect input dimensions"))
        ));
    }
}

#[test]
fn busy_frame_chooses_cpu_before_sources_and_output() {
    struct Busy(Arc<AtomicU8>);
    impl EffectCompositor for Busy {
        fn frame<'a>(
            &'a self,
            _: &'a dyn ExecutionHooks,
        ) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
            self.0.store(1, Ordering::SeqCst);
            Err(Error::Unsupported("contended frame"))
        }
        fn release_idle(&self) -> Result<()> {
            panic!("busy adapter reused")
        }
        fn supports(&self, _: &str) -> bool {
            panic!("busy adapter consulted")
        }
        fn apply(
            &self,
            _: &Image,
            _: &[EffectPass],
            _: EffectInputs<'_>,
            _: &dyn ExecutionHooks,
        ) -> Result<Image> {
            panic!("busy adapter called")
        }
    }
    struct Sources(Arc<AtomicU8>, Hooks);
    impl SourceProvider for Sources {
        fn begin_video(&mut self, _: &[project::ItemId]) {
            assert_eq!(
                self.0.load(Ordering::SeqCst),
                1,
                "scope decision precedes sources"
            );
            assert_eq!(
                self.1.used.load(Ordering::SeqCst),
                0,
                "scope decision precedes output"
            );
            self.0.store(2, Ordering::SeqCst);
        }
        fn status(
            &mut self,
            _: &media::SourceAsset,
            _: bool,
            _: &dyn ExecutionHooks,
        ) -> Result<Option<UnavailableReason>> {
            panic!("solid fixture")
        }
        fn video(
            &mut self,
            _: &media::SourceAsset,
            _: project::ItemId,
            _: time::Tick,
            _: &dyn ExecutionHooks,
        ) -> Result<Option<Image>> {
            panic!("solid fixture")
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: project::ItemId,
            _: i64,
            _: usize,
            _: u32,
            _: &dyn ExecutionHooks,
        ) -> Result<AudioBlock> {
            panic!("solid fixture")
        }
    }
    let mut data = crate::support::solid_project(1, 17, 9, [100, 50, 20, 255]).into_data();
    data.scene_mut().timeline.overlays[0].items[0].effects.push(
        project::effect_definition("color_grade")
            .unwrap()
            .instance("grade")
            .with("brightness", project::EffectValue::Scalar(30.0))
            .unwrap(),
    );
    let project = project::Project::new(data).unwrap();
    let hooks = Hooks::default();
    let reference = Renderer::new(&project, RenderSettings::from_project(&project))
        .unwrap()
        .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &hooks)
        .unwrap();
    let hooks = Hooks::default();
    let state = Arc::new(AtomicU8::new(0));
    let mut settings = RenderSettings::from_project(&project);
    settings.effects = Some(Arc::new(Busy(state.clone())));
    let actual = Renderer::new(&project, settings)
        .unwrap()
        .render_frame(
            time::Tick::ZERO,
            &mut Sources(state.clone(), hooks.clone()),
            &hooks,
        )
        .unwrap();
    assert_eq!(state.load(Ordering::SeqCst), 2);
    assert_eq!(actual.pixels(), reference.pixels());
    drop(actual);
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}

const PIXEL_EFFECTS: [&str; 11] = [
    "black_white",
    "channel_mix",
    "color_pass",
    "color_replace",
    "extract",
    "gamma_correction",
    "invert",
    "levels",
    "posterize",
    "strobe",
    "tint",
];

/// Chart plus a fully transparent working pixel, so alpha handling is exercised.
fn effect_chart() -> Vec<[f32; 4]> {
    let mut pixels = chart();
    // A near-red patch for Swap Colour's tight default similarity, then a
    // fully transparent working pixel so alpha handling is exercised.
    pixels.push([1.0, 0.006, 0.006, 1.0]);
    pixels.push([0.0; 4]);
    pixels
}

fn effect_instance(kind: &str, params: &[(&str, EffectValue)]) -> project::EffectInstance {
    let mut instance = project::effect_definition(kind).unwrap().instance(kind);
    for (id, value) in params {
        instance = instance.with(id, *value).unwrap();
    }
    instance
}

fn effect_output_at(
    kind: &str,
    params: &[(&str, EffectValue)],
    at: time::Tick,
    source: &[[f32; 4]],
) -> Vec<[f32; 4]> {
    let hooks = Hooks::default();
    let image =
        Image::from_working_pixels(source.len() as u32, 1, source.iter().copied(), &hooks).unwrap();
    let instance = effect_instance(kind, params);
    let passes = evaluate_effect(
        &project::Animations::default(),
        &instance,
        at,
        render::effects::EffectGeometry::new([source.len() as u32, 1], [1.0, 1.0]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    for pass in &passes {
        validate_pass(pass, image.width(), image.height()).unwrap();
    }
    CpuEffects
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap()
        .pixels()
        .to_vec()
}

fn effect_output(kind: &str, params: &[(&str, EffectValue)]) -> Vec<[f32; 4]> {
    effect_output_at(kind, params, time::Tick::ZERO, &effect_chart())
}

// Independent f64 sRGB equations, not render's transfer functions or packing LUT.
fn oracle_decode(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn oracle_encode(v: f64) -> f64 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn oracle_pixel(encoded: [f64; 3], alpha: f64) -> [f64; 4] {
    let [r, g, b] = encoded.map(|v| oracle_decode(v) * alpha);
    [r, g, b, alpha]
}
fn assert_working_pixel(actual: [f32; 4], expected: [f64; 4], context: &str) {
    for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            (f64::from(actual) - expected).abs() <= 3e-6,
            "{context} channel {channel}: {actual} != {expected}"
        );
    }
}

#[test]
fn pixel_equations_match_independent_colour_examples() {
    use EffectValue::{Bool as B, Color as C, Scalar as S};
    // Each expected encoded output is calculated independently of evaluated passes.
    // Nonlinear luma operations explicitly cross into linear light below.
    let input = [0.2, 0.4, 0.6];
    let luma = 0.2126 * oracle_decode(input[0])
        + 0.7152 * oracle_decode(input[1])
        + 0.0722 * oracle_decode(input[2]);
    let grey = oracle_encode(luma);
    let levels = [0.25_f64, 0.5, 0.75].map(|v| 0.1 + ((v - 0.2) / 0.6).sqrt() * 0.8);
    let mut rows = vec![
        ("black_white", vec![], input, [grey; 3]),
        (
            "gamma_correction",
            vec![("gamma", S(20.0))],
            input,
            [0.04, 0.16, 0.36],
        ),
        (
            "gamma_correction",
            vec![("gamma", S(5.0))],
            input,
            input.map(f64::sqrt),
        ),
        (
            "posterize",
            vec![("levels", S(5.0))],
            [0.12, 0.38, 0.88],
            [0.0, 0.5, 1.0],
        ),
        (
            "posterize",
            vec![("levels", S(5.0))],
            [0.374, 0.376, 0.874],
            [0.25, 0.5, 0.75],
        ),
        (
            "levels",
            vec![
                ("in_black", S(51.0)),
                ("in_white", S(204.0)),
                ("out_black", S(25.5)),
                ("out_white", S(229.5)),
                ("gamma", S(200.0)),
            ],
            [0.25, 0.5, 0.75],
            levels,
        ),
        (
            "levels",
            vec![("out_black", S(255.0)), ("out_white", S(0.0))],
            input,
            [0.8, 0.6, 0.4],
        ),
        // Equal/reversed input bounds use a .001 span without emitting white > 1.
        (
            "levels",
            vec![("in_black", S(255.0)), ("in_white", S(255.0))],
            [1.0; 3],
            [0.0; 3],
        ),
        (
            "levels",
            vec![("in_black", S(255.0)), ("in_white", S(0.0))],
            input,
            [0.0; 3],
        ),
        (
            "channel_mix",
            vec![
                ("rr", S(50.0)),
                ("rg", S(25.0)),
                ("rb", S(-50.0)),
                ("rc", S(20.0)),
                ("gr", S(100.0)),
                ("gg", S(0.0)),
                ("bg", S(50.0)),
                ("bb", S(50.0)),
                ("bc", S(10.0)),
            ],
            input,
            [0.1, 0.2, 0.6],
        ),
        (
            "channel_mix",
            vec![("rc", S(25.0)), ("gc", S(50.0)), ("bc", S(75.0))],
            [0.0; 3],
            [0.25, 0.5, 0.75],
        ),
        // Valid hard-range gains produce out-of-gamut colour; the operation clips it.
        (
            "channel_mix",
            vec![
                ("rr", S(200.0)),
                ("rg", S(200.0)),
                ("gg", S(-200.0)),
                ("bb", S(200.0)),
                ("bc", S(200.0)),
            ],
            input,
            [1.0, 0.0, 1.0],
        ),
        (
            "color_pass",
            vec![("similarity", S(50.0))],
            [0.6, 0.2, 0.0],
            [0.6, 0.2, 0.0],
        ),
        (
            "color_pass",
            vec![("similarity", S(0.0)), ("color", C(Color([0, 0, 0, 255])))],
            [0.0; 3],
            [0.0; 3],
        ),
        // Source f32 encoding of linear red is just below the exact key red;
        // at zero distance tolerance it is rejected rather than silently snapped.
        (
            "color_pass",
            vec![("similarity", S(0.0))],
            [1.0, 0.0, 0.0],
            [oracle_encode(0.2126); 3],
        ),
        (
            "color_pass",
            vec![("similarity", S(0.0)), ("reverse", B(true))],
            input,
            input,
        ),
        // Source HSL replacement retains L=.3 or .7 and sets hue/saturation to blue.
        (
            "color_replace",
            vec![("similarity", S(100.0))],
            [0.6, 0.0, 0.0],
            [0.0, 0.0, 0.6],
        ),
        (
            "color_replace",
            vec![("similarity", S(100.0))],
            [1.0, 0.4, 0.4],
            [0.4, 0.4, 1.0],
        ),
        (
            "color_replace",
            vec![("similarity", S(100.0)), ("solid", B(true))],
            [0.6, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        ),
        (
            "color_replace",
            vec![("similarity", S(0.0)), ("solid", B(true))],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        ),
        ("color_replace", vec![("similarity", S(0.0))], input, input),
        // d=.555 is halfway through [.51,.6], smoothstep=.5.
        (
            "color_replace",
            vec![("similarity", S(50.0)), ("solid", B(true))],
            [0.445, 0.0, 0.0],
            [0.2225, 0.0, 0.5],
        ),
        (
            "tint",
            vec![
                ("black", C(Color([255, 0, 0, 255]))),
                ("white", C(Color([0, 0, 255, 255]))),
                ("amount", S(25.0)),
            ],
            input,
            [
                0.75 * input[0] + 0.25 * (1.0 - grey),
                0.75 * input[1],
                0.75 * input[2] + 0.25 * grey,
            ],
        ),
        ("tint", vec![("amount", S(0.0))], input, input),
    ];
    for channel in 0..4 {
        for blend in [0.0, 30.0, 100.0] {
            let expected = std::array::from_fn(|k| {
                if channel == 0 || channel == k + 1 {
                    (1.0 - input[k]) * (1.0 - blend / 100.0) + input[k] * blend / 100.0
                } else {
                    input[k]
                }
            });
            rows.push((
                "invert",
                vec![
                    ("channel", EffectValue::Choice(channel as u32)),
                    ("blend", S(blend)),
                ],
                input,
                expected,
            ));
        }
    }
    for (kind, params, encoded, expected) in rows {
        for alpha in [0.0, 0.25, 0.5, 1.0] {
            let source = [oracle_pixel(encoded, alpha).map(|v| v as f32)];
            let actual = effect_output_at(kind, &params, time::Tick::ZERO, &source)[0];
            assert_working_pixel(
                actual,
                oracle_pixel(expected, alpha),
                &format!("{kind} {params:?} a={alpha}"),
            );
            assert_eq!(actual[3], source[0][3], "{kind} preserves coverage");
        }
    }
    // Outside Keep Colour's distance sphere becomes linear-luma grey, or stays on reverse.
    for reverse in [false, true] {
        let encoded = [0.4, 0.6, 0.0];
        let source = [oracle_pixel(encoded, 0.5).map(|v| v as f32)];
        let l = 0.2126 * oracle_decode(0.4) + 0.7152 * oracle_decode(0.6);
        let expected = if reverse {
            oracle_pixel(encoded, 0.5)
        } else {
            [l * 0.5, l * 0.5, l * 0.5, 0.5]
        };
        let output = effect_output_at(
            "color_pass",
            &[("similarity", S(50.0)), ("reverse", B(reverse))],
            time::Tick::ZERO,
            &source,
        );
        assert_working_pixel(output[0], expected, "Keep Colour outside sphere");
    }
}

#[test]
fn alpha_invert_blends_coverage_and_repremultiplies_the_same_colour() {
    let rows = [
        ([0.2, 0.1, 0.05, 0.25], 0.0, [0.6, 0.3, 0.15, 0.75]),
        ([0.2, 0.1, 0.05, 0.25], 50.0, [0.4, 0.2, 0.1, 0.5]),
        ([0.2, 0.1, 0.05, 0.25], 100.0, [0.2, 0.1, 0.05, 0.25]),
        ([0.0; 4], 0.0, [0.0, 0.0, 0.0, 1.0]),
        ([0.0; 4], 50.0, [0.0, 0.0, 0.0, 0.5]),
        ([0.0; 4], 100.0, [0.0; 4]),
        ([0.8, 0.4, 0.2, 1.0], 0.0, [0.0; 4]),
        ([0.8, 0.4, 0.2, 1.0], 50.0, [0.4, 0.2, 0.1, 0.5]),
        ([0.8, 0.4, 0.2, 1.0], 100.0, [0.8, 0.4, 0.2, 1.0]),
    ];
    for (input, blend, expected) in rows {
        let output = effect_output_at(
            "invert",
            &[
                ("channel", EffectValue::Choice(4)),
                ("blend", EffectValue::Scalar(blend)),
            ],
            time::Tick::ZERO,
            &[input],
        );
        assert_working_pixel(output[0], expected, "Alpha Invert");
    }
}

#[test]
fn threshold_matches_strict_cutoffs_and_independent_feather_values() {
    use EffectValue::{Bool as B, Scalar as S};
    for (black, white, softness, encoded, inside) in [
        (0.0, 255.0, 0.0, 0.0, 0.0),
        // The source f32 sRGB equation encodes linear white just below 1, so
        // it is inside the upper 255 cutoff (not an exact encoded endpoint).
        (0.0, 255.0, 0.0, 1.0, 1.0),
        (90.0, 160.0, 0.0, 90.0 / 255.0 - 0.00005, 0.0),
        (90.0, 160.0, 0.0, 90.0 / 255.0 + 0.00005, 1.0),
        (90.0, 160.0, 0.0, 160.0 / 255.0 - 0.00005, 1.0),
        (90.0, 160.0, 0.0, 160.0 / 255.0 + 0.00005, 0.0),
        (51.0, 204.0, 0.0, 0.5, 1.0),
        (51.0, 204.0, 50.0, 0.15, 0.0),
        (51.0, 204.0, 50.0, 0.25, 0.05 / 0.1001),
        (51.0, 204.0, 50.0, 0.5, 1.0),
        (51.0, 204.0, 50.0, 0.75, 0.05 / 0.1001),
        (204.0, 51.0, 50.0, 0.5, 0.0),
    ] {
        for invert in [false, true] {
            let source = [oracle_pixel([encoded; 3], 0.25).map(|v| v as f32)];
            let out = effect_output_at(
                "extract",
                &[
                    ("black", S(black)),
                    ("white", S(white)),
                    ("softness", S(softness)),
                    ("invert", B(invert)),
                ],
                time::Tick::ZERO,
                &source,
            );
            // Threshold's result is already linear black/white, not decoded grey.
            let v = if invert { 1.0 - inside } else { inside } * 0.25;
            assert_working_pixel(out[0], [v, v, v, 0.25], "Threshold feather/cutoff");
        }
    }
}

#[test]
fn strobe_uses_real_local_ticks_and_linear_original_picture_blend() {
    use EffectValue::{Color as C, Scalar as S};
    let input = oracle_pixel([0.2, 0.4, 0.6], 0.5);
    let source = [input.map(|v| v as f32)];
    let params = [
        ("color", C(Color([255, 0, 0, 255]))),
        ("blend", S(40.0)),
        ("duration", S(0.125)),
        ("period", S(0.5)),
    ];
    for (at, active) in [
        (time::Tick::ZERO, true),
        (tick(1, 8).saturating_sub(time::Tick(1)), true),
        (tick(1, 8), false),
        (tick(1, 2).saturating_sub(time::Tick(1)), false),
        (tick(1, 2), true),
    ] {
        let expected = if active {
            [0.3 + input[0] * 0.4, input[1] * 0.4, input[2] * 0.4, 0.5]
        } else {
            input
        };
        let out = effect_output_at("strobe", &params, at, &source);
        assert_working_pixel(out[0], expected, "Strobe timing/linear blend");
    }
    for (duration, period, at, active) in [
        (0.0, 0.5, time::Tick::ZERO, false),
        (0.0005, 0.0, tick(1, 1000), true),
        (0.0005, 0.0, tick(3, 4000), false),
    ] {
        let params = [("duration", S(duration)), ("period", S(period))];
        let out = effect_output_at("strobe", &params, at, &source);
        assert_working_pixel(
            out[0],
            if active { [0.5; 4] } else { input },
            "Strobe zero duration/period floor",
        );
    }
}

#[test]
fn pixel_parameters_refuse_hostile_static_values_without_mutation() {
    for kind in PIXEL_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for param in definition.params {
            if let Some((min, max)) = param.scalar_range() {
                for value in [
                    min - 1.0,
                    max + 1.0,
                    f64::NAN,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                ] {
                    let mut instance = definition.instance("one");
                    let before = instance.clone();
                    assert!(
                        matches!(
                            instance.set_scalar(param.id, value),
                            Err(project::Error::Invalid(_))
                        ),
                        "{kind}.{} admitted {value}",
                        param.id
                    );
                    assert_eq!(instance, before);
                }
            } else {
                assert!(
                    definition
                        .instance("one")
                        .with(param.id, EffectValue::Point([0.0, 0.0]))
                        .is_err(),
                    "{kind}.{} accepted another type",
                    param.id
                );
            }
        }
    }
    // Working-pixel ingress refuses out-of-gamut/nonfinite pixels; effects must not
    // weaken Image's admission to invent an invalid working image for this test.
    for pixel in [
        [1.1, 0.0, 0.0, 1.0],
        [-0.1, 0.0, 0.0, 1.0],
        [0.6, 0.0, 0.0, 0.5],
        [0.0, 0.0, 0.0, 1.1],
        [f32::NAN, 0.0, 0.0, 1.0],
        [f32::INFINITY, 0.0, 0.0, 1.0],
    ] {
        assert!(matches!(
            Image::from_working_pixels(1, 1, std::iter::once(pixel), &Hooks::default()),
            Err(Error::Invalid("working pixel"))
        ));
    }
}

/// A value each parameter has not: the other hard bound, the next option, the
/// negated flag or a colour the chart does not hold.
fn changed_value(param: &project::EffectParamDefinition) -> EffectValue {
    use project::{EffectParamKind, EffectValue};
    match param.kind {
        EffectParamKind::Float { min, max, .. } => {
            let current = param.default.scalar().unwrap();
            EffectValue::Scalar(if current > (min + max) / 2.0 {
                min
            } else {
                max
            })
        }
        EffectParamKind::Angle => EffectValue::Scalar(if param.default.scalar() == Some(0.0) {
            ANGLE_MAX
        } else {
            0.0
        }),
        EffectParamKind::Choice { options } => {
            EffectValue::Choice((param.default.choice().unwrap() + 1) % options.len() as u32)
        }
        EffectParamKind::Bool => EffectValue::Bool(!param.default.flag().unwrap()),
        EffectParamKind::Color => {
            EffectValue::Color(if param.default.color() == Some(Color([255, 0, 0, 255])) {
                Color([0, 255, 255, 255])
            } else {
                Color([255, 0, 0, 255])
            })
        }
        EffectParamKind::Point => EffectValue::Point([1.0, 1.0]),
        EffectParamKind::Lut => EffectValue::Lut(None),
        EffectParamKind::Curve => EffectValue::Curve(
            project::ToneCurve::new(&[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]).unwrap(),
        ),
    }
}

#[test]
fn pixel_effects_that_are_identity_at_default_emit_no_pass() {
    for kind in ["gamma_correction", "levels", "channel_mix"] {
        let passes = evaluate_effect(
            &project::Animations::default(),
            &effect_instance(kind, &[]),
            time::Tick::ZERO,
            render::effects::EffectGeometry::new([13, 1], [1.0, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        )
        .unwrap();
        assert!(passes.is_empty(), "{kind} emits an identity pass");
        assert_eq!(
            effect_output(kind, &[]),
            effect_chart(),
            "{kind} is identity"
        );
    }
}

#[test]
fn every_pixel_effect_parameter_changes_the_picture() {
    // Strobe's time gates need a moment where the default is idle; every other
    // parameter reads at the clip start.
    let idle = time::Tick(time::TICKS_PER_SECOND * 12 / 100);
    for kind in PIXEL_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for param in definition.params {
            let at = match (kind, param.id) {
                ("strobe", "duration") => idle,
                // Default .5s period fires here, changed 30s period is idle.
                ("strobe", "period") => tick(1, 2),
                _ => time::Tick::ZERO,
            };
            let base = effect_output_at(kind, &[], at, &effect_chart());
            let changed = effect_output_at(
                kind,
                &[(param.id, changed_value(param))],
                at,
                &effect_chart(),
            );
            assert_ne!(changed, base, "{kind}.{} changed nothing", param.id);
        }
    }
}

#[test]
fn animated_pixel_parameters_clamp_to_their_hard_range() {
    for kind in PIXEL_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for (param, (min, max)) in definition
            .params
            .iter()
            .filter(|p| p.animatable)
            .filter_map(|p| p.scalar_range().map(|range| (p, range)))
        {
            for (raw, expected) in [(1e9, max), (-1e9, min)] {
                let mut animations = project::Animations::default();
                animations
                    .set(
                        project::AnimationPath::new(format!("effects.one.params.{}", param.id))
                            .unwrap(),
                        project::Param::new(project::ParamValue::Scalar(raw)).unwrap(),
                    )
                    .unwrap();
                let instance = definition.instance("one");
                let frame =
                    EffectFrame::new(&animations, &instance, time::Tick::ZERO, 64, 36).unwrap();
                assert_eq!(
                    frame.scalar(param.id).unwrap(),
                    expected,
                    "{kind}.{}",
                    param.id
                );
                for pass in (evaluator(kind).unwrap())(
                    &frame,
                    &EffectGeometry::new([64, 36], [1.0, 1.0]).unwrap(),
                )
                .unwrap()
                {
                    validate_pass(&pass, 64, 36)
                        .unwrap_or_else(|error| panic!("{kind}.{} {error:?}", param.id));
                }
            }
        }
    }
}

#[test]
fn pixel_effects_keep_premultiplied_pixels_finite_at_hostile_bounds() {
    for kind in PIXEL_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for high in [false, true] {
            let mut params = Vec::new();
            for param in definition.params {
                let value = match param.kind {
                    project::EffectParamKind::Float { min, max, .. } => {
                        project::EffectValue::Scalar(if high { max } else { min })
                    }
                    project::EffectParamKind::Angle => {
                        project::EffectValue::Scalar(if high { ANGLE_MAX } else { -ANGLE_MAX })
                    }
                    project::EffectParamKind::Choice { options } => {
                        project::EffectValue::Choice(if high {
                            options.len() as u32 - 1
                        } else {
                            0
                        })
                    }
                    project::EffectParamKind::Bool => project::EffectValue::Bool(high),
                    project::EffectParamKind::Color => project::EffectValue::Color(if high {
                        Color([255, 255, 255, 255])
                    } else {
                        Color([0, 0, 0, 255])
                    }),
                    project::EffectParamKind::Point => {
                        project::EffectValue::Point([if high { 1.0 } else { 0.0 }; 2])
                    }
                    project::EffectParamKind::Lut => project::EffectValue::Lut(None),
                    project::EffectParamKind::Curve => project::EffectValue::Curve(
                        project::ToneCurve::new(if high {
                            &[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]]
                        } else {
                            &[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]
                        })
                        .unwrap(),
                    ),
                };
                params.push((param.id, value));
            }
            let output = effect_output(kind, &params);
            for (pixel, source) in output.iter().zip(effect_chart()) {
                assert!(
                    pixel.iter().all(|v| v.is_finite() && *v >= 0.0),
                    "{kind} {high} {pixel:?}"
                );
                assert!(
                    pixel[..3].iter().all(|v| *v <= pixel[3] + 1e-6),
                    "{kind} {high} {pixel:?}"
                );
                assert!(pixel[3] <= 1.0, "{kind} {high} {pixel:?}");
                if kind != "invert" || !high {
                    assert_eq!(pixel[3], source[3], "{kind} preserves alpha");
                } else {
                    // The all-high row selects Alpha with 100% original picture.
                    assert_working_pixel(*pixel, source.map(f64::from), "Alpha Invert blend100");
                }
            }
        }
    }
}

/// Regression signatures of the packed chart, not independent adaptation oracles.
/// Bless only after the independent equation rows pass and the artifacts are viewed.
#[test]
fn pixel_effect_oracle_rows() {
    type Row = (&'static str, Vec<(&'static str, EffectValue)>, u64);
    let rows: [Row; 11] = [
        ("black_white", vec![], 0xed1fade70be90ac9),
        (
            "channel_mix",
            vec![
                ("rr", EffectValue::Scalar(0.0)),
                ("rb", EffectValue::Scalar(100.0)),
                ("br", EffectValue::Scalar(100.0)),
                ("bb", EffectValue::Scalar(0.0)),
            ],
            0x996d26e2ef89db9b,
        ),
        (
            "color_pass",
            vec![("similarity", EffectValue::Scalar(40.0))],
            0xba9a2c5032d7ff78,
        ),
        (
            "color_replace",
            vec![
                ("similarity", EffectValue::Scalar(60.0)),
                ("target", EffectValue::Color(Color([0, 0, 255, 255]))),
                ("replace", EffectValue::Color(Color([255, 0, 0, 255]))),
            ],
            0x921bc1a126b7b3ab,
        ),
        (
            "extract",
            vec![
                ("black", EffectValue::Scalar(30.0)),
                ("white", EffectValue::Scalar(200.0)),
                ("softness", EffectValue::Scalar(20.0)),
            ],
            0xd55208ec5ff4c981,
        ),
        (
            "gamma_correction",
            vec![("gamma", EffectValue::Scalar(20.0))],
            0xa4a42cbe7fe537d5,
        ),
        (
            "invert",
            vec![("blend", EffectValue::Scalar(30.0))],
            0xd386f19aab25df7b,
        ),
        (
            "levels",
            vec![
                ("in_black", EffectValue::Scalar(20.0)),
                ("in_white", EffectValue::Scalar(220.0)),
                ("gamma", EffectValue::Scalar(150.0)),
            ],
            0x74d098ac863127be,
        ),
        (
            "posterize",
            vec![("levels", EffectValue::Scalar(5.0))],
            0xa97c91d97e8655f8,
        ),
        (
            "strobe",
            vec![
                ("color", EffectValue::Color(Color([255, 210, 0, 255]))),
                ("blend", EffectValue::Scalar(40.0)),
            ],
            0xef2c2ad47d878f97,
        ),
        (
            "tint",
            vec![("amount", EffectValue::Scalar(60.0))],
            0xdfee605bbb004c81,
        ),
    ];
    let hooks = Hooks::default();
    let mut actual = Vec::new();
    for (kind, params, expected) in rows {
        let output = effect_output(kind, &params);
        let image =
            Image::from_working_pixels(output.len() as u32, 1, output.iter().copied(), &hooks)
                .unwrap();
        let bytes = image.rgba8(13, &hooks).unwrap();
        let artifact = testkit::fixture::dir("effect-pixels").join(format!("{kind}.rgba"));
        std::fs::write(&artifact, &bytes.bytes).unwrap();
        let checksum = bytes.bytes.iter().fold(0xcbf29ce484222325u64, |hash, b| {
            (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        eprintln!("(\"{kind}\", 0x{checksum:016x}),");
        actual.push((kind, checksum, expected));
    }
    for (kind, checksum, expected) in actual {
        assert_eq!(checksum, expected, "{kind} oracle checksum");
    }
}

fn colour_curve(from: [f64; 4], to: [f64; 4]) -> project::Param {
    use project::*;
    Param::Color(Box::new(std::array::from_fn(|i| {
        Channel::new(
            ParamValue::Scalar(from[i]),
            vec![
                Keyframe::new(time::Tick::ZERO, ParamValue::Scalar(from[i])),
                Keyframe::new(tick(2, 1), ParamValue::Scalar(to[i])),
            ],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap()
    })))
}

fn colour_output(
    effect: &project::EffectInstance,
    animations: &project::Animations,
    at: time::Tick,
    source: [f32; 4],
    fine: bool,
) -> render::Result<[f32; 4]> {
    let hooks = Hooks::default();
    let image = Image::from_working_pixels(1, 1, [source].into_iter(), &hooks)?;
    let below = Image::new(1, 1, [0.0; 4], &hooks)?;
    let passes = evaluate_effect(
        animations,
        effect,
        at,
        render::effects::EffectGeometry::new([1, 1], [1.0, 1.0]).unwrap(),
        fine,
        &render::lut::LutResources::default(),
    )?;
    Ok(apply_effect(image, &passes, &below, &CpuEffects, &hooks)?.pixels()[0])
}

#[test]
fn animated_effect_colours_match_independent_linear_pixels() {
    use project::*;
    let a = 0.25;
    let midpoint = [0.125, 0.0, 0.125, a];
    for (kind, key, source, expected) in [
        ("strobe", "color", [0.0, a, 0.0, a], midpoint),
        ("tint", "black", [0.0, 0.0, 0.0, a], midpoint),
        ("tint", "white", [a; 4], midpoint),
        ("color_pass", "color", midpoint, midpoint),
        ("color_replace", "target", midpoint, [0.0, 0.0, a, a]),
        ("color_replace", "replace", [a, 0.0, 0.0, a], midpoint),
    ] {
        let mut effect = effect_definition(kind).unwrap().instance("one");
        if kind == "color_pass" {
            effect.set_scalar("similarity", 0.0).unwrap();
        }
        if kind == "color_replace" {
            effect.set_scalar("solid", 1.0).unwrap();
        }
        let mut animations = Animations::default();
        animations
            .set(
                effect.param_path(key).unwrap(),
                colour_curve([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]),
            )
            .unwrap();
        // Linear-light midpoint encodes to ~.73535698, never rounded to byte188.
        for fine in [true, false] {
            let actual = colour_output(&effect, &animations, tick(1, 1), source, fine).unwrap();
            assert_working_pixel(
                actual,
                expected.map(f64::from),
                &format!("{kind}.{key} fine={fine}"),
            );
            assert_eq!(actual[3], a);
        }
        // Alpha keys on source RGB-only controls do not alter image colour or coverage.
        animations
            .set(
                effect.param_path(key).unwrap(),
                colour_curve([0.5, 0.0, 0.5, 0.0], [0.5, 0.0, 0.5, 1.0]),
            )
            .unwrap();
        assert_eq!(
            colour_output(&effect, &animations, tick(0, 1), source, true).unwrap(),
            colour_output(&effect, &animations, tick(2, 1), source, true).unwrap()
        );
        for impostor in [
            ParamValue::Scalar(0.5),
            ParamValue::Bool(true),
            ParamValue::Text("#FF0000".into()),
        ] {
            animations
                .set(
                    effect.param_path(key).unwrap(),
                    Param::new(impostor).unwrap(),
                )
                .unwrap();
            assert!(
                matches!(
                    colour_output(&effect, &animations, tick(1, 1), source, true),
                    Err(render::Error::Invalid(_))
                ),
                "{kind}.{key}"
            );
        }
    }
    // Encoded distance around a floating, animated key has independently placed cutoffs.
    let key_encoded = oracle_encode(0.5);
    let encoded: [f64; 3] = [0.8, 0.0, 0.4];
    let distance = ((encoded[0] - key_encoded).powi(2) + (encoded[2] - key_encoded).powi(2)).sqrt();
    let grey = 0.2126 * oracle_decode(encoded[0]) + 0.0722 * oracle_decode(encoded[2]);
    for (delta, expected) in [
        (-0.01, [grey, grey, grey]),
        (0.01, encoded.map(oracle_decode)),
    ] {
        let effect = effect_definition("color_pass")
            .unwrap()
            .instance("one")
            .with(
                "similarity",
                EffectValue::Scalar(distance / 1.2 * 100.0 + delta),
            )
            .unwrap();
        let mut animations = Animations::default();
        animations
            .set(
                effect.param_path("color").unwrap(),
                colour_curve([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]),
            )
            .unwrap();
        let actual = colour_output(
            &effect,
            &animations,
            tick(1, 1),
            oracle_pixel(encoded, 0.25).map(|v| v as f32),
            true,
        )
        .unwrap();
        assert_working_pixel(
            actual,
            [
                expected[0] * 0.25,
                expected[1] * 0.25,
                expected[2] * 0.25,
                0.25,
            ],
            "animated key distance boundary",
        );
    }
    // Swap d=.555 is the midpoint of [.51,.6], giving independently known weight .5.
    let effect = effect_definition("color_replace")
        .unwrap()
        .instance("one")
        .with("similarity", EffectValue::Scalar(50.0))
        .unwrap()
        .with("solid", EffectValue::Bool(true))
        .unwrap();
    let mut animations = Animations::default();
    animations
        .set(
            effect.param_path("target").unwrap(),
            colour_curve([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]),
        )
        .unwrap();
    let input = [key_encoded - 0.555, 0.0, key_encoded];
    let actual = colour_output(
        &effect,
        &animations,
        tick(1, 1),
        oracle_pixel(input, 0.25).map(|v| v as f32),
        true,
    )
    .unwrap();
    assert_working_pixel(
        actual,
        oracle_pixel([input[0] * 0.5, 0.0, (input[2] + 1.0) * 0.5], 0.25),
        "animated soft replacement",
    );
    // Animated replacement also retains the source HSL lightness when solid=false.
    let effect = effect_definition("color_replace")
        .unwrap()
        .instance("one")
        .with("similarity", EffectValue::Scalar(100.0))
        .unwrap();
    let mut animations = Animations::default();
    animations
        .set(
            effect.param_path("replace").unwrap(),
            colour_curve([0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0]),
        )
        .unwrap();
    for (source, expected) in [
        ([0.6, 0.0, 0.0], [0.0, 0.0, 0.6]),
        ([1.0, 0.4, 0.4], [0.4, 0.4, 1.0]),
    ] {
        let actual = colour_output(
            &effect,
            &animations,
            tick(1, 1),
            oracle_pixel(source, 0.25).map(|v| v as f32),
            true,
        )
        .unwrap();
        assert_working_pixel(
            actual,
            oracle_pixel(expected, 0.25),
            "animated HSL replacement",
        );
    }
}

#[test]
fn effect_colour_bases_preserve_subbyte_precision_and_clamp_curves() {
    use project::*;
    let effect = effect_definition("strobe")
        .unwrap()
        .instance("one")
        .with("color", EffectValue::Color(Color([51, 102, 153, 0])))
        .unwrap();
    let source = [0.0, 0.25, 0.0, 0.25];
    let mut animations = Animations::default();
    let fallback = colour_output(&effect, &animations, tick(0, 1), source, true).unwrap();
    assert_working_pixel(
        fallback,
        oracle_pixel([0.2, 0.4, 0.6], 0.25),
        "static byte fallback",
    );
    let path = effect.param_path("color").unwrap();
    let base = [0.123456, 0.234567, 0.345678, 0.5];
    let mut param = Param::new(ParamValue::Color(base)).unwrap();
    let channel = param.channel_mut(Some(ColorComponent::R)).unwrap();
    channel
        .put(Keyframe::new(tick(1, 1), ParamValue::Scalar(0.9)))
        .unwrap();
    channel.remove(tick(1, 1)).unwrap();
    animations.set(path.clone(), param).unwrap();
    let changed = effect
        .clone()
        .with("color", EffectValue::Color(Color([0, 255, 0, 255])))
        .unwrap();
    for instance in [&effect, &changed] {
        let actual = colour_output(instance, &animations, tick(0, 1), source, true).unwrap();
        assert_working_pixel(
            actual,
            [base[0] * 0.25, base[1] * 0.25, base[2] * 0.25, 0.25],
            "explicit zero-key base",
        );
    }
    let first = colour_output(&effect, &animations, tick(0, 1), source, true).unwrap();
    animations
        .set(
            path.clone(),
            Param::new(ParamValue::Color([
                base[0] + 0.0001,
                base[1],
                base[2],
                base[3],
            ]))
            .unwrap(),
        )
        .unwrap();
    let second = colour_output(&effect, &animations, tick(0, 1), source, true).unwrap();
    assert_working_pixel(
        second,
        [
            (base[0] + 0.0001) * 0.25,
            base[1] * 0.25,
            base[2] * 0.25,
            0.25,
        ],
        "sub-byte edit",
    );
    assert!(((second[0] - first[0]) as f64 - 0.000025).abs() < 3e-6);
    // Finite Bezier overshoot clamps after interpolation; all stored keys remain admitted.
    let mut overshoot = colour_curve([0.5, 0.5, 0.25, 0.5], [0.5, 0.5, 0.25, 0.5]);
    for (component, dv) in [
        (ColorComponent::R, 2.0),
        (ColorComponent::G, -2.0),
        (ColorComponent::A, 2.0),
    ] {
        let c = overshoot.channel_mut(Some(component)).unwrap();
        c.set_handle(
            tick(0, 1),
            HandleSide::Out,
            Some(CurveHandle { dt: tick(2, 3), dv }),
            TangentMode::Broken,
        )
        .unwrap();
        c.set_handle(
            tick(2, 1),
            HandleSide::In,
            Some(CurveHandle {
                dt: tick(-2, 3),
                dv,
            }),
            TangentMode::Broken,
        )
        .unwrap();
        let mut first = c.keys()[0].clone();
        first.segment = Segment::Bezier;
        c.put(first).unwrap();
    }
    animations.set(path.clone(), overshoot).unwrap();
    assert_working_pixel(
        colour_output(&effect, &animations, tick(1, 1), source, true).unwrap(),
        [0.25, 0.0, 0.0625, 0.25],
        "Bezier overshoot",
    );
    let mut extrapolate = colour_curve([0.0, 1.0, 0.25, 1.0], [1.0, 0.0, 0.25, 0.0]);
    for c in [ColorComponent::R, ColorComponent::G] {
        extrapolate
            .channel_mut(Some(c))
            .unwrap()
            .set_extrapolation(Extrapolation::Linear, Extrapolation::Linear)
            .unwrap();
    }
    animations.set(path, extrapolate).unwrap();
    assert_working_pixel(
        colour_output(&effect, &animations, tick(4, 1), source, true).unwrap(),
        [0.25, 0.0, 0.0625, 0.25],
        "linear extrapolation",
    );
    let chroma = effect_definition("chroma_key")
        .unwrap()
        .instance("one")
        .with("intensity", EffectValue::Scalar(100.0))
        .unwrap();
    let expected =
        colour_output(&chroma, &Animations::default(), tick(0, 1), source, true).unwrap();
    let mut ignored = Animations::default();
    ignored
        .set(
            chroma.param_path("key_color").unwrap(),
            Param::new(ParamValue::Color([1.0, 0.0, 0.0, 1.0])).unwrap(),
        )
        .unwrap();
    assert_eq!(
        colour_output(&chroma, &ignored, tick(0, 1), source, true).unwrap(),
        expected,
        "chroma colour stays static"
    );
}

// Independent f64 corner weights: no production sampler/evaluator/kernel as expected data.
fn spatial_sample(pixels: &[[f64; 4]], [w, h]: [u32; 2], x: f64, y: f64) -> [f64; 4] {
    let x = x.clamp(0.0, f64::from(w - 1));
    let y = y.clamp(0.0, f64::from(h - 1));
    let (ix, iy) = (x.floor() as u32, y.floor() as u32);
    let (fx, fy) = (x.fract(), y.fract());
    let mut out = [0.0; 4];
    for (xx, wx) in [(ix, 1.0 - fx), ((ix + 1).min(w - 1), fx)] {
        for (yy, wy) in [(iy, 1.0 - fy), ((iy + 1).min(h - 1), fy)] {
            for k in 0..4 {
                out[k] += pixels[(yy * w + xx) as usize][k] * wx * wy;
            }
        }
    }
    out
}
// Counts are analytical table data: ceil(hypot(rounded components)) can add a
// spurious tap at an integer span (e.g. 3*sqrt(2) on each axis has span exactly 6).
fn streak_expected(
    pixels: &[[f64; 4]],
    size: [u32; 2],
    [dx, dy]: [f64; 2],
    taps: usize,
) -> Vec<[f64; 4]> {
    if taps == 0 {
        return pixels.to_vec();
    }
    assert!((2..=64).contains(&taps));
    // The pass boundary carries f32 uniforms and texel-index coordinates. Round
    // these explicitly; corner weights and accumulation stay independent f64.
    let [dx, dy] = [dx, dy].map(|v| v as f32);
    (0..size[0] * size[1])
        .map(|i| {
            let mut out = [0.0; 4];
            for tap in 0..taps {
                let fraction = tap as f32 / (taps - 1) as f32 - 0.5;
                let x = (i % size[0]) as f32 + dx * fraction;
                let y = (i / size[0]) as f32 + dy * fraction;
                let p = spatial_sample(pixels, size, f64::from(x), f64::from(y));
                for k in 0..4 {
                    out[k] += p[k] / taps as f64;
                }
            }
            out
        })
        .collect()
}
fn streak(length: f64, direction: f64) -> project::EffectInstance {
    project::effect_definition("directional_blur")
        .unwrap()
        .instance("streak")
        .with("length", EffectValue::Scalar(length))
        .unwrap()
        .with("direction", EffectValue::Scalar(direction))
        .unwrap()
}
#[test]
fn streak_taps_match_independent_impulse_checker_alpha_constants_and_boundaries() {
    for size in [[9, 5], [1, 1], [1, 7], [7, 1]] {
        for pattern in 0..3 {
            let pixels: Vec<[f64; 4]> = (0..size[0] * size[1])
                .map(|i| {
                    let a = if pattern == 0 {
                        if i == size[0] * size[1] / 2 { 0.5 } else { 0.0 }
                    } else if pattern == 1 {
                        if i % 2 == 0 { 0.25 } else { 1.0 }
                    } else {
                        0.5
                    };
                    [a, a * 0.5, a * 0.125, a]
                })
                .collect();
            let hooks = Hooks::default();
            let image = Image::from_working_pixels(
                size[0],
                size[1],
                pixels.iter().map(|p| p.map(|v| v as f32)),
                &hooks,
            )
            .unwrap();
            for (length, angle, gains, taps) in [
                (0.0, 0.0, [1.0, 1.0], 0),
                (0.2499, 0.0, [1.0, 1.0], 0),
                (0.25, 0.0, [1.0, 1.0], 2),
                (1.0, 90.0, [1.0, 1.0], 2),
                // Equal gains have span 2*length*gain at every angle. Keep
                // genuine spans on either side of six distinct from six itself.
                (2.0, 30.0, [1.5, 1.5], 6),
                (2.0, 45.0, [1.5, 1.5], 6),
                (2.0, 60.0, [1.5, 1.5], 6),
                (2.0 - f64::EPSILON, 45.0, [1.5, 1.5], 6),
                (2.0 + 2.0 * f64::EPSILON, 45.0, [1.5, 1.5], 7),
                (2.0, 45.0, [0.75, 0.75], 3),
                (2.0, 45.0, [0.75, 1.5], 5),
                (3.0, -360.0, [0.5, 0.5], 3),
                (3.0, 360.0, [0.25, 0.25], 2),
                (31.5, 0.0, [1.0, 1.0], 63),
                (32.1, 90.0, [1.0, 1.0], 64),
                (1000.0, 45.0, [1.0, 1.0], 64),
            ] {
                let mut outputs = Vec::new();
                for fine in [true, false] {
                    let passes = evaluate_effect(
                        &project::Animations::default(),
                        &streak(length, angle),
                        time::Tick::ZERO,
                        EffectGeometry::new(size, gains).unwrap(),
                        fine,
                        &render::lut::LutResources::default(),
                    )
                    .unwrap();
                    let out = apply_effect(
                        Image::from_working_pixels(
                            image.width(),
                            image.height(),
                            image.pixels().iter().copied(),
                            &hooks,
                        )
                        .unwrap(),
                        &passes,
                        &image,
                        &CpuEffects,
                        &hooks,
                    )
                    .unwrap();
                    let a = angle.to_radians();
                    let expected = streak_expected(
                        &pixels,
                        size,
                        [
                            2.0 * length * a.sin() * gains[0],
                            -2.0 * length * a.cos() * gains[1],
                        ],
                        taps,
                    );
                    for (actual, expected) in out.pixels().iter().zip(expected) {
                        assert_working_pixel(*actual, expected, "Streak taps");
                    }
                    outputs.push(out);
                }
                assert_eq!(outputs[0].pixels(), outputs[1].pixels());
                if length < 0.25 {
                    assert_eq!(outputs[0].pixels(), image.pixels());
                }
            }
        }
    }
}
#[test]
fn streak_geometry_extent_raw_pass_budget_and_cancellation_refuse_without_leaks() {
    for (size, gains) in [
        ([0, 4], [1.0, 1.0]),
        ([8193, 8193], [1.0, 1.0]),
        ([8, 4], [0.0, 1.0]),
        ([8, 4], [-1.0, 1.0]),
        ([8, 4], [f64::NAN, 1.0]),
        ([8, 4], [1.0, f64::INFINITY]),
    ] {
        assert!(EffectGeometry::new(size, gains).is_err());
    }
    assert!(matches!(
        evaluate_effect(
            &project::Animations::default(),
            &streak(1000.0, 90.0),
            time::Tick::ZERO,
            EffectGeometry::new([8, 4], [f64::MAX, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        ),
        Err(Error::Invalid("effect spatial extent"))
    ));
    let hooks = Hooks::default();
    let image = Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap();
    let tiny = Hooks {
        limit: 0,
        ..Default::default()
    };
    for v in [
        [1.0, 2.0, 1.0],
        [1.0, 2.0, 2.5],
        [1.0, 2.0, 65.0],
        [f32::INFINITY, 0.0, 2.0],
        [0.0, f32::NAN, 2.0],
    ] {
        let pass = EffectPass::Sample {
            entry_point: "streak_blur",
            uniforms: v.map(Uniform::Float).to_vec(),
        };
        assert!(matches!(
            apply_effect(
                Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap(),
                &[pass],
                &image,
                &CpuEffects,
                &tiny
            ),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(tiny.requested.load(Ordering::SeqCst), 0);
    }
    let passes = evaluate_effect(
        &project::Animations::default(),
        &streak(4.0, 90.0),
        time::Tick::ZERO,
        EffectGeometry::new([8, 4], [1.0, 1.0]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    assert!(matches!(
        apply_effect(
            Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap(),
            &passes,
            &image,
            &CpuEffects,
            &tiny
        ),
        Err(Error::Budget { .. })
    ));
    hooks.state.store(1, Ordering::SeqCst);
    assert!(matches!(
        CpuEffects.apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                below: &image,
                coverage: None
            },
            &hooks
        ),
        Err(Error::Cancelled)
    ));
    hooks.state.store(0, Ordering::SeqCst);
    drop(image);
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}

struct SpatialSource {
    size: [u32; 2],
    calls: usize,
    missing: bool,
    last: Option<time::Tick>,
}
impl SourceProvider for SpatialSource {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(self.missing.then_some(UnavailableReason::Missing))
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        at: time::Tick,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        self.last = Some(at);
        self.calls += 1;
        let [w, hg] = self.size;
        Image::from_working_pixels(
            w,
            hg,
            (0..w * hg).map(|i| {
                let a = if i % w < w / 2 { 0.5 } else { 1.0 };
                [
                    if i % w < w / 2 { a } else { 0.0 },
                    if i / w < hg / 2 { a * 0.5 } else { 0.0 },
                    0.0,
                    a,
                ]
            }),
            h,
        )
        .map(Some)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        h: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, h)
    }
}
#[test]
fn streak_scene_uses_actual_unreduced_clip_crop_fit_and_generated_canvas_units() {
    use project::*;
    // Probe is4x4; actual decoded raster400x200 must win, and only one decode occurs.
    for (size, fit, crop, destination, gains, taps) in [
        (
            [300, 300],
            Fit::Cover,
            Crop::default(),
            Rect::default(),
            [1.5, 1.5],
            6,
        ),
        (
            [300, 300],
            Fit::Contain,
            Crop::default(),
            Rect::default(),
            [0.75, 0.75],
            3,
        ),
        (
            [300, 300],
            Fit::Stretch,
            Crop::default(),
            Rect::default(),
            [0.75, 1.5],
            5,
        ),
        (
            [300, 300],
            Fit::Stretch,
            Crop {
                x: 0.25,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            Rect::default(),
            [1.5, 1.5],
            6,
        ),
        (
            [37, 61],
            Fit::Stretch,
            Crop::default(),
            Rect {
                x: 0.1,
                y: 0.2,
                width: 0.8,
                height: 0.6,
            },
            [37.0 * 0.8 / 400.0, 61.0 * 0.6 / 200.0],
            2,
        ),
    ] {
        let mut data = content_scene(vec![]).into_data();
        data.canvas.width = 300;
        data.canvas.height = 300;
        data.canvas.background = Color([0; 4]);
        let plain = Project::new(data.clone()).unwrap();
        data.scene_mut().timeline.main.items[0].effects = vec![streak(2.0, 45.0)];
        let effected = Project::new(data).unwrap();
        let mut settings = RenderSettings::from_project(&plain);
        settings.width = size[0];
        settings.height = size[1];
        settings.placements.insert(
            ItemId(1),
            render::Placement {
                crop,
                destination,
                fit,
                ..Default::default()
            },
        );
        let mut src = SpatialSource {
            size: [400, 200],
            calls: 0,
            missing: false,
            last: None,
        };
        let before = Renderer::new(&plain, settings.clone())
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut src, &Hooks::default())
            .unwrap();
        let input: Vec<_> = before.pixels().iter().map(|p| p.map(f64::from)).collect();
        let displacement = [
            2.0_f64.sqrt() * 2.0 * gains[0],
            -2.0_f64.sqrt() * 2.0 * gains[1],
        ];
        let expected = streak_expected(&input, size, displacement, taps);
        for preview in [false, true] {
            src.calls = 0;
            let mut renderer = Renderer::new(&effected, settings.clone()).unwrap();
            let out = if preview {
                renderer.preview_frame(time::Tick::ZERO, &mut src, &Hooks::default())
            } else {
                renderer.render_frame(time::Tick::ZERO, &mut src, &Hooks::default())
            }
            .unwrap();
            assert_eq!(src.calls, 1);
            if matches!(fit, Fit::Cover) {
                // At (146,0), five taps have red .5; the last is at
                // x=146+3*sqrt(2)/2. Placed texels 148 and 149 have red
                // .5 and 5/12, so red=.5-(3*sqrt(2)/2-2)/72 for six taps.
                // Seven taps would give .4985557101957185, outside 3e-6.
                // Alpha is 1-red and green is alpha/2 in this source region.
                assert_working_pixel(
                    out.pixels()[146],
                    [
                        0.4983149952283383,
                        0.2508425023858309,
                        0.0,
                        0.5016850047716617,
                    ],
                    "Streak analytical six-tap Cover edge",
                );
            } else if matches!(fit, Fit::Contain) {
                // At (148,100), two taps have red .5; the last interpolates
                // between placed texels 149 (.5) and 150 (0). Thus
                // red=.5-(3*sqrt(2)/4-1)/6 with span exactly 3, not four taps.
                assert_working_pixel(
                    out.pixels()[100 * 300 + 148],
                    [
                        0.4898899713700298,
                        0.2550550143149851,
                        0.0,
                        0.5101100286299702,
                    ],
                    "Streak analytical three-tap Contain edge",
                );
                // At (151,223), the first f32 tap is (149.9393463134765625,
                // 224.0606536865234375), giving q=3975/65536. Its four
                // corner alphas are .5,1,0,0; the other two taps have alpha 1.
                // Thus red=q*(1-q)/6 and alpha=1-q/2+q*q/6, independently
                // of production interpolation and accumulation rounding.
                assert_working_pixel(
                    out.pixels()[223 * 300 + 151],
                    [0.009495802805759013, 0.0, 0.0, 0.9702863016864285],
                    "Streak analytical f32-coordinate Contain corner",
                );
            }
            for (a, b) in out.pixels().iter().zip(&expected) {
                assert_working_pixel(*a, *b, "Streak scene geometry");
            }
        }
        src.missing = true;
        assert!(matches!(
            Renderer::new(&effected, settings.clone())
                .unwrap()
                .render_frame(time::Tick::ZERO, &mut src, &Hooks::default()),
            Err(render::Error::Unavailable(_))
        ));
        let missing = Renderer::new(&effected, settings)
            .unwrap()
            .preview_frame(time::Tick::ZERO, &mut src, &Hooks::default())
            .unwrap();
        assert!(missing.pixels().iter().all(|p| *p == [0.0; 4]));
    }
    let mut generated = crate::support::solid_project(1, 40, 20, [255, 0, 0, 128]).into_data();
    generated.canvas.background = Color([0; 4]);
    generated.scene_mut().timeline.overlays[0].items[0].effects = vec![streak(2.0, 90.0)];
    let generated = Project::new(generated).unwrap();
    let mut settings = RenderSettings::from_project(&generated);
    settings.width = 20;
    settings.height = 10;
    settings.placements.insert(
        ItemId(0),
        render::Placement {
            destination: Rect {
                width: 0.5,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let out = Renderer::new(&generated, settings)
        .unwrap()
        .render_frame(
            time::Tick::ZERO,
            &mut crate::support::Empty,
            &Hooks::default(),
        )
        .unwrap();
    for (i, pixel) in out.pixels().iter().enumerate() {
        let alpha = match i % 20 {
            0..=8 => 128.0 / 255.0,
            9..=10 => 64.0 / 255.0,
            _ => 0.0,
        };
        assert_working_pixel(*pixel, [alpha, 0.0, 0.0, alpha], "generated canvas units");
    }
}

#[test]
fn streak_animated_crop_and_length_keep_real_time_before_later_transform() {
    use project::*;
    let hold = |id, rate| {
        effect_definition("hold_time")
            .unwrap()
            .instance(id)
            .with("rate", EffectValue::Scalar(rate))
            .unwrap()
    };
    let mut data = content_scene(vec![
        hold("first", 2.0),
        hold("second", 3.0),
        streak(0.0, 45.0),
    ])
    .into_data();
    data.canvas.width = 40;
    data.canvas.height = 20;
    data.canvas.background = Color([0; 4]);
    let item = &mut data.scene_mut().timeline.main.items[0];
    if let ItemParams::Video(p) = &mut item.content {
        p.crop = Some(Crop {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        });
    }
    for (path, from, to) in [
        ("crop.x", 0.0, 0.5),
        ("effects.streak.params.length", 0.0, 20.0),
    ] {
        let mut first = Keyframe::new(time::Tick::ZERO, ParamValue::Scalar(from));
        first.segment = Segment::Step;
        item.animations
            .set(
                AnimationPath::new(path).unwrap(),
                Param::Value(
                    Channel::new(
                        ParamValue::Scalar(from),
                        vec![first, Keyframe::new(tick(3, 5), ParamValue::Scalar(to))],
                        Extrapolation::Hold,
                        Extrapolation::Hold,
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
    }
    let effected = Project::new(data.clone()).unwrap();
    data.scene_mut().timeline.main.items[0]
        .effects
        .retain(|e| e.kind != "directional_blur");
    let plain = Project::new(data).unwrap();
    let mut source = SpatialSource {
        size: [400, 200],
        calls: 0,
        missing: false,
        last: None,
    };
    let mut settings = RenderSettings::from_project(&plain);
    settings.placements.insert(
        ItemId(1),
        render::Placement {
            fit: Fit::Stretch,
            ..Default::default()
        },
    );
    for (at, displacement, taps) in [
        (tick(1, 10), [0.0, 0.0], 0),
        (tick(4, 5), [5.6568542495, -2.8284271247], 7),
    ] {
        let input = Renderer::new(&plain, settings.clone())
            .unwrap()
            .render_frame(at, &mut source, &Hooks::default())
            .unwrap();
        let expected = streak_expected(
            &input
                .pixels()
                .iter()
                .map(|p| p.map(f64::from))
                .collect::<Vec<_>>(),
            [40, 20],
            displacement,
            taps,
        );
        let out = Renderer::new(&effected, settings.clone())
            .unwrap()
            .render_frame(at, &mut source, &Hooks::default())
            .unwrap();
        assert_eq!(
            source.last,
            Some(if at == tick(4, 5) {
                tick(1, 3)
            } else {
                time::Tick::ZERO
            })
        );
        for (p, e) in out.pixels().iter().zip(&expected) {
            assert_working_pixel(*p, *e, "Streak real clock");
        }
        let mut transformed = effected.data().clone();
        transformed.scene_mut().timeline.main.items[0]
            .animations
            .set(
                AnimationPath::from(BuiltinPath::TransformPositionX),
                Param::new(ParamValue::Scalar(1.0)).unwrap(),
            )
            .unwrap();
        let transformed = Project::new(transformed).unwrap();
        let shifted = Renderer::new(&transformed, settings.clone())
            .unwrap()
            .render_frame(at, &mut source, &Hooks::default())
            .unwrap();
        for y in 0..20 {
            for x in 0..40 {
                let p = shifted.pixels()[y * 40 + x];
                let e = if x == 0 {
                    [0.0; 4]
                } else {
                    expected[y * 40 + x - 1]
                };
                assert_working_pixel(p, e, "Streak later transform");
            }
        }
    }
}

#[test]
#[ignore = "manual bounded Streak CPU cost at1080p"]
fn measure_streak_1080p() {
    let hooks = Hooks::default();
    let input = Image::from_working_pixels(
        1920,
        1080,
        (0..1920 * 1080).map(|i| [if i % 32 < 16 { 0.5 } else { 0.0 }, 0.0, 0.0, 0.5]),
        &hooks,
    )
    .unwrap();
    for (label, length) in [("identity", 0.0), ("typical", 8.0), ("hard", 1000.0)] {
        let passes = evaluate_effect(
            &project::Animations::default(),
            &streak(length, 45.0),
            time::Tick::ZERO,
            EffectGeometry::new([1920, 1080], [1.0, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        )
        .unwrap();
        let run = || {
            CpuEffects
                .apply(
                    &input,
                    &passes,
                    EffectInputs {
                        input: &input,
                        below: &input,
                        coverage: None,
                    },
                    &hooks,
                )
                .unwrap()
        };
        let start = std::time::Instant::now();
        drop(run());
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..2 {
            drop(run());
        }
        let seconds = start.elapsed().as_secs_f64() / 2.0;
        eprintln!(
            "STREAK {label} 1920x1080 angle45 scale1 cold_s={} warm_mean_s={seconds} repetitions=2 mpix_per_s={} peak_charged_bytes={}",
            cold.as_secs_f64(),
            2.0736 / seconds,
            hooks.peak.load(Ordering::SeqCst)
        );
    }
    #[cfg(windows)]
    {
        let rss = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id()),
            ])
            .output()
            .unwrap();
        eprintln!(
            "STREAK peak_process_working_set_bytes={}",
            String::from_utf8(rss.stdout).unwrap().trim()
        );
    }
    drop(input);
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}

// Independent hand-indexed Sobel/Relief oracles; no production sampler or evaluator
// supplies expected pixels.
fn edge_luma(pixel: [f64; 4]) -> f64 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}
fn colour_luma(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}
fn edge_lines_expected(
    pixels: &[[f64; 4]],
    [w, h]: [u32; 2],
    invert: bool,
    blend: f64,
) -> Vec<[f64; 4]> {
    let at = |x: i64, y: i64| {
        let x = x.clamp(0, i64::from(w) - 1) as u32;
        let y = y.clamp(0, i64::from(h) - 1) as u32;
        pixels[(y * w + x) as usize]
    };
    let l = |x: i64, y: i64| edge_luma(at(x, y));
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let gx = l(x + 1, y - 1) + 2.0 * l(x + 1, y) + l(x + 1, y + 1)
                - l(x - 1, y - 1)
                - 2.0 * l(x - 1, y)
                - l(x - 1, y + 1);
            let gy = l(x - 1, y + 1) + 2.0 * l(x, y + 1) + l(x + 1, y + 1)
                - l(x - 1, y - 1)
                - 2.0 * l(x, y - 1)
                - l(x + 1, y - 1);
            let m0 = gx.hypot(gy).min(1.0);
            let m = if invert { m0 } else { 1.0 - m0 };
            let c = at(x, y);
            let level = m * c[3] * (1.0 - blend);
            [
                level + c[0] * blend,
                level + c[1] * blend,
                level + c[2] * blend,
                c[3],
            ]
        })
        .collect()
}
fn relief_expected(
    pixels: &[[f64; 4]],
    size: [u32; 2],
    (direction, relief, contrast, blend): (f64, f64, f64, f64),
    gains: [f64; 2],
) -> Vec<[f64; 4]> {
    let angle = direction.to_radians();
    let dx = angle.cos() * relief * gains[0].max(0.25);
    let dy = -angle.sin() * relief * gains[1].max(0.25);
    let straight = |p: [f64; 4]| {
        if p[3] <= 1e-6 {
            [0.0; 3]
        } else {
            [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
        }
    };
    (0..size[0] * size[1])
        .map(|i| {
            let pixel = pixels[i as usize];
            let alpha = pixel[3];
            if alpha <= 1e-6 {
                return pixel;
            }
            let (x, y) = ((i % size[0]) as f64, (i / size[0]) as f64);
            let difference = colour_luma(straight(spatial_sample(pixels, size, x + dx, y + dy)))
                - colour_luma(straight(spatial_sample(pixels, size, x - dx, y - dy)));
            let encoded = [0, 1, 2].map(|k| oracle_encode((pixel[k] / alpha).clamp(0.0, 1.0)));
            let relief =
                encoded.map(|v| (v + 2.0 * (contrast / 100.0) * difference).clamp(0.0, 1.0));
            let mixed = [0, 1, 2].map(|k| relief[k] + (encoded[k] - relief[k]) * (blend / 100.0));
            let out = [0, 1, 2].map(|k| oracle_decode(mixed[k]) * alpha);
            [out[0], out[1], out[2], alpha]
        })
        .collect()
}
fn edge_lines(invert: bool, blend: f64) -> project::EffectInstance {
    project::effect_definition("find_edges")
        .unwrap()
        .instance("edges")
        .with("invert", EffectValue::Bool(invert))
        .unwrap()
        .with("blend", EffectValue::Scalar(blend))
        .unwrap()
}
fn colour_relief(
    direction: f64,
    relief: f64,
    contrast: f64,
    blend: f64,
) -> project::EffectInstance {
    project::effect_definition("color_emboss")
        .unwrap()
        .instance("relief")
        .with("direction", EffectValue::Scalar(direction))
        .unwrap()
        .with("relief", EffectValue::Scalar(relief))
        .unwrap()
        .with("contrast", EffectValue::Scalar(contrast))
        .unwrap()
        .with("blend", EffectValue::Scalar(blend))
        .unwrap()
}
fn spatial_output(
    pixels: &[[f64; 4]],
    size: [u32; 2],
    gains: [f64; 2],
    effect: &project::EffectInstance,
    fine: bool,
) -> (Vec<[f32; 4]>, usize) {
    let hooks = Hooks::default();
    let source = || {
        Image::from_working_pixels(
            size[0],
            size[1],
            pixels.iter().copied().map(|p| p.map(|v| v as f32)),
            &hooks,
        )
        .unwrap()
    };
    let image = source();
    let passes = evaluate_effect(
        &project::Animations::default(),
        effect,
        time::Tick::ZERO,
        EffectGeometry::new(size, gains).unwrap(),
        fine,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    for pass in &passes {
        validate_pass(pass, size[0], size[1]).unwrap();
    }
    let out = apply_effect(source(), &passes, &image, &CpuEffects, &hooks).unwrap();
    (out.pixels().to_vec(), passes.len())
}

#[test]
fn edge_lines_matches_independent_sobel_blend_and_invert_on_clamped_borders() {
    for size in [[5, 5], [1, 1], [1, 7], [7, 1], [8, 5]] {
        for pattern in 0..6 {
            let n = (size[0] * size[1]) as usize;
            let pixels: Vec<[f64; 4]> = (0..n)
                .map(|i| match pattern {
                    0 => [0.4, 0.4, 0.4, 1.0],
                    1 => {
                        let v = if (i as u32 % size[0]) < size[0] / 2 {
                            0.0
                        } else {
                            1.0
                        };
                        [v, v, v, 1.0]
                    }
                    2 if i == n / 2 => [1.0, 0.0, 0.0, 1.0],
                    2 => [0.0; 4],
                    3 if i % 2 == 0 => [0.75, 0.25, 0.1, 1.0],
                    3 => [0.1, 0.4, 0.7, 1.0],
                    4 => {
                        let a = [0.25, 0.5, 1.0][i % 3];
                        [0.9 * a, 0.4 * a, 0.2 * a, a]
                    }
                    _ if i == n / 2 => [0.0; 4],
                    _ => [0.2, 0.4, 0.6, 1.0],
                })
                .collect();
            for invert in [false, true] {
                for blend in [0.0, 50.0, 100.0] {
                    let (actual, passes) =
                        spatial_output(&pixels, size, [1.0, 1.0], &edge_lines(invert, blend), true);
                    assert_eq!(
                        passes,
                        usize::from(blend < 100.0),
                        "Edge Lines pass count {size:?} {pattern} {invert} {blend}"
                    );
                    let expected = edge_lines_expected(&pixels, size, invert, blend / 100.0);
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_working_pixel(*actual, expected, "Edge Lines");
                    }
                    if blend == 100.0 {
                        let input: Vec<[f32; 4]> =
                            pixels.iter().map(|p| p.map(|v| v as f32)).collect();
                        assert_eq!(actual, input, "Edge Lines blend100 exact identity");
                    }
                }
            }
            if size != [1, 1] {
                let (inverted, _) =
                    spatial_output(&pixels, size, [1.0, 1.0], &edge_lines(true, 0.0), true);
                let (plain, _) =
                    spatial_output(&pixels, size, [1.0, 1.0], &edge_lines(false, 0.0), true);
                assert_ne!(
                    inverted, plain,
                    "invert changes the picture {size:?} {pattern}"
                );
            }
        }
    }
}

#[test]
fn colour_relief_matches_independent_straight_luma_transfer_and_axis_gains() {
    type Row = (f64, f64, f64, f64, [f64; 2], bool);
    let rows: &[Row] = &[
        (45.0, 1.0, 100.0, 0.0, [1.0, 1.0], false),
        (0.0, 1.0, 100.0, 0.0, [1.0, 1.0], false),
        (90.0, 1.0, 100.0, 0.0, [1.0, 1.0], false),
        (-90.0, 1.0, 100.0, 0.0, [1.0, 1.0], false),
        (360.0, 1.0, 100.0, 0.0, [1.0, 1.0], false),
        (-360.0, 1.0, 100.0, 0.0, [1.0, 1.0], false),
        (45.0, 0.0, 100.0, 0.0, [1.0, 1.0], true),
        (45.0, 1.0, 0.0, 0.0, [1.0, 1.0], true),
        (45.0, 1.0, 100.0, 100.0, [1.0, 1.0], true),
        (45.0, 0.5, 100.0, 0.0, [1.0, 1.0], false),
        (45.0, 10.0, 100.0, 0.0, [1.0, 1.0], false),
        (45.0, 1.0, 500.0, 0.0, [1.0, 1.0], false),
        (45.0, 1.0, 100.0, 50.0, [1.0, 1.0], false),
        (45.0, 1.0, 100.0, 0.0, [0.5, 0.5], false),
        (45.0, 1.0, 100.0, 0.0, [0.25, 0.25], false),
        (45.0, 1.0, 100.0, 0.0, [0.1, 1.0], false),
        (45.0, 1.0, 100.0, 0.0, [0.75, 1.5], false),
    ];
    for size in [[5, 5], [8, 5], [1, 1], [1, 7], [7, 1]] {
        let n = (size[0] * size[1]) as usize;
        for pattern in 0..3 {
            let pixels: Vec<[f64; 4]> = (0..n)
                .map(|i| match pattern {
                    0 if i % 2 == 0 => [0.75, 0.25, 0.1, 1.0],
                    0 => [0.1, 0.4, 0.7, 1.0],
                    1 => {
                        let a = [0.25, 0.5, 1.0][i % 3];
                        [0.8 * a, 0.3 * a, 0.6 * a, a]
                    }
                    _ if i == n / 2 => [1.0, 0.5, 0.0, 1.0],
                    _ => [0.0; 4],
                })
                .collect();
            for &(direction, relief, contrast, blend, gains, identity) in rows {
                let effect = colour_relief(direction, relief, contrast, blend);
                let (actual, passes) = spatial_output(&pixels, size, gains, &effect, true);
                assert_eq!(
                    passes,
                    usize::from(!identity),
                    "Relief identity row {direction} {relief} {contrast} {blend} {gains:?}"
                );
                let expected =
                    relief_expected(&pixels, size, (direction, relief, contrast, blend), gains);
                for (actual, expected) in actual.iter().zip(expected) {
                    assert_working_pixel(*actual, expected, "Colour Relief");
                }
                if identity {
                    let input: Vec<[f32; 4]> = pixels.iter().map(|p| p.map(|v| v as f32)).collect();
                    assert_eq!(actual, input, "Relief identity exact");
                }
                let (coarse, _) = spatial_output(&pixels, size, gains, &effect, false);
                assert_eq!(actual, coarse, "Relief fine/coarse share one effort");
            }
        }
    }
    // The added luma difference shifts every encoded channel, so a red step also
    // raises green/blue by decode(2 * 0.2126); equal straight luma leaves the centre.
    let shifted = oracle_decode(2.0 * 0.2126);
    for (pixels, expected_center) in [
        (
            vec![
                [0.5, 0.0, 0.0, 0.5],
                [0.5, 0.0, 0.0, 1.0],
                [0.25, 0.0, 0.0, 0.25],
            ],
            [0.5, 0.0, 0.0, 1.0],
        ),
        (
            vec![
                [0.0, 0.0, 0.0, 1.0],
                [0.5, 0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
            ],
            [1.0, shifted, shifted, 1.0],
        ),
        (
            vec![[1e-6, 0.0, 0.0, 1e-6], [0.5, 0.0, 0.0, 1.0], [0.0; 4]],
            [0.5, 0.0, 0.0, 1.0],
        ),
    ] {
        let effect = colour_relief(0.0, 1.0, 100.0, 0.0);
        let (actual, _) = spatial_output(&pixels, [3, 1], [1.0, 1.0], &effect, true);
        assert_working_pixel(actual[1], expected_center, "Relief straight luma centre");
    }
}

// Independent f64 gaussian and unsharp combine: the documented sigma, iteration,
// radius and weight formulas only. No production kernel, sampler or evaluator is
// used to build expected pixels.
fn convolve_axis(
    pixels: &[[f64; 4]],
    [w, h]: [u32; 2],
    weights: &[f64],
    horizontal: bool,
) -> Vec<[f64; 4]> {
    let radius = weights.len() as i64 / 2;
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let mut sum = [0.0; 4];
            for (tap, weight) in weights.iter().enumerate() {
                let delta = tap as i64 - radius;
                let sx = if horizontal {
                    (x as i64 + delta).clamp(0, i64::from(w) - 1) as u32
                } else {
                    x
                };
                let sy = if horizontal {
                    y
                } else {
                    (y as i64 + delta).clamp(0, i64::from(h) - 1) as u32
                };
                let source = pixels[(sy * w + sx) as usize];
                for (v, s) in sum.iter_mut().zip(source) {
                    *v += s * weight;
                }
            }
            let alpha = sum[3].clamp(0.0, 1.0);
            let rgb = [0, 1, 2].map(|k| sum[k].clamp(0.0, alpha));
            [rgb[0], rgb[1], rgb[2], alpha]
        })
        .collect()
}
fn sharpen_expected(
    pixels: &[[f64; 4]],
    size: [u32; 2],
    gains: [f64; 2],
    amount: f64,
) -> Vec<[f64; 4]> {
    let sigma = f64::from(gains[0].min(gains[1]).max(0.6) as f32);
    let iterations = ((sigma / 8.0).powi(2).ceil().max(1.0)) as usize;
    let sigma = sigma / (iterations as f64).sqrt();
    let radius = (sigma * 3.0).ceil().clamp(1.0, 24.0) as i64;
    let mut weights: Vec<f64> = (-radius..=radius)
        .map(|x| (-(x as f64).powi(2) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= total;
    }
    let mut current = pixels.to_vec();
    for _ in 0..iterations {
        current = convolve_axis(&current, size, &weights, true);
        current = convolve_axis(&current, size, &weights, false);
    }
    let gain = amount / 100.0;
    current
        .iter()
        .zip(pixels)
        .map(|(blurred, original)| {
            let alpha = original[3];
            let rgb = [0, 1, 2]
                .map(|k| (original[k] + gain * (original[k] - blurred[k])).clamp(0.0, alpha));
            [rgb[0], rgb[1], rgb[2], alpha]
        })
        .collect()
}
fn sharpen(amount: f64) -> project::EffectInstance {
    project::effect_definition("sharpen")
        .unwrap()
        .instance("sharpen")
        .with("amount", EffectValue::Scalar(amount))
        .unwrap()
}
#[test]
fn sharpen_matches_independent_gaussian_unsharp_rows() {
    for size in [[5, 5], [8, 5], [1, 1], [1, 7], [7, 1]] {
        let n = (size[0] * size[1]) as usize;
        for pattern in 0..3 {
            let pixels: Vec<[f64; 4]> = (0..n)
                .map(|i| match pattern {
                    // A hard step: the sharpening peak sits on the border texels.
                    0 => {
                        let v = if (i as u32 % size[0]) < size[0] / 2 {
                            0.25
                        } else {
                            0.75
                        };
                        [v, v * 0.5, v * 0.25, 1.0]
                    }
                    // Unequal alpha, so the coverage stays per pixel.
                    1 => {
                        let a = [0.25, 0.5, 1.0][i % 3];
                        [0.6 * a, 0.2 * a, 0.1 * a, a]
                    }
                    // An impulse over a flat field.
                    _ if (i as u32 % size[0]) == size[0] / 2 => [1.0, 0.5, 0.0, 1.0],
                    _ => [0.2, 0.2, 0.2, 1.0],
                })
                .collect();
            for gains in [[1.0, 1.0], [0.5, 0.5], [0.5, 1.5]] {
                for amount in [0.0, 50.0, 100.0, 250.0] {
                    let effect = sharpen(amount);
                    let (actual, passes) = spatial_output(&pixels, size, gains, &effect, true);
                    assert_eq!(
                        passes,
                        if amount == 0.0 { 0 } else { 3 },
                        "Sharpen pass count {size:?} {pattern} {gains:?} {amount}"
                    );
                    let input: Vec<[f32; 4]> = pixels.iter().map(|p| p.map(|v| v as f32)).collect();
                    if amount == 0.0 {
                        assert_eq!(actual, input, "Sharpen amount0 exact identity");
                        continue;
                    }
                    let expected = sharpen_expected(&pixels, size, gains, amount);
                    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                        assert_working_pixel(
                            *actual,
                            expected,
                            &format!("Sharpen {size:?} {pattern} {gains:?} {amount} #{index}"),
                        );
                        assert_eq!(
                            actual[3], pixels[index][3] as f32,
                            "Sharpen preserves coverage {size:?} {pattern} {gains:?} {amount}"
                        );
                    }
                    let (coarse, _) = spatial_output(&pixels, size, gains, &effect, false);
                    assert_eq!(actual, coarse, "Sharpen fine/coarse share one effort");
                }
            }
        }
    }
    // The hard gain ceiling must stay finite and inside the premultiplied
    // contract even on fully opaque white beside transparent black.
    let pixels = vec![
        [1.0, 1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0, 0.0],
        [0.5, 0.25, 0.125, 1.0],
    ];
    let (hard, passes) = spatial_output(&pixels, [3, 1], [1.0, 1.0], &sharpen(4000.0), true);
    assert_eq!(passes, 3);
    for pixel in &hard {
        assert!(pixel.iter().all(|v| v.is_finite()));
        assert!((0.0..=1.0).contains(&pixel[3]));
        assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
    }
    assert_eq!(hard[0][3], 1.0);
    assert_eq!(hard[1][3], 0.0);
}
#[test]
fn sharpen_refuses_hostile_values_and_malformed_detail_passes() {
    for amount in [-1.0, 4000.1, f64::NAN, f64::INFINITY] {
        let definition = project::effect_definition("sharpen").unwrap();
        assert!(
            definition
                .instance("sharpen")
                .with("amount", EffectValue::Scalar(amount))
                .is_err(),
            "Sharpen refuses {amount}"
        );
    }
    assert!(matches!(
        evaluate_effect(
            &project::Animations::default(),
            &sharpen(4000.0),
            time::Tick::ZERO,
            EffectGeometry::new([8, 4], [1e9, 1e9]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        ),
        Err(Error::Unsupported("blur iteration bound"))
    ));
    let hooks = Hooks::default();
    let image = Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap();
    let tiny = Hooks {
        limit: 0,
        ..Default::default()
    };
    for values in [
        vec![1.0],
        vec![1.0, 0.0, 0.0],
        vec![40.1, 0.0],
        vec![1.0, 1.1],
        vec![f32::NAN, 0.0],
        vec![1.0, f32::INFINITY],
    ] {
        let pass = EffectPass::Sample {
            entry_point: "detail_boost",
            uniforms: values.iter().copied().map(Uniform::Float).collect(),
        };
        assert!(matches!(
            apply_effect(
                Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap(),
                &[pass],
                &image,
                &CpuEffects,
                &tiny
            ),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(tiny.requested.load(Ordering::SeqCst), 0);
    }
}

fn pixelate_expected(
    pixels: &[[f64; 4]],
    size: [u32; 2],
    horizontal: f64,
    vertical: f64,
    sharp: bool,
    softness: f64,
) -> Vec<[f64; 4]> {
    let [w, h] = size;
    // The pass boundary carries f32 uniforms and indices; round every coordinate
    // and fraction explicitly, as the streak oracle does.
    let bw = (w as f32 / horizontal.trunc().max(1.0) as f32).max(1.0);
    let bh = (h as f32 / vertical.trunc().max(1.0) as f32).max(1.0);
    let tiles_x = (w as f32 / bw).ceil().max(1.0);
    let tiles_y = (h as f32 / bh).ceil().max(1.0);
    let centre = |i: f32, j: f32| {
        let px = ((i + 0.5) * bw - 0.5).clamp(0.0, (w - 1) as f32);
        let py = ((j + 0.5) * bh - 0.5).clamp(0.0, (h - 1) as f32);
        spatial_sample(pixels, size, f64::from(px), f64::from(py))
    };
    let mix = |a: [f64; 4], b: [f64; 4], t: f32| {
        [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * f64::from(t))
    };
    let soft = if sharp {
        0.0
    } else {
        (softness / 100.0) as f32
    };
    (0..w * h)
        .map(|index| {
            let x = (index % w) as f32;
            let y = (index / w) as f32;
            let nearest = centre(
                (x / bw).floor().clamp(0.0, tiles_x - 1.0),
                (y / bh).floor().clamp(0.0, tiles_y - 1.0),
            );
            if soft <= 0.0 {
                return nearest;
            }
            // Source lattice coordinate: pixel centres at half offsets, then one
            // half-tile shift into tile-centre space.
            let raw_x = (x + 0.5) / bw - 0.5;
            let raw_y = (y + 0.5) / bh - 0.5;
            let ix = raw_x.floor();
            let iy = raw_y.floor();
            let (fx, fy) = (raw_x - ix, raw_y - iy);
            let tile_at =
                |i: f32, j: f32| centre(i.clamp(0.0, tiles_x - 1.0), j.clamp(0.0, tiles_y - 1.0));
            let top = mix(tile_at(ix, iy), tile_at(ix + 1.0, iy), fx);
            let bottom = mix(tile_at(ix, iy + 1.0), tile_at(ix + 1.0, iy + 1.0), fx);
            mix(nearest, mix(top, bottom, fy), soft)
        })
        .collect()
}
fn pixelate(horizontal: f64, vertical: f64, sharp: bool, softness: f64) -> project::EffectInstance {
    project::effect_definition("mosaic")
        .unwrap()
        .instance("pixelate")
        .with("horizontal", EffectValue::Scalar(horizontal))
        .unwrap()
        .with("vertical", EffectValue::Scalar(vertical))
        .unwrap()
        .with("sharp", EffectValue::Bool(sharp))
        .unwrap()
        .with("softness", EffectValue::Scalar(softness))
        .unwrap()
}
#[test]
fn pixelate_matches_independent_tile_lattice_rows() {
    let counts = [
        (1.0, 1.0),
        (2.0, 2.0),
        (3.0, 2.0),
        (3.7, 2.2),
        (10.0, 10.0),
        (4000.0, 4000.0),
    ];
    for size in [[8, 6], [5, 5], [1, 1], [1, 7], [7, 1], [9, 4]] {
        let n = (size[0] * size[1]) as usize;
        for pattern in 0..3 {
            let pixels: Vec<[f64; 4]> = (0..n)
                .map(|i| match pattern {
                    0 => {
                        let c = (i % 4) as f64 / 3.0;
                        let a = [0.25, 0.5, 1.0][i % 3];
                        [c * a, (1.0 - c) * a, 0.2 * a, a]
                    }
                    1 => {
                        let v = if (i as u32 % size[0]) < size[0] / 2 {
                            1.0
                        } else {
                            0.0
                        };
                        [v, 0.25 * v, 0.0, v]
                    }
                    _ if i == n / 2 => [1.0, 0.0, 0.0, 1.0],
                    _ => [0.0, 0.0, 0.0, 0.0],
                })
                .collect();
            for (horizontal, vertical) in counts {
                for (sharp, softness) in [(false, 0.0), (false, 35.0), (false, 100.0), (true, 60.0)]
                {
                    let effect = pixelate(horizontal, vertical, sharp, softness);
                    let (actual, passes) = spatial_output(&pixels, size, [1.0, 1.0], &effect, true);
                    // Both axes at one pixel per block are an exact identity for
                    // every softness value, so the evaluator emits no pass.
                    let identity =
                        horizontal >= f64::from(size[0]) && vertical >= f64::from(size[1]);
                    assert_eq!(
                        passes,
                        usize::from(!identity),
                        "Pixelate pass count {size:?} {pattern} {horizontal} {vertical} {sharp} {softness}"
                    );
                    let input: Vec<[f32; 4]> = pixels.iter().map(|p| p.map(|v| v as f32)).collect();
                    if identity {
                        assert_eq!(actual, input, "Pixelate identity exact");
                        continue;
                    }
                    let expected =
                        pixelate_expected(&pixels, size, horizontal, vertical, sharp, softness);
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_working_pixel(
                            *actual,
                            expected,
                            &format!(
                                "Pixelate {size:?} {pattern} {horizontal} {vertical} {sharp} {softness}"
                            ),
                        );
                    }
                    let (coarse, _) = spatial_output(&pixels, size, [1.0, 1.0], &effect, false);
                    assert_eq!(actual, coarse, "Pixelate fine/coarse share one effort");
                }
            }
        }
    }
}
#[test]
fn pixelate_parameters_change_the_picture_and_refuse_hostile_values() {
    let size = [8, 8];
    // A ramp along both axes, so horizontal and vertical block counts each show.
    let pixels: Vec<[f64; 4]> = (0..64)
        .map(|i| {
            let v = 0.1 + 0.05 * f64::from(i % 8) + 0.05 * f64::from(i / 8);
            [v, 0.5 * v, 0.0, 1.0]
        })
        .collect();
    for (label, left_effect, right_effect) in [
        (
            "horizontal",
            pixelate(2.0, 8.0, false, 0.0),
            pixelate(3.0, 8.0, false, 0.0),
        ),
        (
            "vertical",
            pixelate(8.0, 2.0, false, 0.0),
            pixelate(8.0, 3.0, false, 0.0),
        ),
        (
            "softness",
            pixelate(2.0, 2.0, false, 0.0),
            pixelate(2.0, 2.0, false, 55.0),
        ),
        (
            "sharp",
            pixelate(2.0, 2.0, false, 55.0),
            pixelate(2.0, 2.0, true, 55.0),
        ),
    ] {
        let (left, _) = spatial_output(&pixels, size, [1.0, 1.0], &left_effect, true);
        let (right, _) = spatial_output(&pixels, size, [1.0, 1.0], &right_effect, true);
        assert_ne!(left, right, "Pixelate {label} must change the picture");
    }
    let definition = project::effect_definition("mosaic").unwrap();
    for (id, value) in [
        ("horizontal", EffectValue::Scalar(0.5)),
        ("horizontal", EffectValue::Scalar(4000.1)),
        ("vertical", EffectValue::Scalar(f64::NAN)),
        ("softness", EffectValue::Scalar(-1.0)),
        ("softness", EffectValue::Scalar(101.0)),
        ("sharp", EffectValue::Scalar(1.0)),
    ] {
        assert!(
            definition.instance("pixelate").with(id, value).is_err(),
            "Pixelate refuses {id} {value:?}"
        );
    }
    let hooks = Hooks::default();
    let image = Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap();
    let tiny = Hooks {
        limit: 0,
        ..Default::default()
    };
    for values in [
        vec![2.0, 2.0],
        vec![2.0, 2.0, 0.5, 0.0],
        vec![0.5, 2.0, 0.0],
        vec![f32::NAN, 2.0, 0.0],
        vec![2.0, f32::INFINITY, 0.0],
        vec![2.0, 2.0, 1.1],
    ] {
        let pass = EffectPass::Sample {
            entry_point: "pixelate",
            uniforms: values.iter().copied().map(Uniform::Float).collect(),
        };
        assert!(matches!(
            apply_effect(
                Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap(),
                &[pass],
                &image,
                &CpuEffects,
                &tiny
            ),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(tiny.requested.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn edge_relief_raw_extent_budget_cancel_and_drop_refuse_without_leaks() {
    let hooks = Hooks::default();
    let image = Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap();
    let tiny = Hooks {
        limit: 0,
        ..Default::default()
    };
    let malformed: &[(&str, &[f32])] = &[
        ("edge_lines", &[0.0]),
        ("edge_lines", &[0.0, 1.5]),
        ("edge_lines", &[f32::NAN, 0.0]),
        ("edge_lines", &[2.0, 0.0]),
        ("colour_relief", &[0.0, 0.0, 0.5]),
        ("colour_relief", &[0.0, 0.0, 5.5, 0.0]),
        ("colour_relief", &[f32::INFINITY, 0.0, 1.0, 0.0]),
        ("colour_relief", &[0.0, 0.0, 1.0, 1.5]),
    ];
    for (entry_point, values) in malformed {
        let pass = EffectPass::Sample {
            entry_point,
            uniforms: values.iter().copied().map(Uniform::Float).collect(),
        };
        assert!(matches!(
            apply_effect(
                Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap(),
                &[pass],
                &image,
                &CpuEffects,
                &tiny
            ),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(tiny.requested.load(Ordering::SeqCst), 0, "{entry_point}");
    }
    assert!(matches!(
        evaluate_effect(
            &project::Animations::default(),
            &colour_relief(0.0, 10.0, 100.0, 0.0),
            time::Tick::ZERO,
            EffectGeometry::new([8, 4], [f64::MAX, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        ),
        Err(Error::Invalid("effect spatial extent"))
    ));
    let passes = evaluate_effect(
        &project::Animations::default(),
        &colour_relief(45.0, 10.0, 500.0, 0.0),
        time::Tick::ZERO,
        EffectGeometry::new([8, 4], [0.75, 1.5]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    assert!(matches!(
        apply_effect(
            Image::new(8, 4, [0.125, 0.0625, 0.0, 0.25], &hooks).unwrap(),
            &passes,
            &image,
            &CpuEffects,
            &tiny
        ),
        Err(Error::Budget { .. })
    ));
    hooks.state.store(1, Ordering::SeqCst);
    assert!(matches!(
        CpuEffects.apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                below: &image,
                coverage: None
            },
            &hooks
        ),
        Err(Error::Cancelled)
    ));
    hooks.state.store(0, Ordering::SeqCst);
    drop(image);
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}

#[test]
#[ignore = "manual bounded Edge Lines/Colour Relief CPU cost at1080p"]
fn measure_edge_relief_1080p() {
    let hooks = Hooks::default();
    let input = Image::from_working_pixels(
        1920,
        1080,
        (0..1920 * 1080).map(|i| [if i % 32 < 16 { 0.5 } else { 0.0 }, 0.0, 0.0, 0.5]),
        &hooks,
    )
    .unwrap();
    for (label, effect) in [
        ("EdgeLines-identity", edge_lines(false, 100.0)),
        ("EdgeLines-typical", edge_lines(false, 0.0)),
        ("EdgeLines-hard", edge_lines(true, 0.0)),
        ("Relief-identity", colour_relief(45.0, 0.0, 100.0, 0.0)),
        ("Relief-typical", colour_relief(45.0, 1.0, 100.0, 0.0)),
        ("Relief-hard", colour_relief(45.0, 10.0, 500.0, 0.0)),
    ] {
        let passes = evaluate_effect(
            &project::Animations::default(),
            &effect,
            time::Tick::ZERO,
            EffectGeometry::new([1920, 1080], [1.0, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        )
        .unwrap();
        let run = || {
            CpuEffects
                .apply(
                    &input,
                    &passes,
                    EffectInputs {
                        input: &input,
                        below: &input,
                        coverage: None,
                    },
                    &hooks,
                )
                .unwrap()
        };
        let start = std::time::Instant::now();
        drop(run());
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..2 {
            drop(run());
        }
        let seconds = start.elapsed().as_secs_f64() / 2.0;
        eprintln!(
            "EDGE_RELIEF {label} 1920x1080 gains1 cold_s={} warm_mean_s={seconds} repetitions=2 mpix_per_s={} peak_charged_bytes={}",
            cold.as_secs_f64(),
            2.0736 / seconds,
            hooks.peak.load(Ordering::SeqCst)
        );
    }
    #[cfg(windows)]
    {
        let rss = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id()),
            ])
            .output()
            .unwrap();
        eprintln!(
            "EDGE_RELIEF peak_process_working_set_bytes={}",
            String::from_utf8(rss.stdout).unwrap().trim()
        );
    }
    drop(input);
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}

#[test]
fn streak_cancels_inside_sample_rows_and_releases_partial_images() {
    use std::sync::atomic::AtomicUsize;
    struct Interrupt {
        hooks: Hooks,
        checks: AtomicUsize,
    }
    impl ExecutionHooks for Interrupt {
        fn cancelled(&self) -> bool {
            self.checks.fetch_add(1, Ordering::SeqCst) > 12
        }
        fn reserve(&self, k: MemoryKind, n: u64) -> Result<Box<dyn Reservation>> {
            self.hooks.reserve(k, n)
        }
    }
    let base = Hooks::default();
    let image = Image::new(8, 8, [0.25; 4], &base).unwrap();
    let interrupt = Interrupt {
        hooks: base.clone(),
        checks: AtomicUsize::new(0),
    };
    let passes = evaluate_effect(
        &project::Animations::default(),
        &streak(20.0, 45.0),
        time::Tick::ZERO,
        EffectGeometry::new([8, 8], [1.0, 1.0]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    assert!(matches!(
        CpuEffects.apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                below: &image,
                coverage: None
            },
            &interrupt
        ),
        Err(Error::Cancelled)
    ));
    assert!(
        interrupt.checks.load(Ordering::SeqCst) > 12,
        "copies finish before row cancellation"
    );
    assert_eq!(
        base.used.load(Ordering::SeqCst),
        Image::bytes(8, 8).unwrap()
    );
    drop(image);
    assert_eq!(base.used.load(Ordering::SeqCst), 0);
}

// ------------------------------------------------------------------ 265 whole-image entries

/// The 265 entries with a frame evaluator. `posterize_time` is a content-time
/// remap and is covered by its own test below.
const WHOLE_EFFECTS: [&str; 25] = [
    "twirl",
    "spherize",
    "magnify",
    "wave_warp",
    "lens_distortion",
    "turbulent_displace",
    "corner_pin",
    "basic_3d",
    "echo_glow",
    "edge_glow",
    "glint",
    "wonder_glow",
    "volumetric_rays",
    "light_leaks",
    "lens_flare",
    "rgb_split",
    "brush_strokes",
    "roughen_edges",
    "camera_shake",
    "drop_shadow",
    "long_shadow",
    "gradient",
    "four_color_gradient",
    "color_key",
    "luma_key",
];

/// A 24x18 fixture with spatial structure, colour variation and transparent
/// and translucent regions, so warps, glows and keys all see real neighbours.
fn whole_chart() -> Vec<[f32; 4]> {
    let (w, h) = (24u32, 18u32);
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let alpha = if x < 8 || y < 4 {
                1.0
            } else if (x + y) % 5 == 0 {
                0.6
            } else {
                0.9
            };
            let r = x as f32 / (w - 1) as f32;
            let g = y as f32 / (h - 1) as f32;
            let b = ((x * 3 + y * 5) % 7) as f32 / 6.0;
            [r * alpha, g * alpha, b * alpha, alpha]
        })
        .collect()
}

fn whole_output_at(
    kind: &str,
    params: &[(&str, EffectValue)],
    at: time::Tick,
    fine: bool,
    source: &[[f32; 4]],
) -> Vec<[f32; 4]> {
    let hooks = Hooks::default();
    let image = Image::from_working_pixels(24, 18, source.iter().copied(), &hooks).unwrap();
    let instance = effect_instance(kind, params);
    let passes = evaluate_effect(
        &project::Animations::default(),
        &instance,
        at,
        EffectGeometry::new([24, 18], [1.0, 1.0]).unwrap(),
        fine,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    for pass in &passes {
        validate_pass(pass, 24, 18).unwrap();
    }
    CpuEffects
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap()
        .pixels()
        .to_vec()
}

fn whole_output(kind: &str, params: &[(&str, EffectValue)]) -> Vec<[f32; 4]> {
    whole_output_at(kind, params, whole_time(kind), true, &whole_fixture(kind))
}

/// Key fixtures sweep a continuous colour/luma ramp so tolerance, feather and
/// cutoff land inside their bands; every other entry uses the spatial chart.
fn whole_fixture(kind: &str) -> Vec<[f32; 4]> {
    if matches!(kind, "color_key" | "luma_key") {
        (0..432)
            .map(|i| {
                let t = i as f32 / 431.0;
                let encoded = [t, t * 0.8, 1.0 - 0.5 * t];
                [
                    linear(encoded[0]),
                    linear(encoded[1]),
                    linear(encoded[2]),
                    1.0,
                ]
            })
            .collect()
    } else if kind == "roughen_edges" {
        // Torn Edges works on the coverage boundary: cut a transparent corner
        // into the chart so there is an edge for border, scale and complexity.
        whole_chart()
            .into_iter()
            .enumerate()
            .map(|(i, pixel)| {
                let (x, y) = (i % 24, i / 24);
                if x >= 14 && y >= 8 { [0.0; 4] } else { pixel }
            })
            .collect()
    } else {
        whole_chart()
    }
}

/// Entries whose parameters include a clock read the wave/shake phase at a
/// nonzero item-local time so a speed/frequency change is observable.
fn whole_time(kind: &str) -> time::Tick {
    match kind {
        "wave_warp" | "light_leaks" | "camera_shake" => tick(1, 4),
        _ => time::Tick::ZERO,
    }
}

/// A value the parameter does not hold at the tested state. Angles avoid
/// 360-degree aliases and Corner Pin stays a valid quad.
fn whole_changed_value(kind: &str, param: &project::EffectParamDefinition) -> EffectValue {
    use project::{EffectParamKind, EffectValue};
    // Magnifier sizes at or beyond the 24x18 chart (the 100 px default and the
    // 4000 px bound) all cover every pixel; a 7 px lens leaves most of it outside.
    if (kind, param.id) == ("magnify", "size") {
        return EffectValue::Scalar(7.0);
    }
    match param.kind {
        EffectParamKind::Float { min, max, .. } => {
            let current = param.default.scalar().unwrap();
            EffectValue::Scalar(if (current - min).abs() >= (current - max).abs() {
                min
            } else {
                max
            })
        }
        EffectParamKind::Angle => {
            let current = param.default.scalar().unwrap_or(0.0);
            EffectValue::Scalar(if current == 0.0 { 45.0 } else { 0.0 })
        }
        EffectParamKind::Choice { options } => {
            EffectValue::Choice((param.default.choice().unwrap() + 1) % options.len() as u32)
        }
        EffectParamKind::Bool => EffectValue::Bool(!param.default.flag().unwrap()),
        EffectParamKind::Color => {
            EffectValue::Color(if param.default.color() == Some(Color([255, 0, 0, 255])) {
                Color([0, 255, 255, 255])
            } else {
                Color([255, 0, 0, 255])
            })
        }
        // A corner moved to the centre lies on a quad diagonal, which is a
        // singular homography; [0.3, 0.2] is off both diagonals for every corner.
        EffectParamKind::Point => EffectValue::Point(if kind == "corner_pin" {
            [0.3, 0.2]
        } else {
            [1.0, 1.0]
        }),
        EffectParamKind::Lut => EffectValue::Lut(None),
        EffectParamKind::Curve => EffectValue::Curve(
            project::ToneCurve::new(&[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]).unwrap(),
        ),
    }
}

/// State another parameter needs before this one is observable: Lens Warp's
/// decentering scales the radial term, RGB Split's centre only applies in
/// Radial mode and the keys need a tolerance/cutoff band.
fn whole_base_params(
    kind: &str,
    param: &project::EffectParamDefinition,
) -> Vec<(&'static str, EffectValue)> {
    use EffectValue::Scalar as S;
    match (kind, param.id) {
        ("spherize", _) => vec![("radius", S(250.0))],
        ("lens_distortion", _) => vec![("curvature", S(60.0))],
        // The default 100 px lens covers the whole 24x18 chart in either shape;
        // a 7 px lens leaves the square's corners outside the circle.
        ("magnify", "shape") => vec![("size", S(7.0))],
        // Sparkle tints its rays only in proportion to Colorize (default 0).
        ("glint", "color") => vec![("colorize", S(100.0))],
        // Bloom tints with its colour only when "Use glow colour" is on.
        ("wonder_glow", "color") => vec![("use_color", EffectValue::Bool(true))],
        ("rgb_split", "center") => vec![("mode", EffectValue::Choice(1))],
        ("rgb_split", "angle") => vec![("mode", EffectValue::Choice(0))],
        ("color_key", _) => vec![("tolerance", S(80.0))],
        ("luma_key", _) => vec![("threshold", S(60.0)), ("cutoff", S(20.0))],
        _ => Vec::new(),
    }
}

#[test]
fn whole_image_effects_that_are_identity_at_default_emit_no_pass() {
    for kind in ["lens_distortion", "corner_pin", "basic_3d"] {
        let instance = effect_instance(kind, &[]);
        let passes = evaluate_effect(
            &project::Animations::default(),
            &instance,
            time::Tick::ZERO,
            EffectGeometry::new([24, 18], [1.0, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        )
        .unwrap();
        assert!(passes.is_empty(), "{kind} emits an identity pass");
        assert_eq!(whole_output(kind, &[]), whole_chart(), "{kind} is identity");
    }
}

#[test]
fn every_whole_image_parameter_changes_the_picture() {
    for kind in WHOLE_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for param in definition.params {
            let mut base = whole_base_params(kind, param);
            base.retain(|(id, _)| *id != param.id);
            let changed = whole_changed_value(kind, param);
            base.push((param.id, changed));
            let at = whole_time(kind);
            let source = whole_fixture(kind);
            let before = whole_output_at(kind, &[], at, true, &source);
            let base_before = if base.len() > 1 {
                let defaults: Vec<_> = base
                    .iter()
                    .filter(|(id, _)| *id != param.id)
                    .copied()
                    .collect();
                whole_output_at(kind, &defaults, at, true, &source)
            } else {
                before.clone()
            };
            let after = whole_output_at(kind, &base, at, true, &source);
            assert_ne!(after, base_before, "{kind}.{} changed nothing", param.id);
        }
    }
}

#[test]
fn whole_image_outputs_are_deterministic_for_an_item_local_time() {
    for kind in WHOLE_EFFECTS {
        let at = whole_time(kind);
        let source = whole_fixture(kind);
        let first = whole_output_at(kind, &[], at, true, &source);
        let second = whole_output_at(kind, &[], at, true, &source);
        assert_eq!(first, second, "{kind} is not deterministic");
    }
}

#[test]
fn whole_image_effects_keep_premultiplied_pixels_finite_at_hostile_bounds() {
    for kind in WHOLE_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for high in [false, true] {
            let mut params = Vec::new();
            for param in definition.params {
                let value = match param.kind {
                    project::EffectParamKind::Float { min, max, .. } => {
                        EffectValue::Scalar(if high { max } else { min })
                    }
                    project::EffectParamKind::Angle => {
                        EffectValue::Scalar(if high { ANGLE_MAX } else { -ANGLE_MAX })
                    }
                    project::EffectParamKind::Choice { options } => {
                        EffectValue::Choice(if high { options.len() as u32 - 1 } else { 0 })
                    }
                    project::EffectParamKind::Bool => EffectValue::Bool(high),
                    project::EffectParamKind::Color => EffectValue::Color(if high {
                        Color([255, 255, 255, 255])
                    } else {
                        Color([0, 0, 0, 255])
                    }),
                    project::EffectParamKind::Point => {
                        EffectValue::Point([if high { 1.0 } else { 0.0 }; 2])
                    }
                    project::EffectParamKind::Lut => EffectValue::Lut(None),
                    project::EffectParamKind::Curve => EffectValue::Curve(
                        project::ToneCurve::new(if high {
                            &[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]]
                        } else {
                            &[[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]
                        })
                        .unwrap(),
                    ),
                };
                params.push((param.id, value));
            }
            let output = whole_output(kind, &params);
            for pixel in output {
                assert!(
                    pixel
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                    "{kind} {high} {pixel:?}"
                );
                assert!(
                    pixel[..3].iter().all(|v| *v <= pixel[3] + 1e-6),
                    "{kind} {high} {pixel:?}"
                );
            }
        }
    }
}

#[test]
fn whole_image_animations_clamp_to_their_hard_range() {
    for kind in WHOLE_EFFECTS {
        let definition = project::effect_definition(kind).unwrap();
        for param in definition
            .params
            .iter()
            .filter(|p| p.animatable)
            .filter_map(|p| p.scalar_range().map(|range| (p, range)))
        {
            for (raw, expected) in [(1e9, param.1.1), (-1e9, param.1.0)] {
                let mut animations = project::Animations::default();
                animations
                    .set(
                        project::AnimationPath::new(format!("effects.one.params.{}", param.0.id))
                            .unwrap(),
                        project::Param::new(project::ParamValue::Scalar(raw)).unwrap(),
                    )
                    .unwrap();
                let instance = definition.instance("one");
                let frame =
                    EffectFrame::new(&animations, &instance, time::Tick::ZERO, 24, 18).unwrap();
                assert_eq!(
                    frame.scalar(param.0.id).unwrap(),
                    expected,
                    "{kind}.{}",
                    param.0.id
                );
                for pass in (evaluator(kind).unwrap())(
                    &frame,
                    &EffectGeometry::new([24, 18], [1.0, 1.0]).unwrap(),
                )
                .unwrap()
                {
                    validate_pass(&pass, 24, 18)
                        .unwrap_or_else(|error| panic!("{kind}.{} {error:?}", param.0.id));
                }
            }
        }
    }
}

#[test]
fn low_frame_rate_remaps_content_time_deterministically() {
    let rate = |rate| {
        project::effect_definition("posterize_time")
            .unwrap()
            .instance("low")
            .with("rate", EffectValue::Scalar(rate))
            .unwrap()
    };
    let item = content_scene(vec![rate(4.0)])
        .data()
        .scene()
        .timeline
        .main
        .items[0]
        .clone();
    let at = tick(3, 10);
    let held = content_time(&item, at, 4, 4).unwrap();
    assert_eq!(held, tick(1, 4));
    assert_eq!(content_time(&item, at, 4, 4).unwrap(), held);
    assert!(content_remap("posterize_time").is_some());
    let mut plain = item.clone();
    plain.effects.clear();
    assert_eq!(content_time(&plain, at, 4, 4).unwrap(), at);
}

#[test]
#[ignore = "manual 1080p CPU cost of the 265 whole-image entries"]
fn measure_whole_image_1080p() {
    let hooks = Hooks::default();
    let input = Image::from_working_pixels(
        1920,
        1080,
        (0..1920 * 1080).map(|i| {
            let a = if i % 97 < 48 { 1.0 } else { 0.5 };
            [
                (i % 251) as f32 / 250.0 * a,
                (i % 113) as f32 / 112.0 * a,
                (i % 61) as f32 / 60.0 * a,
                a,
            ]
        }),
        &hooks,
    )
    .unwrap();
    let mut rows = Vec::new();
    for kind in WHOLE_EFFECTS.iter().chain(["posterize_time"].iter()) {
        let kind = *kind;
        let instance = effect_instance(kind, &[]);
        let passes = evaluate_effect(
            &project::Animations::default(),
            &instance,
            whole_time(kind),
            EffectGeometry::new([1920, 1080], [1.0, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        )
        .unwrap();
        let run = || {
            CpuEffects
                .apply(
                    &input,
                    &passes,
                    EffectInputs {
                        input: &input,
                        below: &input,
                        coverage: None,
                    },
                    &hooks,
                )
                .unwrap()
        };
        drop(run());
        let start = std::time::Instant::now();
        for _ in 0..2 {
            drop(run());
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / 2.0;
        eprintln!(
            "WHOLE_COST {kind} {ms:.3} ms/frame 1920x1080 passes={}",
            passes.len()
        );
        rows.push((kind, ms));
    }
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (kind, ms) in rows.iter().take(5) {
        eprintln!("WHOLE_SLOWEST {kind} {ms:.3} ms");
    }
    drop(input);
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}
