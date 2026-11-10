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
        for pass in evaluate(&frame).unwrap() {
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
    let passes =
        |fine| evaluate_effect(&Animations::default(), &view, Tick::ZERO, 64, 36, fine).unwrap();
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
        source.len() as u32,
        1,
        true,
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
    }
}

#[test]
fn pixel_effects_that_are_identity_at_default_emit_no_pass() {
    for kind in ["gamma_correction", "levels", "channel_mix"] {
        let passes = evaluate_effect(
            &project::Animations::default(),
            &effect_instance(kind, &[]),
            time::Tick::ZERO,
            13,
            1,
            true,
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
                for pass in (evaluator(kind).unwrap())(&frame).unwrap() {
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
    let rows: [(&str, Vec<(&str, EffectValue)>, u64); 11] = [
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
    let passes = evaluate_effect(animations, effect, at, 1, 1, fine)?;
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
