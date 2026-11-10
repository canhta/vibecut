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
    let gpu = std::sync::Arc::new(
        pollster::block_on(vibecut::effects::GpuEffects::new(device, queue)).unwrap(),
    );
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
        budget,
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
                source.clone(),
                project::effect_definition("blur")
                    .unwrap()
                    .instance("blur")
                    .with("intensity", project::EffectValue::Scalar(100.0))
                    .unwrap(),
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
                meta.target() == "vibecut::effects"
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
        pollster::block_on(vibecut::effects::GpuEffects::new(device, queue)).unwrap(),
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
    let first = project::Item {
        id: project::ItemId(1),
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: vec![],
        animations: Default::default(),
        effects: vec![
            project::effect_definition("color_grade")
                .unwrap()
                .instance("grade")
                .with("brightness", project::EffectValue::Scalar(30.0))
                .unwrap(),
        ],
        source_asset: None,
        start: time::Tick::ZERO,
        source_range: time::SourceRange::new(time::Tick::ZERO, duration).unwrap(),
        content: project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([100, 50, 20, 255]),
        }),
    };
    let mut second = first.clone();
    second.id = project::ItemId(2);
    second.effects.clear();
    second.source_asset = Some(media::AssetId(1));
    second.content = project::ItemParams::Video(project::VideoParams {
        crop: None,
        fades: Default::default(),
        source_audio_enabled: false,
        volume: 0.0,
        muted: false,
    });
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
        plain.extend(chroma_key_passes([0.0, 1.0, 0.0, 1.0], 40.0, 0.0).unwrap());
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
}
