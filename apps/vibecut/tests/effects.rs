#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use eframe::wgpu;
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
#[test]
fn gpu_matches_cpu_working_pixels_at_both_scales() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            assert!(
                std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
                "required GPU unavailable: {error}"
            );
            eprintln!("SKIP: no GPU adapter: {error}");
            return;
        }
    };
    eprintln!("GPU parity adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let gpu = pollster::block_on(vibecut::effects::GpuEffects::new(device, queue)).unwrap();
    let pair: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/render/tests/fixtures/blend-linear.json"
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
        for intensity in [0.0, 15.0, 100.0] {
            let passes = blur_passes(intensity, w, h).unwrap();
            let inputs = || EffectInputs {
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
}
