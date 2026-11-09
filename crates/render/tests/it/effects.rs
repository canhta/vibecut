#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::Hooks;
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
    let bad = EffectPass {
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
        effects: vec![EffectInstance {
            id: "b".into(),
            enabled: true,
            params: EffectParams::Blur { intensity: 100.0 },
        }],
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
            validate_pass(&EffectPass {
                entry_point: "grade",
                horizontal: false,
                weights,
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
        .map_or(pixel, |pass| chroma_key_pixel(pixel, &pass.weights))
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
            validate_pass(&EffectPass {
                entry_point: "chroma_key",
                horizontal: false,
                weights,
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
    let keyed_out = render(vec![EffectInstance {
        id: "chroma_key".into(),
        enabled: true,
        params: EffectParams::ChromaKey {
            key_color: Color(GREEN_KEY),
            intensity: 40.0,
            shadow: 0.0,
        },
    }]);
    let at = |image: &Image, x: usize| image.pixels()[20 * 40 + x];
    for x in [5, 35] {
        assert_eq!(at(&keyed_out, x), [0.0, 0.0, 1.0, 1.0], "x={x}");
    }
    // A near key keeps partial alpha: inside the mask it blends, outside it is
    // fully masked out by the coverage stage after the effect.
    let partial = render(vec![EffectInstance {
        id: "chroma_key".into(),
        enabled: true,
        params: EffectParams::ChromaKey {
            key_color: Color([0, 255, 60, 255]),
            intensity: 40.0,
            shadow: 0.0,
        },
    }]);
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
    let bright = render(vec![EffectInstance {
        id: "color_grade".into(),
        enabled: true,
        params: EffectParams::ColorGrade {
            brightness: 50.0,
            contrast: 0.0,
            saturation: 0.0,
            hue: 0.0,
        },
    }]);
    let at = |image: &Image, x: usize| image.pixels()[20 * 40 + x];
    // Inside the mask the grade doubles the linear value; outside is transparent.
    assert!(at(&bright, 5)[0] > at(&plain, 5)[0] * 1.5);
    assert_eq!(at(&bright, 35)[3], 0.0);
    assert_eq!(at(&plain, 35)[3], 0.0);
}
