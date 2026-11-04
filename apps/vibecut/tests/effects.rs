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
