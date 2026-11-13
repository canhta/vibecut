#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use eframe::wgpu;
use render::effects::EffectGeometry;
use render::{effects::*, *};
struct Hooks;
struct Lease;
impl Reservation for Lease {}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(&self, _: MemoryKind, _: u64) -> Result<Box<dyn Reservation>> {
        Ok(Box::new(Lease))
    }
}
fn pixel(entry_point: &'static str, values: &[f32]) -> EffectPass {
    EffectPass::Pixel {
        entry_point,
        uniforms: values.iter().copied().map(Uniform::Float).collect(),
    }
}
const MIX_IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
const MIX_CROSS: [f32; 12] = [
    2.0, -2.0, 0.25, -0.25, -0.5, 1.5, 0.5, 0.125, 0.25, 0.5, -2.0, 2.0,
];
fn curve_values(points: &[(f32, f32)]) -> Vec<f32> {
    let mut values = vec![points.len() as f32];
    for index in 0..project::CURVE_MAX_POINTS {
        let (x, y) = points.get(index).copied().unwrap_or((0.0, 0.0));
        values.push(x);
        values.push(y);
    }
    values
}
fn linear_colour_rows() -> Vec<EffectPass> {
    let mut rows = vec![
        pixel("black_white", &[]), // No parameters; nondefault parameter rows are N/A.
        pixel("lift", &[0.0, 0.0, 0.0]),
        pixel("lift", &[0.5, 0.5, 0.5]),
        pixel("lift", &[-0.5, 0.2, 1.0]),
        pixel("tone_gamma", &[1.0, 1.0, 1.0]),
        pixel("tone_gamma", &[0.25, 1.0, 4.0]),
        pixel("gain", &[1.0, 1.0, 1.0]),
        pixel("gain", &[0.0, 2.0, 4.0]),
        pixel("channel_mix", &MIX_IDENTITY),
        pixel(
            "channel_mix",
            &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        ),
        pixel("channel_mix", &MIX_CROSS),
        pixel(
            "channel_mix",
            &[
                0.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.1234567,
            ],
        ),
        pixel("tint", &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
        pixel("tint", &[0.2, 0.4, 0.6, 0.2, 0.4, 0.6, 1.0]),
    ];
    for amount in [0.0, 0.35, 1.0] {
        rows.push(pixel(
            "tint",
            &[0.1234567, 0.75, 0.05, 0.91, 0.234567, 0.66, amount],
        ));
    }
    rows
}
fn levels_strobe_rows() -> Vec<EffectPass> {
    let mut rows = vec![
        pixel("levels", &[0.0, 1.0, 0.0, 1.0, 1.0]),
        pixel("levels", &[0.1, 0.8, 0.05, 0.9, 0.7]),
        pixel("levels", &[0.8, 0.2, 1.0, 0.0, 2.0]),
        pixel("levels", &[0.4, 0.4, 0.2, 0.8, 1.0]),
        pixel("levels", &[0.1, 0.9, 0.37, 0.37, 1.0]),
        pixel("levels", &[0.0, 1.0, 0.0, 1.0, 1e-6]),
        pixel("levels", &[0.0, 1.0, 0.0, 1.0, 12.0]),
        pixel("levels", &[0.4, 0.40001, 0.0, 1.0, 2.0]),
        pixel("strobe", &[1.0, 1.0, 1.0, 0.0]),
        pixel("strobe", &[0.0, 0.0, 0.0, 0.35]),
        pixel("strobe", &[1.0, 0.0, 0.0, 0.0]),
    ];
    for blend in [0.0, 0.35, 1.0] {
        rows.push(pixel("strobe", &[0.1234567, 0.456789, 0.789123, blend]));
    }
    rows
}
// Independent f64 colour equations, not calls into the f32 CPU reference.
fn expected_linear_colour(pass: &EffectPass, source: [f32; 4]) -> Option<[f64; 4]> {
    let values = pass.values();
    let a = f64::from(source[3]);
    let c = [0, 1, 2].map(|k| {
        if a > 0.0 {
            f64::from(source[k]) / a
        } else {
            0.0
        }
    });
    let encode = |v: f64| {
        if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    let decode = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let e = c.map(encode);
    let rgb = match pass.entry_point() {
        "black_white" => [luma; 3],
        "lift" => [0, 1, 2].map(|k| (c[k] + f64::from(values[k]) * (1.0 - c[k])).clamp(0.0, 1.0)),
        "tone_gamma" => [0, 1, 2].map(|k| {
            c[k].max(0.0)
                .powf(1.0 / f64::from(values[k]).max(0.25))
                .clamp(0.0, 1.0)
        }),
        "gain" => [0, 1, 2].map(|k| (c[k] * f64::from(values[k])).clamp(0.0, 1.0)),
        "channel_mix" => [0, 1, 2].map(|k| {
            let row = k * 4;
            decode(
                f64::from(values[row]) * e[0]
                    + f64::from(values[row + 1]) * e[1]
                    + f64::from(values[row + 2]) * e[2]
                    + f64::from(values[row + 3]),
            )
        }),
        "tint" => [0, 1, 2].map(|k| {
            let black = f64::from(values[k]);
            let target = black + (f64::from(values[k + 3]) - black) * encode(luma);
            decode(e[k] + (target - e[k]) * f64::from(values[6]))
        }),
        "levels" => e.map(|v| {
            let span = (f64::from(values[1]) - f64::from(values[0])).max(1e-3);
            let fraction = ((v - f64::from(values[0])) / span).clamp(0.0, 1.0);
            decode(
                f64::from(values[2])
                    + fraction.powf(f64::from(values[4]))
                        * (f64::from(values[3]) - f64::from(values[2])),
            )
        }),
        "strobe" => [0, 1, 2].map(|k| {
            let colour = decode(f64::from(values[k]));
            colour + (c[k] - colour) * f64::from(values[3])
        }),
        _ => return None,
    };
    Some([rgb[0] * a, rgb[1] * a, rgb[2] * a, a])
}
fn pixel_rows() -> Vec<EffectPass> {
    let mut rows: Vec<_> = [1.0, 1e-6, 1e-3, 0.7, 2.8, 4.0]
        .map(|g| pixel("gamma", &[g]))
        .into();
    for channel in 0..=4 {
        for blend in [0.0, 0.35, 1.0] {
            rows.push(pixel("invert", &[channel as f32, blend]));
        }
    }
    // The pass validator admits fractional channel values, with CPU round-away ties.
    for channel in [0.49, 0.5, 1.5, 3.5] {
        rows.push(pixel("invert", &[channel, 0.35]));
    }
    rows.extend(linear_colour_rows());
    rows.extend(levels_strobe_rows());
    rows
}
fn max_error(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    a.pixels()
        .iter()
        .zip(b.pixels())
        .flat_map(|(a, b)| a.iter().zip(b))
        .map(|(a, b)| {
            assert!(a.is_finite() && b.is_finite());
            (a - b).abs()
        })
        .fold(0.0, f32::max)
}
fn pixel_parity(gpu: &vibecut::services::gpu::effects::GpuEffects, image: &Image, label: &str) {
    let original = image.pixels().to_vec();
    for pass in pixel_rows() {
        assert!(gpu.supports(pass.entry_point()));
        let inputs = || EffectInputs {
            input: image,
            coverage: None,
            below: image,
        };
        let cpu = CpuEffects
            .apply(image, std::slice::from_ref(&pass), inputs(), &Hooks)
            .unwrap();
        // Direct adapter execution: Unsupported cannot be hidden by CPU retry.
        let actual = gpu
            .apply(image, std::slice::from_ref(&pass), inputs(), &Hooks)
            .unwrap();
        let max = max_error(&cpu, &actual);
        assert!(max <= 2e-5, "{label} {pass:?} max_abs={max}");
        for (out, src) in actual.pixels().iter().zip(image.pixels()) {
            assert!(out[..3].iter().all(|v| *v >= 0.0 && *v <= out[3]));
            let values = pass.values();
            let alpha = if pass.entry_point() == "invert" && values[0].round() == 4.0 {
                (1.0 - src[3]) * (1.0 - values[1]) + src[3] * values[1]
            } else {
                src[3]
            };
            assert!((out[3] - alpha).abs() <= 2e-5, "{pass:?} alpha");
            if let Some(expected) = expected_linear_colour(&pass, *src) {
                assert_eq!(out[3], src[3], "linear-colour alpha {pass:?}");
                for (actual, expected) in out.iter().zip(expected) {
                    assert!(
                        (f64::from(*actual) - expected).abs() <= 2e-5,
                        "independent colour {label} {pass:?}: {out:?} vs {expected}"
                    );
                }
                if pass.entry_point() == "black_white" {
                    assert_eq!(out[0], out[1]);
                    assert_eq!(out[1], out[2]);
                }
            }
        }
        if label.starts_with("scale") {
            let values = pass.values();
            let identity = match pass.entry_point() {
                "gamma" => values[0] == 1.0,
                "invert" => values[1] == 1.0,
                "channel_mix" => values.as_ref() == MIX_IDENTITY,
                "tint" => values[6] == 0.0,
                "levels" => values.as_ref() == [0.0, 1.0, 0.0, 1.0, 1.0],
                "strobe" => values[3] == 1.0,
                "lift" => values.iter().all(|v| *v == 0.0),
                "tone_gamma" => values.iter().all(|v| *v == 1.0),
                "gain" => values.iter().all(|v| *v == 1.0),
                _ => false,
            };
            if identity {
                assert!(max_error(image, &actual) <= 2e-5, "identity {pass:?}");
            } else {
                assert!(max_error(image, &actual) > 1e-5, "visible {pass:?}");
            }
        }
        eprintln!("pixel-parity {label} {pass:?} max_abs={max:e}");
        assert_eq!(image.pixels(), original, "immutable input");
    }
}
#[test]
fn gpu_matches_cpu_working_pixels_at_both_scales() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) if adapter.get_info().device_type != wgpu::DeviceType::Cpu => adapter,
        other => {
            assert!(
                std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
                "required native GPU unavailable: {other:?}"
            );
            eprintln!("SKIP: no native GPU adapter: {other:?}");
            return;
        }
    };
    eprintln!("GPU parity adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let gpu = std::sync::Arc::new(
        pollster::block_on(vibecut::services::gpu::effects::GpuEffects::new(
            device, queue,
        ))
        .unwrap(),
    );
    for entry in ["color_mix", "gamma_unknown", "posterize"] {
        assert!(!gpu.supports(entry), "unadmitted {entry}");
    }
    // The public image boundary rejects HDR/out-of-gamut working pixels; decoded
    // media must already satisfy that owner's premultiplied/clamped contract.
    for color in [
        [1.2, 0.0, 0.0, 1.0],
        [-0.1, 0.0, 0.0, 1.0],
        [0.4, 0.0, 0.0, 0.2],
    ] {
        assert!(matches!(
            Image::new(1, 1, color, &Hooks),
            Err(Error::Invalid(_))
        ));
    }
    let edge_pixels = [
        [0.0; 4],
        [1.0; 4],
        [0.0, 0.5, 0.25, 0.5],
        [0.0031308, 0.0031307, 0.0031309, 1.0],
        [1e-30, 0.5e-30, 0.0, 1e-30],
        [1e-8, 0.5e-8, 0.0, 1e-8],
        [1e-6, 0.5e-6, 0.0, 1e-6],
        [0.49999997, 0.0, 0.25, 0.5],
    ];
    for (w, h) in [(1, 1), (1, 9), (9, 1), (17, 9)] {
        let image = Image::from_working_pixels(
            w,
            h,
            (0..w * h).map(|i| edge_pixels[i as usize % edge_pixels.len()]),
            &Hooks,
        )
        .unwrap();
        pixel_parity(&gpu, &image, &format!("edges-{w}x{h}"));
    }
    // Primary colours independently expose each Rec.709 weight and channel row.
    let primaries = Image::from_working_pixels(
        3,
        1,
        [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.5, 0.0, 0.5],
            [0.0, 0.0, 0.25, 0.25],
        ]
        .into_iter(),
        &Hooks,
    )
    .unwrap();
    pixel_parity(&gpu, &primaries, "primaries-3x1");
    // Curves are CPU-only until a 65-value native admission exists; a mixed run
    // still executes supported neighbours natively and matches the CPU path.
    let curve = pixel(
        "curve_red",
        &curve_values(&[(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)]),
    );
    assert!(!gpu.supports("curve_red"));
    let curve_inputs = || EffectInputs {
        input: &primaries,
        coverage: None,
        below: &primaries,
    };
    assert!(matches!(
        gpu.apply(
            &primaries,
            std::slice::from_ref(&curve),
            curve_inputs(),
            &Hooks
        ),
        Err(Error::Unsupported(_))
    ));
    let mixed = [
        pixel("lift", &[0.2, -0.1, 0.4]),
        curve,
        pixel("gain", &[0.8, 1.2, 0.6]),
    ];
    let cpu = CpuEffects
        .apply(&primaries, &mixed, curve_inputs(), &Hooks)
        .unwrap();
    let actual = apply_effect(
        Image::from_working_pixels(3, 1, primaries.pixels().iter().copied(), &Hooks).unwrap(),
        &mixed,
        &primaries,
        gpu.as_ref(),
        &Hooks,
    )
    .unwrap();
    let max = max_error(&cpu, &actual);
    assert!(max <= 2e-5, "mixed CPU/GPU curve run {max}");
    // Actual device admission and transparent singleton footprints use the
    // same contract as the CPU, including the largest admitted frame centres.
    for (w, h) in [(8, 4), (1, 3), (3, 1), (1, 1)] {
        let image = Image::new(w, h, [0.5, 0.0, 0.0, 1.0], &Hooks).unwrap();
        let pass = |v: [f32; 4]| EffectPass::Sample {
            entry_point: "mirror",
            uniforms: v.map(Uniform::Float).to_vec(),
        };
        let inputs = || EffectInputs {
            input: &image,
            coverage: None,
            below: &image,
        };
        for v in [
            [0.0, 0.0, 1.0, 0.0],
            [w as f32, h as f32, -0.6, -0.8],
            [0.25, 0.0, 1.0, 0.0],
            [0.0, 0.25, 0.0, 1.0],
        ] {
            let cpu = CpuEffects
                .apply(&image, &[pass(v)], inputs(), &Hooks)
                .unwrap();
            let actual = gpu.apply(&image, &[pass(v)], inputs(), &Hooks).unwrap();
            let max = cpu
                .pixels()
                .iter()
                .zip(actual.pixels())
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            assert!(
                max <= 2e-5,
                "GPU admitted/singleton difference {w}x{h} {v:?}: {max}"
            );
            if w == 1 && v == [0.25, 0.0, 1.0, 0.0] {
                assert!(actual.pixels().iter().all(|p| *p == [0.25, 0.0, 0.0, 0.5]));
            }
            if h == 1 && v == [0.0, 0.25, 0.0, 1.0] {
                assert!(actual.pixels().iter().all(|p| *p == [0.25, 0.0, 0.0, 0.5]));
            }
            eprintln!("mirror-admission {w}x{h} values={v:?} max_abs={max}");
        }
        for v in [
            [f32::MAX, 0.0, -1.0, 0.0],
            [-f32::MAX, 0.0, 1.0, 0.0],
            [0.0, f32::MAX, 0.0, -1.0],
            [0.0, -f32::MAX, 0.0, 1.0],
            [0.0, 0.0, 2.0, 0.0],
            [0.0, 0.0, 1e-30, 0.0],
            [f32::NAN, 0.0, 1.0, 0.0],
            [0.0, f32::INFINITY, 0.0, 1.0],
        ] {
            assert!(matches!(
                gpu.apply(&image, &[pass(v)], inputs(), &Hooks),
                Err(Error::Unsupported(_))
            ));
        }
    }
    let pair: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/render/tests/fixtures/blend-linear.json"
    ))
    .unwrap();
    let source: Vec<[f32; 4]> = serde_json::from_value(pair["source"].clone()).unwrap();
    let below: Vec<[f32; 4]> = serde_json::from_value(pair["below"].clone()).unwrap();
    let source = Image::from_working_pixels(4, 4, source.into_iter(), &Hooks).unwrap();
    for case in pair["cases"].as_array().unwrap() {
        let mode: project::BlendMode = serde_json::from_value(case["mode"].clone()).unwrap();
        let opacity = case["opacity"].as_f64().unwrap() as f32;
        let expected: Vec<[f32; 4]> = serde_json::from_value(case["pixels"].clone()).unwrap();
        let mut actual = Image::from_working_pixels(4, 4, below.iter().copied(), &Hooks).unwrap();
        gpu.composite(&mut actual, &source, opacity, mode, &Hooks)
            .unwrap();
        let max = actual
            .pixels()
            .iter()
            .zip(&expected)
            .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
            .fold(0.0_f32, f32::max);
        assert!(
            max <= 2e-5,
            "working golden GPU difference {mode:?} {opacity}: {max}"
        );
    }
    for scale in [1, 2] {
        let (w, h) = (64 * scale, 36 * scale);
        let image = Image::from_working_pixels(
            w,
            h,
            (0..w * h).map(|i| {
                let x = i % w;
                let y = i / w;
                let a = if x < w / 2 { 0.5 } else { 1.0 };
                if y < h / 4 {
                    [0.0; 4]
                } else {
                    [
                        a * (x % 7) as f32 / 7.0,
                        a * (y % 5) as f32 / 5.0,
                        0.1 * a,
                        a,
                    ]
                }
            }),
            &Hooks,
        )
        .unwrap();
        pixel_parity(&gpu, &image, &format!("scale-{scale}"));
        for intensity in [0.0, 15.0, 100.0] {
            let passes = blur_passes(intensity, w, h).unwrap();
            let inputs = || EffectInputs {
                input: &image,
                coverage: Some(&image),
                below: &image,
            };
            let cpu = CpuEffects.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let actual = gpu.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let max = cpu
                .pixels()
                .iter()
                .zip(actual.pixels())
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            eprintln!("scale={scale} intensity={intensity} max_abs={max}");
            assert!(max <= 2e-5, "GPU difference {max}");
        }
        for grade in [
            [50.0, 0.0, 0.0, 0.0],
            [-50.0, 50.0, -50.0, 0.0],
            [0.0, -50.0, 50.0, 50.0],
            [20.0, -30.0, 40.0, -25.0],
            [50.0; 4],
            [-50.0; 4],
        ] {
            let passes = grade_passes(grade[0], grade[1], grade[2], grade[3]).unwrap();
            let inputs = || EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            };
            let cpu = CpuEffects.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let actual = gpu.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let max = cpu
                .pixels()
                .iter()
                .zip(actual.pixels())
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            assert!(max <= 1e-4, "GPU grade difference {grade:?}: {max}");
        }
        for (key, intensity, shadow) in [
            ([0, 255, 0, 255], 40.0, 0.0),
            ([0, 255, 0, 255], 100.0, 0.0),
            ([0, 255, 0, 255], 40.0, 100.0),
            ([30, 60, 200, 255], 75.0, 50.0),
            ([128, 128, 128, 255], 40.0, 0.0),
        ] {
            let passes =
                chroma_key_passes(key.map(|v| f64::from(v) / 255.0), intensity, shadow).unwrap();
            let inputs = || EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            };
            let cpu = CpuEffects.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let actual = gpu.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let max = cpu
                .pixels()
                .iter()
                .zip(actual.pixels())
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            assert!(
                max <= 1e-4,
                "GPU chroma key difference {key:?} {intensity} {shadow}: {max}"
            );
        }
        // A whole-image Sample pass reads coordinates and the instance input on
        // both sides: the mirror row is the variant's GPU parity proof.
        assert!(gpu.supports("mirror"));
        for values in [
            [0.0, 0.0, 1.0, 0.0],
            [w as f32 / 2.0, h as f32 / 2.0, 1.0, 0.0],
            [w as f32 / 2.0, h as f32 / 2.0, 0.6, 0.8],
            [w as f32 * 0.3, h as f32 * 0.7, 0.0, -1.0],
        ] {
            let passes = [EffectPass::Sample {
                entry_point: "mirror",
                uniforms: values.map(Uniform::Float).to_vec(),
            }];
            let inputs = || EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            };
            let cpu = CpuEffects.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let actual = gpu.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let max = cpu
                .pixels()
                .iter()
                .zip(actual.pixels())
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            eprintln!("mirror scale={scale} values={values:?} max_abs={max}");
            assert!(max <= 2e-5, "GPU mirror difference {values:?}: {max}");
        }
        // The bilateral reduce-noise pass reads the current image only; the shipped
        // sigmas and two off-default pairs prove the shader against the CPU
        // reference, including the skipped taps at the image border. The window
        // sums 25 exp() taps whose range weight amplifies the encoded distance
        // (pow in oetf), so the row uses the grade/chroma-key tolerance of 1e-4.
        assert!(gpu.supports("reduce_noise"));
        let shipped = reduce_noise_passes();
        let custom = [(0.6, 0.3), (3.0, 0.05)].map(|(spatial, range)| EffectPass::Sample {
            entry_point: "reduce_noise",
            uniforms: vec![Uniform::Float(spatial), Uniform::Float(range)],
        });
        for passes in [shipped, custom.to_vec()] {
            let inputs = || EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            };
            let cpu = CpuEffects.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let actual = gpu.apply(&image, &passes, inputs(), &Hooks).unwrap();
            let max = max_error(&cpu, &actual);
            eprintln!("reduce-noise scale={scale} {passes:?} max_abs={max:e}");
            assert!(max <= 1e-4, "GPU reduce_noise difference {passes:?}: {max}");
            assert!(
                max_error(&image, &actual) > 1e-5,
                "reduce_noise visible {passes:?}"
            );
        }
        // One instance with a visible prefix and a Sample read of its original
        // input, split in both device/CPU directions on the actual adapter.
        let prefix = grade_passes(-50.0, 0.0, 0.0, 0.0).unwrap();
        let sample = [EffectPass::Sample {
            entry_point: "mirror",
            uniforms: [w as f32 / 2.0, h as f32 / 2.0, 1.0, 0.0]
                .map(Uniform::Float)
                .to_vec(),
        }];
        let all: Vec<_> = prefix.iter().chain(&sample).cloned().collect();
        let inputs = || EffectInputs {
            input: &image,
            coverage: None,
            below: &image,
        };
        let reference = CpuEffects.apply(&image, &all, inputs(), &Hooks).unwrap();
        for cpu_prefix in [false, true] {
            let prefix_adapter: &dyn EffectCompositor = if cpu_prefix {
                &CpuEffects
            } else {
                gpu.as_ref()
            };
            let sample_adapter: &dyn EffectCompositor = if cpu_prefix {
                gpu.as_ref()
            } else {
                &CpuEffects
            };
            let current = prefix_adapter
                .apply(&image, &prefix, inputs(), &Hooks)
                .unwrap();
            assert_ne!(current.pixels(), image.pixels());
            let actual = sample_adapter
                .apply(&current, &sample, inputs(), &Hooks)
                .unwrap();
            let max = reference
                .pixels()
                .iter()
                .zip(actual.pixels())
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            eprintln!("mixed-input scale={scale} cpu_prefix={cpu_prefix} max_abs={max}");
            assert!(max <= 2e-5, "GPU mixed-input difference {max}");
        }
        let below = Image::from_working_pixels(
            w,
            h,
            (0..w * h).map(|i| {
                let alpha = [0., 0.01, 0.25, 0.5, 1.][i as usize % 5];
                [
                    alpha * (i % 11) as f32 / 10.,
                    alpha * (i % 7) as f32 / 6.,
                    alpha * (i % 3) as f32 / 2.,
                    alpha,
                ]
            }),
            &Hooks,
        )
        .unwrap();
        for mode in project::BlendMode::ALL {
            for opacity in [0., 0.5, 1.] {
                let mut cpu =
                    Image::from_working_pixels(w, h, below.pixels().iter().copied(), &Hooks)
                        .unwrap();
                let mut actual =
                    Image::from_working_pixels(w, h, below.pixels().iter().copied(), &Hooks)
                        .unwrap();
                CpuEffects
                    .composite(&mut cpu, &image, opacity, mode, &Hooks)
                    .unwrap();
                gpu.composite(&mut actual, &image, opacity, mode, &Hooks)
                    .unwrap();
                let max = cpu
                    .pixels()
                    .iter()
                    .zip(actual.pixels())
                    .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
                    .fold(0.0_f32, f32::max);
                eprintln!("blend scale={scale} mode={mode:?} opacity={opacity} max_abs={max}");
                assert!(
                    max <= 2e-5,
                    "blend GPU difference {mode:?} {opacity}: {max}"
                );
            }
        }
    }
    // Exercise bounded repeated passes separately from the small-resolution scale fixtures.
    let image = Image::from_working_pixels(
        31,
        17,
        (0..527).map(|i| {
            if i % 3 == 0 {
                [0.5, 0.25, 0.0, 0.5]
            } else {
                [0.0; 4]
            }
        }),
        &Hooks,
    )
    .unwrap();
    let passes = blur_passes(100.0, 3840, 2160).unwrap();
    assert!(passes.len() > 2);
    let out = gpu
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &Hooks,
        )
        .unwrap();
    let reference = CpuEffects
        .apply(
            &image,
            &passes,
            EffectInputs {
                input: &image,
                coverage: None,
                below: &image,
            },
            &Hooks,
        )
        .unwrap();
    let max = reference
        .pixels()
        .iter()
        .zip(out.pixels())
        .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
        .fold(0.0_f32, f32::max);
    eprintln!("repeated-pass max_abs={max} passes={}", passes.len());
    assert!(max <= 2e-5);
    // The public thumbnail seam schedules real offscreen GPU work and reuses it.
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let thumbnails = gpu.thumbnails(budget.clone());
    use engine::jobs::*;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(10),
            progress_interval: std::time::Duration::from_millis(10),
        },
        budget.clone(),
        std::sync::Arc::new(SystemClock::default()),
    )
    .unwrap();
    let source = std::sync::Arc::new(
        Image::from_working_pixels(
            320,
            180,
            (0..320 * 180).map(|i| {
                [
                    ((i % 320) % 17) as f32 / 17.,
                    ((i / 320) % 11) as f32 / 11.,
                    0.1,
                    1.,
                ]
            }),
            &Hooks,
        )
        .unwrap(),
    );
    let mut previous = Vec::new();
    for run in 0..2 {
        let start = std::time::Instant::now();
        let (job, _) = thumbnails
            .submit(
                &mut runner,
                EffectGeometry::new([source.width(), source.height()], [1.0, 1.0]).unwrap(),
                source.clone(),
                project::effect_definition("blur")
                    .unwrap()
                    .instance("blur")
                    .with("intensity", project::EffectValue::Scalar(100.0))
                    .unwrap(),
                project::Animations::default(),
                time::Tick::ZERO,
            )
            .unwrap();
        loop {
            runner.tick().unwrap();
            let state = runner.snapshot(job).unwrap();
            if state.state == JobState::Completed {
                break;
            }
            assert!(
                !matches!(state.state, JobState::Failed | JobState::Cancelled),
                "{:?}",
                state.error
            );
            assert!(start.elapsed() < std::time::Duration::from_secs(20));
            std::thread::yield_now();
        }
        let result = runner.take_output(job).unwrap();
        let elapsed = start.elapsed();
        if run == 0 {
            let expected = CpuEffects
                .apply(
                    &source,
                    &blur_passes(100.0, 320, 180).unwrap(),
                    EffectInputs {
                        input: &source,
                        coverage: None,
                        below: &source,
                    },
                    &Hooks,
                )
                .unwrap()
                .rgba8(13, &Hooks)
                .unwrap();
            let max = result
                .bytes()
                .iter()
                .zip(&expected.bytes)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(max <= 1, "thumbnail GPU/CPU byte difference {max}");
        } else {
            assert_eq!(result.bytes(), previous);
        }
        previous = result.bytes().to_vec();
        eprintln!(
            "effect thumbnail 320x180 run={run} elapsed_us={} bytes={}",
            elapsed.as_micros(),
            result.bytes().len()
        );
    }
    // The existing injected-compositor hook records successful REAL device runs;
    // neither an Unsupported CPU retry nor a cached image can masquerade as one.
    struct NativeStrobe {
        gpu: std::sync::Arc<vibecut::services::gpu::effects::GpuEffects>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl EffectCompositor for NativeStrobe {
        fn frame<'a>(
            &'a self,
            _: &'a dyn ExecutionHooks,
        ) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
            Ok(None)
        }
        fn release_idle(&self) -> Result<()> {
            self.gpu.release_idle()
        }
        fn supports(&self, entry: &str) -> bool {
            self.gpu.supports(entry)
        }
        fn apply(
            &self,
            image: &Image,
            passes: &[EffectPass],
            inputs: EffectInputs<'_>,
            hooks: &dyn ExecutionHooks,
        ) -> Result<Image> {
            assert!(passes.iter().all(|pass| pass.entry_point() == "strobe"));
            let output = self.gpu.apply(image, passes, inputs, hooks)?;
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(output)
        }
    }
    let native = std::sync::Arc::new(NativeStrobe {
        gpu: gpu.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    assert!(native.supports("strobe"));
    let cache = engine::effect_thumbnails::EffectThumbnails::new(native.clone(), budget);
    let effect = project::effect_definition("strobe")
        .unwrap()
        .instance("native-flash");
    let mut red = project::Animations::default();
    red.set(
        effect.param_path("color").unwrap(),
        project::Param::new(project::ParamValue::Color([1.0, 0.0, 0.0, 1.0])).unwrap(),
    )
    .unwrap();
    let frozen = source.pixels().to_vec();
    let mut phases = Vec::new();
    for (frame, animated, calls) in [
        (0, false, 1),
        (3, false, 1),
        (15, false, 2),
        (15, false, 2),
        (0, true, 3),
        (0, true, 3),
    ] {
        let local = time::FrameRate::FPS_30.tick_of(frame).unwrap();
        let animations = if animated {
            red.clone()
        } else {
            project::Animations::default()
        };
        let passes = evaluate_effect(
            &animations,
            &effect,
            local,
            EffectGeometry::new([320, 180], [1.0, 1.0]).unwrap(),
            true,
            &render::lut::LutResources::default(),
        )
        .unwrap();
        assert_eq!(passes.is_empty(), frame == 3, "actual evaluator phase");
        let expected = CpuEffects
            .apply(
                &source,
                &passes,
                EffectInputs {
                    input: &source,
                    coverage: None,
                    below: &source,
                },
                &Hooks,
            )
            .unwrap()
            .rgba8(13, &Hooks)
            .unwrap();
        let (job, _) = cache
            .submit(
                &mut runner,
                EffectGeometry::new([source.width(), source.height()], [1.0, 1.0]).unwrap(),
                source.clone(),
                effect.clone(),
                animations,
                local,
            )
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            runner.tick().unwrap();
            let state = runner.snapshot(job).unwrap();
            if state.state == JobState::Completed {
                break;
            }
            assert!(
                !matches!(state.state, JobState::Failed | JobState::Cancelled),
                "{:?}",
                state.error
            );
            assert!(start.elapsed() < std::time::Duration::from_secs(20));
            std::thread::yield_now();
        }
        let output = runner.take_output(job).unwrap();
        let error = output
            .bytes()
            .iter()
            .zip(&expected.bytes)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(error <= 1, "native timed thumbnail byte error {error}");
        assert_eq!(
            native.calls.load(std::sync::atomic::Ordering::SeqCst),
            calls
        );
        if animated {
            assert!(
                output
                    .bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| *p == [255, 0, 0, 255])
            );
        } else if frame != 3 {
            assert!(
                output.bytes().iter().all(|v| *v == 255),
                "lit default is white"
            );
        } else {
            assert_eq!(output.bytes(), source.rgba8(13, &Hooks).unwrap().bytes);
        }
        eprintln!(
            "native-strobe frame={frame} animated={animated} gpu_calls={calls} byte_error={error}"
        );
        phases.push(output.bytes().to_vec());
        assert_eq!(source.pixels(), frozen, "thumbnail source immutable");
    }
    assert_ne!(phases[0], phases[1], "lit versus idle");
    assert_eq!(phases[0], phases[2], "period returns to lit");
    assert_eq!(phases[2], phases[3], "complete time identity caches");
    assert_ne!(phases[0], phases[4], "animation identity changes colour");
    assert_eq!(phases[4], phases[5], "animated repeat caches");
}

#[test]
#[ignore = "bounded native 1080p pixel-effect measurement; run explicitly on hardware"]
fn pixel_effects_native_1080p_measurement() {
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Meter {
        budget: engine::ByteBudget,
        peak: AtomicU64,
    }
    impl ExecutionHooks for Meter {
        fn cancelled(&self) -> bool {
            false
        }
        fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
            let lease = engine::ByteBudgetHooks::new(&self.budget, None).reserve(kind, bytes)?;
            self.peak
                .fetch_max(self.budget.snapshot().unwrap().used, Ordering::Relaxed);
            Ok(lease)
        }
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("measurement requires a native adapter");
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    eprintln!(
        "measurement adapter={:?} profile=dev warmup=2 repetitions=7 size=1920x1080",
        adapter.get_info()
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let start = std::time::Instant::now();
    let gpu = pollster::block_on(vibecut::services::gpu::effects::GpuEffects::new(
        device, queue,
    ))
    .unwrap();
    eprintln!(
        "measurement pipeline_setup_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let meter = Meter {
        budget: engine::ByteBudget::new(256 << 20).unwrap(),
        peak: AtomicU64::new(0),
    };
    let input = Image::from_working_pixels(
        1920,
        1080,
        (0..1920 * 1080).map(|i| {
            let a = [0.0, 0.01, 0.5, 1.0][i as usize % 4];
            [
                a * (i % 19) as f32 / 19.0,
                a * (i % 13) as f32 / 13.0,
                a * 0.1,
                a,
            ]
        }),
        &meter,
    )
    .unwrap();
    let bytes = Image::bytes(1920, 1080).unwrap();
    let inputs = || EffectInputs {
        input: &input,
        coverage: None,
        below: &input,
    };
    for pass in [
        pixel("levels", &[0.1, 0.8, 0.05, 0.9, 0.7]),
        pixel("strobe", &[0.1234567, 0.456789, 0.789123, 0.35]),
    ] {
        assert!(gpu.supports(pass.entry_point()));
        // Oracle remains outside timed/charged work; every timed result is verified.
        let expected = CpuEffects
            .apply(&input, std::slice::from_ref(&pass), inputs(), &Hooks)
            .unwrap();
        for run in 0..9 {
            let cpu: &dyn EffectCompositor = &CpuEffects;
            let device: &dyn EffectCompositor = &gpu;
            let order = if run % 2 == 0 {
                [("cpu", cpu), ("gpu", device)]
            } else {
                [("gpu", device), ("cpu", cpu)]
            };
            for (kind, adapter) in order {
                assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
                meter.peak.store(bytes, Ordering::Relaxed);
                let start = std::time::Instant::now();
                let output = adapter
                    .apply(&input, std::slice::from_ref(&pass), inputs(), &meter)
                    .unwrap();
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                let error = max_error(&expected, &output);
                assert!(error <= 2e-5, "measurement {pass:?} {kind} error={error}");
                let peak = meter.peak.load(Ordering::Relaxed);
                eprintln!(
                    "measurement entry={} kind={kind} run={run} warmup={} ms={ms:.3} max_abs={error:e} peak_charged_bytes={peak}",
                    pass.entry_point(),
                    run < 2
                );
                drop(output);
                assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
            }
        }
    }
    drop(input);
    assert_eq!(meter.budget.snapshot().unwrap().used, 0);
}

#[test]
fn unused_gpu_inputs_preserve_pixels_with_a_smaller_budget() {
    use project::BlendMode::Normal;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Meter {
        budget: engine::ByteBudget,
        requests: Mutex<Vec<u64>>,
        cancel: AtomicBool,
    }
    impl Meter {
        fn new(limit: u64) -> Self {
            Self {
                budget: engine::ByteBudget::new(limit).unwrap(),
                requests: Mutex::new(Vec::new()),
                cancel: AtomicBool::new(false),
            }
        }
        fn take(&self) -> Vec<u64> {
            std::mem::take(&mut *self.requests.lock().unwrap())
        }
    }
    impl ExecutionHooks for Meter {
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::SeqCst)
        }
        fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
            self.requests.lock().unwrap().push(bytes);
            engine::ByteBudgetHooks::new(&self.budget, None).reserve(kind, bytes)
        }
    }
    fn parity(a: &Image, b: &Image) {
        assert_eq!((a.width(), a.height()), (b.width(), b.height()));
        let max = a
            .pixels()
            .iter()
            .zip(b.pixels())
            .flat_map(|(a, b)| {
                a.iter().zip(b).map(|(a, b)| {
                    assert!(a.is_finite() && b.is_finite());
                    (a - b).abs()
                })
            })
            .fold(0.0f32, f32::max);
        assert!(max <= 2e-5, "working-channel difference {max}");
        eprintln!(
            "liveness parity {}x{} max_abs={max:e}",
            a.width(),
            a.height()
        );
    }
    use tracing_subscriber::prelude::*;
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_test_writer()
            .with_filter(tracing_subscriber::filter::filter_fn(|meta| {
                meta.target() == "vibecut::services::gpu::effects"
            })),
    );
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) if adapter.get_info().device_type != wgpu::DeviceType::Cpu => adapter,
        other => {
            assert!(
                std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
                "required native GPU unavailable: {other:?}"
            );
            eprintln!("SKIP: no native GPU");
            return;
        }
    };
    eprintln!("liveness GPU adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    eprintln!(
        "liveness requested=default device_limits={:?}",
        device.limits()
    );
    let gpu = std::sync::Arc::new(
        pollster::block_on(vibecut::services::gpu::effects::GpuEffects::new(
            device, queue,
        ))
        .unwrap(),
    );
    let bytes = Image::bytes(17, 9).unwrap();
    let meter = Meter::new(6 * bytes + 4096);
    let source = Image::new(17, 9, [0.2, 0.1, 0.3, 0.5], &meter).unwrap();
    assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
    meter.take();
    let resources = |requests: Vec<u64>, bytes: u64, passes: usize, aux: bool| {
        assert!(requests[0] > 0 && requests[0] <= 4096);
        let expected = if aux {
            vec![4 * bytes, 2 * bytes, 1024 * passes as u64, bytes]
        } else {
            vec![4 * bytes, 1024 * passes as u64, bytes]
        };
        assert_eq!(&requests[1..], expected);
    };
    let grade = grade_passes(20.0, 0.0, 0.0, 0.0).unwrap();
    let inputs = || EffectInputs {
        input: &source,
        below: &source,
        coverage: None,
    };
    let out = gpu.apply(&source, &grade, inputs(), &meter).unwrap();
    resources(meter.take(), bytes, 1, false);
    assert_eq!(meter.budget.snapshot().unwrap().used, 2 * bytes);
    parity(
        &out,
        &CpuEffects.apply(&source, &grade, inputs(), &Hooks).unwrap(),
    );
    drop(out);
    assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
    for pass in [pixel("gamma", &[0.7]), pixel("invert", &[4.0, 0.35])]
        .into_iter()
        .chain(linear_colour_rows())
        .chain(levels_strobe_rows())
    {
        let original = source.pixels().to_vec();
        let out = gpu
            .apply(&source, std::slice::from_ref(&pass), inputs(), &meter)
            .unwrap();
        resources(meter.take(), bytes, 1, false);
        parity(
            &out,
            &CpuEffects
                .apply(&source, std::slice::from_ref(&pass), inputs(), &Hooks)
                .unwrap(),
        );
        drop(out);
        assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
        let bad = Image::new(18, 9, [0.0; 4], &Hooks).unwrap();
        assert!(matches!(
            gpu.apply(
                &source,
                std::slice::from_ref(&pass),
                EffectInputs {
                    input: &bad,
                    ..inputs()
                },
                &meter
            ),
            Err(Error::Invalid(_))
        ));
        assert!(meter.take().is_empty(), "geometry before allocation");
        let tight = Meter::new(1);
        assert!(matches!(
            gpu.apply(&source, std::slice::from_ref(&pass), inputs(), &tight),
            Err(Error::Budget { .. })
        ));
        assert_eq!(tight.budget.snapshot().unwrap().used, 0);
        meter.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(
            gpu.apply(&source, &[pass], inputs(), &meter),
            Err(Error::Cancelled)
        ));
        assert!(meter.take().is_empty());
        meter.cancel.store(false, Ordering::SeqCst);
        assert_eq!(source.pixels(), original);
    }
    for pass in [
        pixel("gamma", &[0.0]),
        pixel("gamma", &[4.0001]),
        pixel("gamma", &[f32::NAN]),
        pixel("gamma", &[]),
        pixel("invert", &[5.0, 0.0]),
        pixel("invert", &[-0.1, 0.0]),
        pixel("invert", &[0.0, -0.1]),
        pixel("invert", &[0.0, 1.0001]),
        pixel("invert", &[0.0, f32::INFINITY]),
        pixel("invert", &[0.0]),
        pixel("gamma_unknown", &[1.0]),
        pixel("black_white", &[1.0]),
        pixel("channel_mix", &[1.0; 11]),
        pixel("channel_mix", &[2.0001; 12]),
        pixel("channel_mix", &[-2.0001; 12]),
        pixel("channel_mix", &[f32::NAN; 12]),
        pixel("tint", &[0.5; 6]),
        pixel("tint", &[-0.0001, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
        pixel("tint", &[0.0, 0.0, 0.0, 1.0001, 1.0, 1.0, 1.0]),
        pixel("tint", &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0, -0.0001]),
        pixel("tint", &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0001]),
        pixel("tint", &[f32::INFINITY; 7]),
        pixel("levels", &[0.0; 4]),
        pixel("levels", &[-0.0001, 1.0, 0.0, 1.0, 1.0]),
        pixel("levels", &[0.0, 1.0001, 0.0, 1.0, 1.0]),
        pixel("levels", &[0.0, 1.0, -0.0001, 1.0, 1.0]),
        pixel("levels", &[0.0, 1.0, 0.0, 1.0001, 1.0]),
        pixel("levels", &[0.0, 1.0, 0.0, 1.0, 0.0]),
        pixel("levels", &[0.0, 1.0, 0.0, 1.0, 12.0001]),
        pixel("levels", &[0.0, 1.0, 0.0, 1.0, f32::NAN]),
        pixel("strobe", &[0.0; 3]),
        pixel("strobe", &[-0.0001, 0.0, 0.0, 0.0]),
        pixel("strobe", &[0.0, 1.0001, 0.0, 0.0]),
        pixel("strobe", &[0.0, 0.0, 0.0, -0.0001]),
        pixel("strobe", &[0.0, 0.0, 0.0, 1.0001]),
        pixel("strobe", &[f32::INFINITY; 4]),
    ] {
        assert!(matches!(
            gpu.apply(&source, &[pass], inputs(), &meter),
            Err(Error::Unsupported(_))
        ));
        assert!(meter.take().is_empty(), "invalid pass before allocation");
    }
    eprintln!(
        "budget boundary image_bytes={bytes} limit={} old_gpu_request={} scratch_bytes={} scope_records=separately_reserved result_bytes={bytes}",
        6 * bytes + 4096,
        7 * bytes + 1024,
        4 * bytes + 1024
    );
    drop(source);
    assert_eq!(meter.budget.snapshot().unwrap().used, 0);
    let meter = Meter::new(1 << 24);
    let hooks: &dyn ExecutionHooks = &meter;
    let source = Image::new(17, 9, [0.2, 0.1, 0.3, 0.5], hooks).unwrap();
    meter.take();
    let scope = gpu.frame(hooks).unwrap().unwrap();
    let records = meter.take()[0];
    assert!(matches!(gpu.frame(hooks), Err(Error::Unsupported(_))));
    assert!(matches!(scope.frame(hooks), Err(Error::Invalid(_))));
    let inputs = || EffectInputs {
        input: &source,
        below: &source,
        coverage: None,
    };
    let blur = blur_passes(15.0, 1920, 1080).unwrap();
    for passes in [&grade, &blur, &grade] {
        let actual = scope.apply(&source, passes, inputs(), hooks).unwrap();
        parity(
            &actual,
            &CpuEffects.apply(&source, passes, inputs(), &Hooks).unwrap(),
        );
    }
    let requests = meter.take();
    assert_eq!(
        requests.iter().filter(|v| **v == 4 * bytes).count(),
        1,
        "reuse core allocation"
    );
    assert_eq!(
        requests.iter().filter(|v| **v == bytes).count(),
        3,
        "separate result leases"
    );
    let mirror = vec![EffectPass::Sample {
        entry_point: "mirror",
        uniforms: [8.5, 4.5, 1.0, 0.0].map(Uniform::Float).to_vec(),
    }];
    drop(scope.apply(&source, &mirror, inputs(), hooks).unwrap());
    assert!(meter.take().contains(&(2 * bytes)));
    let before = meter.budget.snapshot().unwrap().used;
    drop(scope.apply(&source, &grade, inputs(), hooks).unwrap());
    assert_eq!(
        before - meter.budget.snapshot().unwrap().used,
        2 * bytes,
        "drop unused auxiliary"
    );
    assert_eq!(meter.take(), [bytes]);
    let bad = Image::new(18, 9, [0.0; 4], &Hooks).unwrap();
    assert!(matches!(
        scope.apply(
            &bad,
            &grade,
            EffectInputs {
                input: &bad,
                below: &bad,
                coverage: None
            },
            hooks
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        scope.apply(&source, &grade, inputs(), &Hooks),
        Err(Error::Invalid("GPU scope hooks"))
    ));
    scope.release_idle().unwrap();
    assert_eq!(meter.budget.snapshot().unwrap().used, bytes + records);
    drop(scope);
    drop(source);
    assert_eq!(meter.budget.snapshot().unwrap().used, 0);
    // Refusal during auxiliary growth and AFTER a completed submission's result admission.
    for (aux, limit) in [
        (true, 5 * bytes + records + 1024),
        (false, 4 * bytes + records + 1024),
    ] {
        let meter = Meter::new(limit);
        let hooks: &dyn ExecutionHooks = &meter;
        let source = Image::new(17, 9, [0.2; 4], &Hooks).unwrap();
        let scope = gpu.frame(hooks).unwrap().unwrap();
        let inputs = || EffectInputs {
            input: &source,
            below: &source,
            coverage: None,
        };
        if aux {
            drop(scope.apply(&source, &grade, inputs(), hooks).unwrap());
        }
        let passes = if aux { &mirror } else { &grade };
        assert!(matches!(
            scope.apply(&source, passes, inputs(), hooks),
            Err(Error::Budget { .. })
        ));
        assert_eq!(
            meter.take().last(),
            Some(&if aux { 2 * bytes } else { bytes })
        );
        scope.release_idle().unwrap();
        drop(scope);
        assert_eq!(meter.budget.snapshot().unwrap().used, 0);
    }
    // Metadata growth is admitted before any replacement buffer allocation.
    let meter = Meter::new(records + 5 * bytes + 1024);
    let hooks: &dyn ExecutionHooks = &meter;
    let source = Image::new(17, 9, [0.2; 4], &Hooks).unwrap();
    let scope = gpu.frame(hooks).unwrap().unwrap();
    let inputs = || EffectInputs {
        input: &source,
        below: &source,
        coverage: None,
    };
    drop(scope.apply(&source, &grade, inputs(), hooks).unwrap());
    assert!(matches!(
        scope.apply(&source, &vec![grade[0].clone(); 4], inputs(), hooks),
        Err(Error::Budget {
            requested: 4096,
            ..
        })
    ));
    assert_eq!(meter.take().last(), Some(&4096));
    drop(scope);
    assert_eq!(meter.budget.snapshot().unwrap().used, 0);

    struct DecodePressure;
    impl SourceProvider for DecodePressure {
        fn status(
            &mut self,
            _: &media::SourceAsset,
            _: bool,
            hooks: &dyn ExecutionHooks,
        ) -> Result<Option<UnavailableReason>> {
            // Real FileSources uses this same temporary probe charge. Retained
            // six-image scratch would refuse it in this fixture's real budget.
            drop(hooks.reserve(MemoryKind::Decoded, 256 << 20)?);
            Ok(None)
        }
        fn video(
            &mut self,
            _: &media::SourceAsset,
            _: project::ItemId,
            _: time::Tick,
            hooks: &dyn ExecutionHooks,
        ) -> Result<Option<Image>> {
            Ok(Some(Image::new(17, 9, [0.1, 0.2, 0.3, 1.0], hooks)?))
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
            panic!("video-only fixture")
        }
    }
    let mut data = project::Project::empty(project::ProjectId(1), "pressure".into())
        .unwrap()
        .into_data();
    data.canvas.width = 17;
    data.canvas.height = 9;
    let duration = time::Tick(time::TICKS_PER_SECOND);
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("scratch-pressure"))
            .format("synthetic")
            .duration(duration)
            .stream(
                testkit::source::video()
                    .size(17, 9)
                    .duration(duration)
                    .build(),
            )
            .build(),
    );
    let first = testkit::item::item(
        project::ItemId(1),
        project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([100, 50, 20, 255]),
        }),
    )
    .source(time::SourceRange::new(time::Tick::ZERO, duration).unwrap())
    .effects(vec![
        project::effect_definition("color_grade")
            .unwrap()
            .instance("grade")
            .with("brightness", project::EffectValue::Scalar(30.0))
            .unwrap(),
        project::effect_definition("gamma_correction")
            .unwrap()
            .instance("gamma")
            .with("gamma", project::EffectValue::Scalar(7.0))
            .unwrap(),
        project::effect_definition("invert")
            .unwrap()
            .instance("invert"),
        project::effect_definition("black_white")
            .unwrap()
            .instance("grey"),
        project::effect_definition("channel_mix")
            .unwrap()
            .instance("mix")
            .with("rg", project::EffectValue::Scalar(50.0))
            .unwrap(),
        project::effect_definition("tint")
            .unwrap()
            .instance("tint")
            .with(
                "white",
                project::EffectValue::Color(project::Color([220, 80, 30, 255])),
            )
            .unwrap(),
        project::effect_definition("posterize")
            .unwrap()
            .instance("cpu-only"),
        project::effect_definition("levels")
            .unwrap()
            .instance("levels")
            .with("in_black", project::EffectValue::Scalar(25.0))
            .unwrap(),
        project::effect_definition("strobe")
            .unwrap()
            .instance("flash")
            .with("blend", project::EffectValue::Scalar(35.0))
            .unwrap(),
    ])
    .build();
    let mut second = first.clone();
    second.id = project::ItemId(2);
    second.effects.clear();
    second.source_asset = Some(media::AssetId(1));
    second.content = project::ItemParams::Video(project::VideoParams::plain());
    data.scene_mut().timeline.main.items.push(first);
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "video".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![second],
    });
    let project = project::Project::new(data).unwrap();
    let meter = Meter::new((256 << 20) + 2 * bytes + 4096);
    let hooks: &dyn ExecutionHooks = &meter;
    let reference = Renderer::new(&project, RenderSettings::from_project(&project))
        .unwrap()
        .render_frame(time::Tick::ZERO, &mut DecodePressure, hooks)
        .unwrap();
    for busy in [true, false] {
        let held = busy.then(|| gpu.frame(hooks).unwrap().unwrap());
        meter.take();
        let mut settings = RenderSettings::from_project(&project);
        settings.effects = Some(gpu.clone());
        let actual = std::thread::scope(|threads| {
            threads
                .spawn(|| {
                    Renderer::new(&project, settings).unwrap().render_frame(
                        time::Tick::ZERO,
                        &mut DecodePressure,
                        hooks,
                    )
                })
                .join()
                .unwrap()
        })
        .unwrap();
        parity(&actual, &reference);
        let requests = meter.take();
        assert_eq!(
            requests.contains(&(4 * bytes)),
            !busy,
            "whole-frame GPU selection"
        );
        assert!(
            requests.contains(&(256 << 20)),
            "next decoder's probe was admitted"
        );
        drop(actual);
        drop(held);
        assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
    }
    drop(reference);
    assert_eq!(meter.budget.snapshot().unwrap().used, 0);
    for (w, h) in [(17, 9), (1, 5), (5, 1), (1, 1)] {
        let bytes = Image::bytes(w, h).unwrap();
        let meter = Meter::new(64 * bytes + 65536);
        let input = Image::from_working_pixels(
            w,
            h,
            (0..w * h).map(|i| {
                let a = if i % 3 == 1 { 0.0 } else { 0.5 };
                [a * (i % 5) as f32 / 5.0, a * 0.3, a * 0.6, a]
            }),
            &Hooks,
        )
        .unwrap();
        let other = Image::new(w, h, [0.1, 0.2, 0.4, 0.5], &Hooks).unwrap();
        let zero = Image::new(w, h, [0.0; 4], &Hooks).unwrap();
        let start = CpuEffects
            .apply(
                &input,
                &grade,
                EffectInputs {
                    input: &input,
                    below: &other,
                    coverage: None,
                },
                &Hooks,
            )
            .unwrap();
        assert_ne!(input.pixels(), start.pixels());
        let mirror = |nx, ny| EffectPass::Sample {
            entry_point: "mirror",
            uniforms: [w as f32 / 2.0, h as f32 / 2.0, nx, ny]
                .map(Uniform::Float)
                .to_vec(),
        };
        let mut plain = grade.clone();
        plain.push(pixel("gamma", &[0.7]));
        plain.push(pixel("invert", &[3.0, 0.35]));
        plain.push(pixel("black_white", &[]));
        plain.push(pixel("channel_mix", &MIX_CROSS));
        plain.push(pixel(
            "tint",
            &[0.1234567, 0.75, 0.05, 0.91, 0.234567, 0.66, 0.35],
        ));
        plain.extend(chroma_key_passes([0.0, 1.0, 0.0, 1.0], 40.0, 0.0).unwrap());
        plain.push(pixel("levels", &[0.1, 0.8, 0.05, 0.9, 0.7]));
        plain.push(pixel("strobe", &[0.1234567, 0.456789, 0.789123, 0.35]));
        plain.extend(blur_passes(5.0, 1920, 1080).unwrap());
        let mut mixed = plain.clone();
        mixed.insert(1, mirror(1.0, 0.0));
        mixed.push(mirror(0.0, 1.0));
        for (passes, extra) in [(plain, 0), (mixed, 1)] {
            let mut current =
                Image::from_working_pixels(w, h, start.pixels().iter().copied(), &Hooks).unwrap();
            for step in 0..2 {
                let frozen = if step == 0 { &input } else { &other };
                let inputs = |different| EffectInputs {
                    input: frozen,
                    below: if different { &other } else { &zero },
                    coverage: Some(if different { &other } else { &zero }),
                };
                let expected = CpuEffects
                    .apply(&current, &passes, inputs(false), &Hooks)
                    .unwrap();
                for different in [false, true] {
                    let actual = gpu
                        .apply(&current, &passes, inputs(different), &meter)
                        .unwrap();
                    resources(meter.take(), bytes, passes.len(), extra != 0);
                    parity(&actual, &expected);
                    if different {
                        current = actual;
                    }
                }
            }
        }
        let mut below =
            Image::from_working_pixels(w, h, other.pixels().iter().copied(), &Hooks).unwrap();
        let mut expected =
            Image::from_working_pixels(w, h, other.pixels().iter().copied(), &Hooks).unwrap();
        CpuEffects
            .composite(&mut expected, &start, 0.7, Normal, &Hooks)
            .unwrap();
        gpu.composite(&mut below, &start, 0.7, Normal, &meter)
            .unwrap();
        resources(meter.take(), bytes, 1, true);
        parity(&below, &expected);
        assert_ne!(below.pixels(), start.pixels());
        drop(below);
        assert_eq!(meter.budget.snapshot().unwrap().used, 0);
        // Resident two-layer frame: a half-alpha layer with transparent pixels over a
        // backdrop is one upload pair, one dispatch, one submission and one readback.
        let layers = [
            FrameLayer {
                image: &other,
                opacity: 1.0,
                mode: Normal,
            },
            FrameLayer {
                image: &input,
                opacity: 0.5,
                mode: Normal,
            },
        ];
        let frame = |layers: &[FrameLayer<'_>], hooks: &dyn ExecutionHooks| {
            composite_layers(w, h, layers, gpu.as_ref(), hooks)
        };
        let expected = reference_layers(w, h, &layers, &Hooks).unwrap();
        meter.take();
        let before = gpu.work();
        let actual = frame(&layers, &meter).unwrap();
        let work = gpu.work();
        resources(meter.take(), bytes, 1, true);
        parity(&actual, &expected);
        assert_eq!(
            [
                work.image_uploads - before.image_uploads,
                work.dispatches - before.dispatches,
                work.submissions - before.submissions,
                work.readbacks - before.readbacks,
            ],
            [2, 1, 1, 1]
        );
        assert_eq!(meter.budget.snapshot().unwrap().used, bytes);
        drop(actual);
        // Three layers or a non-Normal mode decline before GPU work and equal the CPU path.
        let third = FrameLayer {
            image: &start,
            opacity: 0.3,
            mode: Normal,
        };
        let multiply = FrameLayer {
            mode: project::BlendMode::Multiply,
            ..layers[1]
        };
        for plan in [vec![layers[0], layers[1], third], vec![layers[0], multiply]] {
            let before = gpu.work();
            let out = frame(&plan, &meter).unwrap();
            assert_eq!(gpu.work(), before, "unsupported plan did no GPU work");
            assert_eq!(
                out.pixels(),
                reference_layers(w, h, &plan, &Hooks).unwrap().pixels()
            );
        }
        meter.take();
        // Cancellation and byte refusal propagate unpublished, with no GPU work.
        let before = gpu.work();
        meter.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(frame(&layers, &meter), Err(Error::Cancelled)));
        meter.cancel.store(false, Ordering::SeqCst);
        let tight = Meter::new(1);
        assert!(matches!(frame(&layers, &tight), Err(Error::Budget { .. })));
        assert_eq!(tight.budget.snapshot().unwrap().used, 0);
        assert_eq!(gpu.work(), before);
        meter.take();
        let bad = Image::new(w + 1, h, [0.0; 4], &Hooks).unwrap();
        let valid = || EffectInputs {
            input: &input,
            below: &other,
            coverage: Some(&zero),
        };
        for inputs in [
            EffectInputs {
                input: &bad,
                ..valid()
            },
            EffectInputs {
                below: &bad,
                ..valid()
            },
            EffectInputs {
                coverage: Some(&bad),
                ..valid()
            },
        ] {
            assert!(matches!(
                gpu.apply(&start, &grade, inputs, &meter),
                Err(Error::Invalid(_))
            ));
            assert!(meter.take().is_empty());
        }
        let tight = Meter::new(1);
        let inputs = || EffectInputs {
            input: &input,
            below: &other,
            coverage: None,
        };
        assert!(matches!(
            gpu.apply(&start, &grade, inputs(), &tight),
            Err(Error::Budget { .. })
        ));
        assert_eq!(tight.take().len(), 1);
        assert_eq!(tight.budget.snapshot().unwrap().used, 0);
        let mut backdrop =
            Image::from_working_pixels(w, h, other.pixels().iter().copied(), &Hooks).unwrap();
        assert!(matches!(
            gpu.composite(&mut backdrop, &start, 0.7, Normal, &tight),
            Err(Error::Budget { .. })
        ));
        assert_eq!(backdrop.pixels(), other.pixels());
        assert_eq!(tight.take().len(), 1);
        meter.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(
            gpu.apply(&start, &grade, inputs(), &meter),
            Err(Error::Cancelled)
        ));
        assert!(matches!(
            gpu.composite(&mut backdrop, &start, 0.7, Normal, &meter),
            Err(Error::Cancelled)
        ));
        assert_eq!(backdrop.pixels(), other.pixels());
        assert!(meter.take().is_empty());
        meter.cancel.store(false, Ordering::SeqCst);
        let blend = EffectPass::Pixel {
            entry_point: "blend",
            uniforms: vec![Uniform::Float(0.7), Uniform::Float(0.0)],
        };
        assert!(matches!(
            gpu.apply(&start, &[blend], inputs(), &meter),
            Err(Error::Unsupported(_))
        ));
        assert!(meter.take().is_empty());
    }
    // Pressure at the 768 MiB budget: 1080p fits its resident working set; 4K is refused
    // by byte admission before any native buffer or work, and nothing leaks.
    let pressure = Meter::new(768 << 20);
    for (w, h, fits) in [(1920, 1080, true), (3840, 2160, false)] {
        let bytes = Image::bytes(w, h).unwrap();
        let base = Image::new(w, h, [0.1, 0.2, 0.4, 0.5], &pressure).unwrap();
        let top = Image::new(w, h, [0.2, 0.1, 0.3, 0.4], &pressure).unwrap();
        let layers = [
            FrameLayer {
                image: &base,
                opacity: 1.0,
                mode: Normal,
            },
            FrameLayer {
                image: &top,
                opacity: 0.5,
                mode: Normal,
            },
        ];
        let before = gpu.work();
        let result = composite_layers(w, h, &layers, gpu.as_ref(), &pressure);
        if fits {
            let actual = result.unwrap();
            parity(&actual, &reference_layers(w, h, &layers, &Hooks).unwrap());
            assert_eq!(pressure.budget.snapshot().unwrap().used, 3 * bytes);
            assert_eq!(gpu.work().submissions - before.submissions, 1);
        } else {
            assert!(matches!(result, Err(Error::Budget { .. })));
            assert_eq!(gpu.work(), before, "refused before native work");
            assert_eq!(pressure.budget.snapshot().unwrap().used, 2 * bytes);
        }
        drop((base, top));
        assert_eq!(pressure.budget.snapshot().unwrap().used, 0);
    }
}

#[test]
#[ignore = "native reduce-noise frame time at 1080p and 4K; run explicitly on hardware"]
fn reduce_noise_native_frame_time_1080p_and_4k() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("measurement requires a native adapter");
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    eprintln!("NOISE_COST adapter={:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = pollster::block_on(vibecut::services::gpu::effects::GpuEffects::new(
        device, queue,
    ))
    .unwrap();
    let passes = reduce_noise_passes();
    for (width, height) in [(1920_u32, 1080_u32), (3840, 2160)] {
        let input = Image::from_working_pixels(
            width,
            height,
            (0..width * height).map(|i| {
                let v = ((i.wrapping_mul(2_654_435_761) >> 20) % 64) as f32 / 255.0 + 0.3;
                [v, v, v, 1.0]
            }),
            &Hooks,
        )
        .unwrap();
        let inputs = || EffectInputs {
            input: &input,
            coverage: None,
            below: &input,
        };
        let expected = CpuEffects.apply(&input, &passes, inputs(), &Hooks).unwrap();
        // Two warm-ups, then seven timed frames; every result is checked against the CPU.
        let mut times = Vec::new();
        for run in 0..9 {
            let start = std::time::Instant::now();
            let output = gpu.apply(&input, &passes, inputs(), &Hooks).unwrap();
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            let error = max_error(&expected, &output);
            assert!(error <= 1e-4, "reduce_noise {width}x{height} error={error}");
            if run >= 2 {
                times.push(ms);
            }
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "NOISE_COST reduce_noise_gpu median={:.3} min={:.3} max={:.3} ms/frame {width}x{height}",
            times[times.len() / 2],
            times[0],
            times[times.len() - 1]
        );
    }
}
