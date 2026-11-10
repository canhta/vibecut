#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::{Hooks, tick};
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
                validate_pass(&pass).unwrap();
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
    assert!(
        CpuEffects
            .apply(
                &image,
                &[bad],
                EffectInputs {
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
            validate_pass(&EffectPass::Pixel {
                entry_point: "grade",
                uniforms: weights.into_iter().map(Uniform::Float).collect(),
            })
            .is_err()
        );
    }
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
    let passes = chroma_key_passes(project::Color(key), intensity, shadow).unwrap();
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
        for pass in chroma_key_passes(project::Color(GREEN_KEY), intensity, 0.0).unwrap() {
            validate_pass(&pass).unwrap();
        }
    }
    // Intensity 0 is the reference's no-op and emits no pass.
    assert!(
        chroma_key_passes(project::Color(GREEN_KEY), 0.0, 0.0)
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
    for bad in [f64::NAN, f64::INFINITY, -0.1, 100.1] {
        assert!(chroma_key_passes(project::Color(GREEN_KEY), bad, 0.0).is_err());
        assert!(chroma_key_passes(project::Color(GREEN_KEY), 40.0, bad).is_err());
    }
    for weights in [
        vec![0.0, 1.0, 0.0, 0.4],
        vec![0.0, 1.0, 0.0, 0.4, 0.0, 0.0],
        vec![0.0, 1.0, 0.0, f32::NAN, 0.0],
        vec![0.0, 1.0, 2.0, 0.4, 0.0],
        vec![0.0, 1.0, 0.0, 0.4, 1.5],
    ] {
        assert!(
            validate_pass(&EffectPass::Pixel {
                entry_point: "chroma_key",
                uniforms: weights.into_iter().map(Uniform::Float).collect(),
            })
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
        for pass in evaluate(&frame).unwrap() {
            validate_pass(&pass).unwrap();
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
        evaluate_effect(&Animations::default(), &view, Tick::ZERO, 64, 36, fine).unwrap()
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
    // Left half red, right half blue: the two sides tell the reflection apart.
    let source = (0..32).map(|i| {
        if i % 8 < 4 {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 0.0, 1.0, 1.0]
        }
    });
    let image = Image::from_working_pixels(8, 4, source, &hooks).unwrap();
    let mirror = |values: [f32; 4]| EffectPass::Sample {
        entry_point: "mirror",
        uniforms: values.map(Uniform::Float).to_vec(),
    };
    // Reflect about the vertical line through the frame centre.
    let pass = mirror([4.0, 2.0, 1.0, 0.0]);
    validate_pass(&pass).unwrap();
    let at = |image: &Image, x: usize, y: usize| image.pixels()[y * 8 + x];
    let output = CpuEffects
        .apply(
            &image,
            &[pass],
            EffectInputs {
                coverage: None,
                below: &image,
            },
            &hooks,
        )
        .unwrap();
    assert_eq!(output.width(), 8);
    assert_eq!(output.height(), 4);
    // The negative side (left) keeps its pixels; the positive side samples the
    // reflection, so the blue right half becomes red.
    for x in 0..4 {
        assert_eq!(at(&output, x, 1), [1.0, 0.0, 0.0, 1.0], "x={x}");
    }
    assert_eq!(at(&output, 7, 1), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(&output, 4, 1), [0.5, 0.0, 0.5, 1.0]);
    // Same input, same output: no clock, no global state.
    let again = CpuEffects
        .apply(
            &image,
            &[mirror([4.0, 2.0, 1.0, 0.0])],
            EffectInputs {
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
        assert!(validate_pass(&bad).is_err(), "{bad:?}");
    }
    // An adapter without a pipeline for the pass answers Unsupported; the
    // renderer then runs the rest of the list on the CPU reference above.
    struct PixelOnly;
    impl EffectCompositor for PixelOnly {
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
