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
