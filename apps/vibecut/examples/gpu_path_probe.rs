//! Manual measurement probe for the GPU effect path: transfer, pass and 60 s
//! export cost on the host adapter, through the production `GpuEffects` and
//! renderer. Not part of any test run; it prints receipt lines (`BENCH`,
//! `GPU_COUNTS`, `PARITY`, `BUDGET`, `PEAK_WORKING_SET`) for the 353/354 reports.
//!
//! `cargo xtask build -- cargo run --release -p vibecut --example gpu_path_probe -- <mode>`
//!   transfer  per-frame upload/readback and pass timing at 1080p and 4K
//!   export    60 s 1080p timeline with a three-effect stack: CPU vs lent GPU
//!   frames    render-only cost of the same timeline, CPU vs GPU
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use eframe::wgpu;
use render::{effects::*, *};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use vibecut::services::gpu::effects::GpuEffects;
use wgpu::util::DeviceExt;

/// Mirrors engine's `JobContext` hooks: eight pixel workers, scoped threads,
/// no global pool. Byte leases are not accounted here because the probe measures
/// time, not the aggregate budget.
struct Hooks {
    parallel: usize,
    budget: Option<engine::ByteBudget>,
    cancel: AtomicBool,
    progress: Mutex<Instant>,
    peak: AtomicU64,
}
impl Default for Hooks {
    fn default() -> Self {
        Self {
            parallel: std::thread::available_parallelism().map_or(1, |n| n.get().min(8)),
            budget: None,
            cancel: AtomicBool::new(false),
            progress: Mutex::new(Instant::now()),
            peak: AtomicU64::new(0),
        }
    }
}
struct Lease;
impl Reservation for Lease {}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn parallelism(&self) -> usize {
        self.parallel
    }
    fn run_tasks(&self, tasks: &mut [&mut (dyn FnMut() -> Result<()> + Send)]) -> Result<()> {
        if tasks.is_empty() {
            return Ok(());
        }
        const STACK: usize = 512 << 10;
        let _stacks = self.reserve(
            MemoryKind::Compositor,
            (tasks.len().saturating_sub(1) * STACK) as u64,
        )?;
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            let Some((local, remote)) = tasks.split_last_mut() else {
                return Ok(());
            };
            for task in remote {
                handles.push(
                    std::thread::Builder::new()
                        .name("probe-pixel".into())
                        .stack_size(STACK)
                        .spawn_scoped(scope, move || task())
                        .map_err(|_| Error::Invalid("probe thread"))?,
                );
            }
            let mut result = local();
            for handle in handles {
                let joined = handle.join().unwrap_or(Err(Error::WorkerPanicked));
                if result.is_ok() {
                    result = joined;
                }
            }
            result
        })
    }
    fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
        if self.cancelled() {
            return Err(Error::Cancelled);
        }
        match &self.budget {
            Some(budget) => {
                let result = engine::ByteBudgetHooks::new(budget, None).reserve(kind, bytes);
                self.peak.fetch_max(budget.snapshot().unwrap().used, Ordering::Relaxed);
                result
            },
            None => Ok(Box::new(Lease)),
        }
    }
    fn progress(&self, stage: Stage, fraction: f32) -> Result<()> {
        let mut last = self.progress.lock().unwrap();
        if last.elapsed() >= Duration::from_secs(10) {
            println!("PROGRESS stage={stage:?} fraction={fraction:.4}");
            *last = Instant::now();
        }
        Ok(())
    }
}
impl Hooks {
    fn budgeted() -> Self {
        Self {
            budget: Some(engine::ByteBudget::new(768 << 20).unwrap()),
            ..Self::default()
        }
    }
    fn drained(&self) {
        if let Some(budget) = &self.budget {
            let used = budget.snapshot().unwrap().used;
            println!("BUDGET limit={} retained_bytes={used} observed_logical_peak={}", 768 << 20, self.peak.load(Ordering::Relaxed));
            assert_eq!(used, 0);
        }
    }
}

// Counts only actual production adapter invocations, including per-run refusals.
struct Counted {
    gpu: GpuEffects,
    apply: [AtomicU64; 4],
    composite: [AtomicU64; 4],
    unsupported_queries: AtomicU64,
    frames: [AtomicU64; 4],
    idle: AtomicU64,
}
impl Counted {
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Arc<Self> {
        Arc::new(Self {
            gpu: pollster::block_on(GpuEffects::new(device, queue)).unwrap(),
            apply: Default::default(),
            composite: Default::default(),
            unsupported_queries: AtomicU64::new(0),
            frames: Default::default(),
            idle: AtomicU64::new(0),
        })
    }
    fn report(&self, label: &str) {
        let frames = self.frames.each_ref().map(|n| n.swap(0, Ordering::Relaxed));
        println!("GPU_SCOPES {label} outcomes={frames:?} idle_calls={}", self.idle.swap(0, Ordering::Relaxed));
        let a = self.apply.each_ref().map(|n| n.swap(0, Ordering::Relaxed));
        let c = self
            .composite
            .each_ref()
            .map(|n| n.swap(0, Ordering::Relaxed));
        println!(
            "GPU_COUNTS {label} apply_ok={} apply_unsupported={} apply_budget={} apply_other={} composite_ok={} composite_unsupported={} composite_budget={} composite_other={} unsupported_support_queries={}",
            a[0],
            a[1],
            a[2],
            a[3],
            c[0],
            c[1],
            c[2],
            c[3],
            self.unsupported_queries.swap(0, Ordering::Relaxed)
        );
    }
    fn record<T>(counts: &[AtomicU64; 4], result: &Result<T>) {
        let slot = match result {
            Ok(_) => 0,
            Err(Error::Unsupported(_)) => 1,
            Err(Error::Budget { .. }) => 2,
            Err(_) => 3,
        };
        counts[slot].fetch_add(1, Ordering::Relaxed);
    }
}
impl EffectCompositor for Counted {
    fn frame<'a>(&'a self, hooks: &'a dyn ExecutionHooks) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
        let result = self.gpu.frame(hooks);
        Self::record(&self.frames, &result);
        result.map(|scope| scope.map(|frame| Box::new(CountedFrame { owner: self, frame }) as Box<dyn EffectCompositor>))
    }
    fn release_idle(&self) -> Result<()> { self.gpu.release_idle() }
    fn supports(&self, entry: &str) -> bool {
        let supported = self.gpu.supports(entry);
        if !supported {
            self.unsupported_queries.fetch_add(1, Ordering::Relaxed);
        }
        supported
    }
    fn apply(
        &self,
        image: &Image,
        passes: &[EffectPass],
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        let result = self.gpu.apply(image, passes, inputs, hooks);
        Self::record(&self.apply, &result);
        result
    }
    fn composite(
        &self,
        below: &mut Image,
        image: &Image,
        opacity: f32,
        mode: project::BlendMode,
        hooks: &dyn ExecutionHooks,
    ) -> Result<()> {
        let result = self.gpu.composite(below, image, opacity, mode, hooks);
        Self::record(&self.composite, &result);
        result
    }
}

struct CountedFrame<'a> {
    owner: &'a Counted,
    frame: Box<dyn EffectCompositor + 'a>,
}
impl EffectCompositor for CountedFrame<'_> {
    fn frame<'a>(&'a self, hooks: &'a dyn ExecutionHooks) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
        self.frame.frame(hooks)
    }
    fn release_idle(&self) -> Result<()> {
        self.owner.idle.fetch_add(1, Ordering::Relaxed);
        self.frame.release_idle()
    }
    fn supports(&self, entry: &str) -> bool {
        let supported = self.frame.supports(entry);
        if !supported { self.owner.unsupported_queries.fetch_add(1, Ordering::Relaxed); }
        supported
    }
    fn apply(&self, image: &Image, passes: &[EffectPass], inputs: EffectInputs<'_>, hooks: &dyn ExecutionHooks) -> Result<Image> {
        let result = self.frame.apply(image, passes, inputs, hooks);
        Counted::record(&self.owner.apply, &result);
        result
    }
    fn composite(&self, below: &mut Image, image: &Image, opacity: f32, mode: project::BlendMode, hooks: &dyn ExecutionHooks) -> Result<()> {
        let result = self.frame.composite(below, image, opacity, mode, hooks);
        Counted::record(&self.owner.composite, &result);
        result
    }
}

fn adapter() -> (wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = match std::env::var("VIBECUT_PROBE_BACKEND").as_deref() {
        Ok("metal") => wgpu::Backends::METAL,
        Ok("vulkan") => wgpu::Backends::VULKAN,
        Ok("dx12") => wgpu::Backends::DX12,
        Err(_) => wgpu::Backends::all(),
        _ => panic!("VIBECUT_PROBE_BACKEND must be metal, vulkan or dx12"),
    };
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("required native GPU unavailable: not a successful measurement");
    println!("REQUESTED_LIMITS {:?}", wgpu::Limits::default());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    println!("DEVICE_LIMITS {:?}", device.limits());
    (adapter, device, queue)
}

fn info(adapter: &wgpu::Adapter) {
    let i = adapter.get_info();
    let l = adapter.limits();
    println!(
        "ADAPTER name={:?} backend={:?} device_type={:?} driver={:?} driver_info={:?}",
        i.name, i.backend, i.device_type, i.driver, i.driver_info
    );
    println!(
        "LIMITS max_storage_buffer_binding={} max_buffer_size={} max_texture_2d={}",
        l.max_storage_buffer_binding_size, l.max_buffer_size, l.max_texture_dimension_2d
    );
}

fn image(w: u32, h: u32, hooks: &dyn ExecutionHooks) -> Image {
    Image::from_working_pixels(
        w,
        h,
        (0..(w as usize * h as usize)).map(|i| {
            let x = (i % w as usize) as f32;
            let y = (i / w as usize) as f32;
            let bars = if ((x as u32 / 37) + (y as u32 / 23)).is_multiple_of(2) {
                0.85
            } else {
                0.15
            };
            [
                bars * (0.2 + 0.8 * x / w as f32),
                bars * (0.2 + 0.8 * y / h as f32),
                bars * 0.6,
                1.0,
            ]
        }),
        hooks,
    )
    .unwrap()
}

fn item(effects: Vec<project::EffectInstance>) -> project::Item {
    project::Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects,
        animations: Default::default(),
        id: project::ItemId(1),
        source_asset: None,
        source_range: time::TimeRange::new(
            time::Tick::ZERO,
            time::FrameRate::FPS_30.tick_of(30).unwrap(),
        )
        .unwrap(),
        start: time::Tick::ZERO,
        content: project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([120, 80, 40, 255]),
        }),
    }
}

fn grade(brightness: f64, saturation: f64, id: &str) -> project::EffectInstance {
    project::effect_definition("color_grade")
        .unwrap()
        .instance(id)
        .with("brightness", project::EffectValue::Scalar(brightness))
        .unwrap()
        .with("saturation", project::EffectValue::Scalar(saturation))
        .unwrap()
}
fn key(intensity: f64, id: &str) -> project::EffectInstance {
    project::effect_definition("chroma_key")
        .unwrap()
        .instance(id)
        .with(
            "key_color",
            project::EffectValue::Color(project::Color([0, 255, 0, 255])),
        )
        .unwrap()
        .with("intensity", project::EffectValue::Scalar(intensity))
        .unwrap()
}
fn blur(intensity: f64, id: &str) -> project::EffectInstance {
    project::effect_definition("blur")
        .unwrap()
        .instance(id)
        .with("intensity", project::EffectValue::Scalar(intensity))
        .unwrap()
}

/// The three stacks the ticket names: 1, 3 and 6 real effect instances.
fn stack(n: usize) -> Vec<project::EffectInstance> {
    match n {
        0 => Vec::new(),
        1 => vec![grade(20.0, 20.0, "g1")],
        3 => vec![grade(20.0, 20.0, "g1"), key(40.0, "k1"), blur(10.0, "b1")],
        6 => vec![
            grade(20.0, 20.0, "g1"),
            key(40.0, "k1"),
            blur(10.0, "b1"),
            grade(-15.0, -30.0, "g2"),
            key(70.0, "k2"),
            blur(20.0, "b2"),
        ],
        _ => panic!("stack"),
    }
}

struct Stats {
    label: String,
    samples: Vec<Duration>,
}
impl Stats {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            samples: Vec::new(),
        }
    }
    fn push(&mut self, d: Duration) {
        self.samples.push(d);
    }
    fn median(&self) -> f64 {
        let mut v: Vec<f64> = self.samples.iter().map(|d| d.as_secs_f64() * 1e3).collect();
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    }
    fn min(&self) -> f64 {
        self.samples
            .iter()
            .map(|d| d.as_secs_f64() * 1e3)
            .fold(f64::INFINITY, f64::min)
    }
    fn max(&self) -> f64 {
        self.samples
            .iter()
            .map(|d| d.as_secs_f64() * 1e3)
            .fold(0.0, f64::max)
    }
    fn report(&self, extra: &str) {
        println!(
            "BENCH {} median_ms={:.3} min_ms={:.3} max_ms={:.3} reps={} {extra}",
            self.label,
            self.median(),
            self.min(),
            self.max(),
            self.samples.len()
        );
    }
}

fn time<T>(stats: &mut Stats, mut run: impl FnMut() -> T) -> T {
    let start = Instant::now();
    let value = run();
    stats.push(start.elapsed());
    value
}

fn transfer() {
    let hooks = Hooks::default();
    let (adapter, device, queue) = adapter();
    info(&adapter);
    let gpu = Counted::new(device, queue);
    let reps = 7;
    let warm = 2;
    // The adapter's own CPU-side pixel marshalling, isolated from the device.
    for (w, h) in [(1920u32, 1080u32), (3840, 2160)] {
        let img = image(w, h, &hooks);
        let bytes = Image::bytes(w, h).unwrap();
        let mut pack = Stats::new(format!("marshal_pack {w}x{h}"));
        let mut unpack = Stats::new(format!("marshal_unpack {w}x{h}"));
        for i in 0..(warm + reps) {
            let start = Instant::now();
            let mut data = Vec::with_capacity(bytes as usize);
            for pixel in img.pixels() {
                for v in pixel {
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
            if i >= warm {
                pack.push(start.elapsed());
            }
            let start = Instant::now();
            let mut sum = 0.0f32;
            for chunk in data.chunks_exact(16) {
                for k in 0..4 {
                    let at = k * 4;
                    sum += f32::from_le_bytes([
                        chunk[at],
                        chunk[at + 1],
                        chunk[at + 2],
                        chunk[at + 3],
                    ]);
                }
            }
            if sum.is_nan() {
                panic!("marshal");
            }
            if i >= warm {
                unpack.push(start.elapsed());
            }
        }
        pack.report(&format!("bytes={bytes}"));
        unpack.report(&format!("bytes={bytes}"));
    }
    for (w, h) in [(640u32, 360u32), (1920, 1080), (3840, 2160)] {
        let img = image(w, h, &hooks);
        let bytes = Image::bytes(w, h).unwrap();
        let cpu = CpuEffects;
        for n in [1usize, 3, 6] {
            let item = item(stack(n));
            let passes: Vec<_> = item
                .effects
                .iter()
                .flat_map(|effect| {
                    evaluate_effect(&item.animations, effect, time::Tick::ZERO, EffectGeometry::new([w, h], [1.0, 1.0]).unwrap(), true, &Default::default()).unwrap()
                })
                .collect();
            let inputs = || EffectInputs {
                input: &img,
                coverage: None,
                below: &img,
            };
            let mut transfer_only = Stats::new(format!("gpu_empty_passes {w}x{h} effects={n}"));
            let mut cpu_stats = Stats::new(format!("cpu_apply {w}x{h} effects={n}"));
            let mut gpu_stats = Stats::new(format!("gpu_apply {w}x{h} effects={n}"));
            let mut cpu_comp = Stats::new(format!("cpu_composite {w}x{h}"));
            let mut gpu_comp = Stats::new(format!("gpu_composite {w}x{h}"));
            for i in 0..(warm + reps) {
                let empty = time(&mut transfer_only, || {
                    gpu.apply(&img, &[], inputs(), &hooks).unwrap()
                });
                let _ = empty;
                let out = time(&mut cpu_stats, || {
                    cpu.apply(&img, &passes, inputs(), &hooks).unwrap()
                });
                drop(out);
                let out = time(&mut gpu_stats, || {
                    gpu.apply(&img, &passes, inputs(), &hooks).unwrap()
                });
                drop(out);
                let clone = || {
                    Image::from_working_pixels(w, h, img.pixels().iter().copied(), &hooks).unwrap()
                };
                let mut a = clone();
                let mut b = clone();
                time(&mut cpu_comp, || {
                    cpu.composite(&mut a, &img, 0.8, project::BlendMode::Normal, &hooks)
                        .unwrap()
                });
                time(&mut gpu_comp, || {
                    gpu.composite(&mut b, &img, 0.8, project::BlendMode::Normal, &hooks)
                        .unwrap()
                });
                if i < warm {
                    transfer_only.samples.clear();
                    cpu_stats.samples.clear();
                    gpu_stats.samples.clear();
                    cpu_comp.samples.clear();
                    gpu_comp.samples.clear();
                }
            }
            // Per-frame transfer share and marginal per-pass cost.
            let empty_ms = transfer_only.median();
            let gpu_ms = gpu_stats.median();
            let cpu_ms = cpu_stats.median();
            println!(
                "SUMMARY {w}x{h} effects={n} passes={passes} image_mb={mb:.2} gpu_total_ms={gpu_ms:.3} gpu_empty_ms={empty_ms:.3} transfer_share={share:.1}% marginal_pass_ms={marginal:.3} cpu_total_ms={cpu_total:.3} speedup={speedup:.2}x cpu_comp_ms={cpu_comp:.3} gpu_comp_ms={gpu_comp:.3}",
                passes = passes.len(),
                mb = bytes as f64 / (1024.0 * 1024.0),
                share = if gpu_ms > 0.0 {
                    empty_ms / gpu_ms * 100.0
                } else {
                    0.0
                },
                marginal = if passes.is_empty() {
                    0.0
                } else {
                    (gpu_ms - empty_ms) / passes.len() as f64
                },
                cpu_total = cpu_ms,
                speedup = cpu_ms / gpu_ms,
                cpu_comp = cpu_comp.median(),
                gpu_comp = gpu_comp.median(),
            );
            transfer_only.report(&format!("passes=0"));
            cpu_stats.report(&format!("passes={}", passes.len()));
            gpu_stats.report(&format!("passes={}", passes.len()));
            cpu_comp.report("");
            gpu_comp.report("");
            gpu.report(&format!("transfer {w}x{h} effects={n} warm=2 measured=7"));
        }
    }
    println!("PEAK_WORKING_SET {}", peak());
}

fn project_at(path: &std::path::Path, effects: Vec<project::EffectInstance>) -> project::Project {
    let asset = media::SourceAsset::import(media::AssetId(1), path.to_owned()).unwrap();
    let mut d = project::Project::empty(project::ProjectId(0), "gpu-probe".into())
        .unwrap()
        .into_data();
    let s = asset
        .probe()
        .streams
        .iter()
        .find_map(|s| s.video.as_ref())
        .unwrap();
    d.canvas.width = s.width;
    d.canvas.height = s.height;
    d.canvas.fps = asset
        .probe()
        .streams
        .iter()
        .find_map(|s| s.frame_rate)
        .unwrap();
    let duration = asset.probe().duration.unwrap();
    d.assets.push(asset);
    d.scene_mut().timeline.main.items.push(project::Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects,
        animations: Default::default(),
        id: project::ItemId(0),
        source_asset: Some(media::AssetId(1)),
        source_range: time::TimeRange::new(time::Tick::ZERO, duration).unwrap(),
        start: time::Tick::ZERO,
        content: project::ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
            fit: Default::default(),
            cleanup: Default::default(),
            background: Default::default(),
            reduce_noise: false,
            stabilize: None,
            background_removal: None,
        }),
    });
    project::Project::new(d).unwrap()
}

fn fixture() -> PathBuf {
    let dir = testkit::fixture::dir("gpu-path");
    testkit::oracle::generate(
        &dir,
        "gpu path fixture",
        "source-1080p60s.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30:duration=60",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ],
    )
    .unwrap()
}

fn settings(
    project: &project::Project,
    path: &std::path::Path,
    effects: Option<Arc<dyn EffectCompositor>>,
) -> ExportSettings {
    let mut s = ExportSettings::from_project(project);
    s.mode = ExportMode::Encode;
    s.overwrite = project::atomic::Overwrite::Refuse;
    s.include_audio = false;
    s.render.effects = effects;
    s.accept_keyframe_cuts = false;
    let _ = path;
    s
}

fn export_run(
    label: &str,
    project: &project::Project,
    effects: Option<Arc<dyn EffectCompositor>>,
    tag: &str,
) -> Result<()> {
    let dir = testkit::fixture::dir("gpu-path");
    let out = dir.join(format!("export-{tag}.mp4"));
    assert!(!out.exists());
    let s = settings(project, &out, effects);
    let hooks = Hooks::budgeted();
    let range = time::TimeRange::new(time::Tick::ZERO, project.duration()).unwrap();
    println!(
        "EXPORT_START {label} frames=1800 warmup=0 repetitions=1 encoder={:?} quality={} bit_rate={} budget={}",
        s.encoder,
        s.quality,
        s.bit_rate,
        768 << 20
    );
    let start = Instant::now();
    let receipt = match render::export(project, range, &s, &out, &hooks) {
        Ok(receipt) => receipt,
        Err(error) => {
            println!(
                "EXPORT_FAILED {label} elapsed_s={:.3} error={error:?} published={}",
                start.elapsed().as_secs_f64(),
                out.exists()
            );
            assert!(!out.exists());
            hooks.drained();
            return Err(error);
        }
    };
    let elapsed = start.elapsed();
    println!(
        "EXPORT {label} elapsed_s={:.3} median_ms={:.3} min_ms={:.3} max_ms={:.3} frames={} fps={:.2} bytes={} path={}",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1e3,
        elapsed.as_secs_f64() * 1e3,
        elapsed.as_secs_f64() * 1e3,
        receipt.video_frames,
        receipt.video_frames as f64 / elapsed.as_secs_f64(),
        std::fs::metadata(&out).unwrap().len(),
        out.display()
    );
    assert_eq!(receipt.video_frames, 1800);
    assert!(!receipt.stream_copied);
    assert_eq!(testkit::oracle::decoded_frames(&out, "v:0"), 1800);
    testkit::oracle::approx(
        testkit::oracle::duration(&out),
        60.0,
        1.0 / 30.0,
        "export duration",
    );
    assert!(testkit::oracle::decodes(&out));
    println!(
        "EXPORT_ORACLE {label} decoded_frames=1800 duration=60 tolerance_s=0.033333333 full_decode=ok peak_process_bytes={}",
        peak()
    );
    hooks.drained();
    Ok(())
}

fn parity(a: &Image, b: &Image) -> (f32, usize) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let mut max = 0.0f32;
    let mut failed = 0;
    for (a, b) in a.pixels().iter().zip(b.pixels()) {
        for (a, b) in a.iter().zip(b) {
            assert!(a.is_finite() && b.is_finite());
            let d = (a - b).abs();
            max = max.max(d);
            failed += usize::from(d > 2e-5);
        }
    }
    (max, failed)
}

fn frames() {
    let hooks = Hooks::budgeted();
    let (adapter, device, queue) = adapter();
    info(&adapter);
    let gpu = Counted::new(device, queue);
    let path = fixture();
    let ticks: Vec<_> = (0..60)
        .map(|n| time::FrameRate::FPS_30.tick_of(n).unwrap())
        .collect();
    for n in [0usize, 3] {
        let p = project_at(&path, stack(n));
        for (label, adapter) in [
            ("cpu", None),
            ("gpu", Some(gpu.clone() as Arc<dyn EffectCompositor>)),
        ] {
            let mut stats = Stats::new(format!(
                "frames effects={n} adapter={label} frames_per_rep=60"
            ));
            for rep in 0..3 {
                let mut settings = RenderSettings::from_project(&p);
                settings.effects = adapter.clone();
                let mut renderer = Renderer::new(&p, settings).unwrap();
                let mut sources = FileSources::default();
                let start = Instant::now();
                for &t in &ticks {
                    drop(renderer.render_frame(t, &mut sources, &hooks).unwrap());
                }
                stats.push(start.elapsed());
                println!(
                    "FRAME_REP effects={n} adapter={label} rep={rep} elapsed_ms={:.3}",
                    start.elapsed().as_secs_f64() * 1e3
                );
            }
            stats.report(&format!("warmup=0 fps={:.3}", 60000.0 / stats.median()));
            gpu.report(&format!("frames effects={n} adapter={label}"));
        }
    }
    let p = project_at(&path, stack(3));
    let mut settings = RenderSettings::from_project(&p);
    let mut cpu = Renderer::new(&p, settings.clone()).unwrap();
    settings.effects = Some(gpu.clone());
    let mut device = Renderer::new(&p, settings).unwrap();
    let (mut sa, mut sb) = (FileSources::default(), FileSources::default());
    let (mut max, mut failed) = (0.0f32, 0usize);
    for &t in ticks.iter().step_by(10) {
        let a = cpu.render_frame(t, &mut sa, &hooks).unwrap();
        let b = device.render_frame(t, &mut sb, &hooks).unwrap();
        let (m, f) = parity(&a, &b);
        max = max.max(m);
        failed += f;
        println!("PARITY_FRAME tick={} max_abs={m:e} over_2e5={f}", t.0);
    }
    println!(
        "PARITY ticks=6 channels={} max_abs={max:e} over_tolerance={failed} tolerance=2e-5 cpu_is_reference=true",
        6usize * 1920 * 1080 * 4
    );
    gpu.report("frame_parity");
    assert_eq!(failed, 0, "GPU disagrees: CPU wins");
    drop((cpu, device, sa, sb));
    safety(&gpu);
    hooks.drained();
    println!("PEAK_WORKING_SET {}", peak());
}

fn safety(gpu: &Counted) {
    let hooks = Hooks::budgeted();
    let img = image(16, 16, &hooks);
    let other = image(8, 8, &hooks);
    let inputs = || EffectInputs {
        input: &img,
        below: &img,
        coverage: None,
    };
    let passes = grade_passes(20.0, 0.0, 20.0, 0.0).unwrap();
    assert!(matches!(
        gpu.apply(
            &img,
            &passes,
            EffectInputs {
                input: &other,
                ..inputs()
            },
            &hooks
        ),
        Err(Error::Invalid(_))
    ));
    let budget = engine::ByteBudget::new(1).unwrap();
    let tiny = engine::ByteBudgetHooks::new(&budget, None);
    assert!(matches!(
        gpu.apply(&img, &passes, inputs(), &tiny),
        Err(Error::Budget { .. })
    ));
    hooks.cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        gpu.apply(&img, &passes, inputs(), &hooks),
        Err(Error::Cancelled)
    ));
    hooks.cancel.store(false, Ordering::Relaxed);
    let unknown = EffectPass::Pixel {
        entry_point: "probe_unknown",
        uniforms: Vec::new(),
    };
    assert!(!gpu.supports(unknown.entry_point()));
    assert!(matches!(
        gpu.apply(&img, &[unknown], inputs(), &hooks),
        Err(Error::Unsupported(_))
    ));
    println!(
        "SAFETY mismatched_input=Invalid tiny_budget=Budget cancelled=Cancelled unknown_shader=Unsupported"
    );
    gpu.report("safety_expected_refusals");
}

fn export() {
    let (adapter, device, queue) = adapter();
    info(&adapter);
    let gpu = Counted::new(device, queue);
    let path = fixture();
    let base = project_at(&path, Vec::new());
    println!(
        "TIMELINE source={} duration_s={:.1} canvas={}x{} fps={:?} items=1 effects=3",
        path.display(),
        base.duration().seconds(),
        base.data().canvas.width,
        base.data().canvas.height,
        base.data().canvas.fps
    );
    export_run("1080p_60s_no_effects_cpu", &base, None, "no-effects").unwrap();
    gpu.report("export_cpu_floor");
    let p = project_at(&path, stack(3));
    export_run("1080p_60s_3effects_cpu", &p, None, "cpu").unwrap();
    gpu.report("export_cpu_3effects");
    let outcome = export_run("1080p_60s_3effects_gpu", &p, Some(gpu.clone()), "gpu");
    gpu.report("export_gpu_3effects");
    // A measured budget refusal is a negative research result, never a GPU rate.
    assert!(matches!(outcome, Ok(()) | Err(Error::Budget { .. })));
    let hooks = Hooks::budgeted();
    hooks.cancel.store(true, Ordering::Relaxed);
    let cancelled = testkit::fixture::dir("gpu-path").join("cancelled.mp4");
    let s = settings(&p, &cancelled, None);
    let range = time::TimeRange::new(time::Tick::ZERO, p.duration()).unwrap();
    assert!(matches!(
        render::export(&p, range, &s, &cancelled, &hooks),
        Err(Error::Cancelled)
    ));
    assert!(!cancelled.exists());
    hooks.cancel.store(false, Ordering::Relaxed);
    let mut bad = s;
    bad.render.width = 1919;
    let invalid = testkit::fixture::dir("gpu-path").join("invalid.mp4");
    assert!(matches!(
        render::export(&p, range, &bad, &invalid, &hooks),
        Err(Error::Invalid(_))
    ));
    assert!(!invalid.exists());
    println!(
        "EXPORT_SAFETY precancel=Cancelled no_published_output=true odd_width=Invalid no_published_output=true"
    );
    hooks.drained();
    println!("PEAK_WORKING_SET {}", peak());
}

/// Peak resident bytes. Windows asks PowerShell for `PeakWorkingSet64`. macOS has no
/// safe in-process peak reader (`unsafe` is banned here), so a sampler thread polls
/// `ps -o rss=` every 100 ms and keeps the maximum: a lower bound that can miss a
/// spike shorter than the interval. `/usr/bin/time -l` reports the exact figure.
static SAMPLED_PEAK: AtomicU64 = AtomicU64::new(0);

fn rss_now() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .map_or(0, |kib| kib * 1024)
}

fn start_peak_sampler() {
    if cfg!(target_os = "macos") {
        std::thread::spawn(|| loop {
            SAMPLED_PEAK.fetch_max(rss_now(), Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(100));
        });
    }
}

fn peak() -> u64 {
    if cfg!(target_os = "macos") {
        return SAMPLED_PEAK.fetch_max(rss_now(), Ordering::Relaxed).max(rss_now());
    }
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id()),
        ])
        .output();
    let Ok(output) = output else {
        return 0;
    };
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn pooled() {
    use std::sync::mpsc;
    let hooks = Hooks::default();
    let (adapter, device, queue) = adapter();
    info(&adapter);
    for (w, h) in [(640u32, 360u32), (1920, 1080), (3840, 2160)] {
        let img = image(w, h, &hooks);
        let bytes = Image::bytes(w, h).unwrap();
        let mut packed = Vec::with_capacity(bytes as usize);
        for pixel in img.pixels() {
            for v in pixel {
                packed.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut per_call = Stats::new(format!("create_buffer_init {w}x{h}"));
        for i in 0..9 {
            let start = Instant::now();
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("probe upload"),
                contents: &packed,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            });
            if i >= 2 {
                per_call.push(start.elapsed());
            }
            drop(buffer);
        }
        per_call.report(&format!("bytes={bytes}"));
        let upload = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe upload"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let work = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe work"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut pooled = Stats::new(format!("pooled_upload_copy_readback {w}x{h}"));
        for i in 0..9 {
            let start = Instant::now();
            queue.write_buffer(&upload, 0, &packed);
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&upload, 0, &work, 0, bytes);
            encoder.copy_buffer_to_buffer(&work, 0, &readback, 0, bytes);
            let index = queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            let (tx, rx) = mpsc::sync_channel(1);
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: Some(Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
            {
                let data = slice.get_mapped_range().unwrap();
                assert_eq!(&*data, packed.as_slice(), "pooled exact byte round trip");
            }
            readback.unmap();
            if i >= 2 {
                pooled.push(start.elapsed());
            }
        }
        pooled.report(&format!("bytes={bytes}"));
    }
    println!("PEAK_WORKING_SET {}", peak());
}

fn main() {
    use tracing_subscriber::prelude::*;
    start_peak_sampler();
    let log = tracing_subscriber::fmt::layer().without_time().with_ansi(false)
        .with_filter(tracing_subscriber::filter::filter_fn(|meta| meta.target() == "vibecut::effects"));
    let _log = tracing::subscriber::set_default(tracing_subscriber::registry().with(log));
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    println!(
        "SOURCE_SHA {} profile=release hooks=microbench_unbudgeted_or_frames_export_768MiB",
        String::from_utf8_lossy(&sha.stdout).trim()
    );
    println!(
        "RUN args={:?} os={} arch={} logical_cpus={}",
        std::env::args().collect::<Vec<_>>(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().unwrap()
    );
    match std::env::args().nth(1).as_deref() {
        Some("transfer") => transfer(),
        Some("frames") => frames(),
        Some("export") => export(),
        Some("pooled") => pooled(),
        _ => panic!("usage: gpu_path_probe transfer|frames|export|pooled"),
    }
}
