//! Runs the Effects tab's tile preview jobs.

use crate::{Result, egui};
use engine::effect_thumbnails::EffectThumbnails;
use engine::jobs::{CancelToken, JobId, JobRunner, JobState, RunnerConfig, SystemClock};
use engine::{ByteBudget, ByteBudgetHooks};
use project::EffectInstance;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

/// Tile preview source edge. Smaller than the engine's 320 px admission cap so
/// the effect is legible in the assets panel without extra downsampling.
const PREVIEW_WIDTH: u32 = 320;
const PREVIEW_HEIGHT: u32 = 180;
const PREVIEW_BUDGET: u64 = 16 << 20;
/// Tiles stand in for this reference clip at [`PREVIEW_WIDTH`] scale. Spatial
/// effect parameters are authored in full-resolution clip pixels, so the tile
/// must shrink their geometry by the same ratio as the picture.
const REFERENCE_WIDTH: u32 = 1920;
const REFERENCE_HEIGHT: u32 = 1080;
/// The working size every tile renders at.
pub const PREVIEW_SIZE: [u32; 2] = [PREVIEW_WIDTH, PREVIEW_HEIGHT];
/// One preview pixel in reference-clip pixels on each axis.
pub const PREVIEW_SCALE: [f64; 2] = [
    PREVIEW_WIDTH as f64 / REFERENCE_WIDTH as f64,
    PREVIEW_HEIGHT as f64 / REFERENCE_HEIGHT as f64,
];
/// Budget for one settle of the whole library on the CPU reference. Every
/// library kind renders once, and a debug build's unoptimised kernels need far
/// longer than the release panel the person actually sees.
pub const SETTLE_BUDGET: Duration = Duration::from_secs(60);

/// The Effects tab's checker ramp.
fn checker_pixel(x: f32, y: f32) -> [f32; 4] {
    let checker = if ((x as u32 / 32) + (y as u32 / 18)).is_multiple_of(2) {
        0.68
    } else {
        0.24
    };
    let shade = checker + x / PREVIEW_WIDTH as f32 * 0.18;
    [shade * 0.55, shade * 0.78, shade, 1.0]
}

/// The Filters tab's procedural picture: a dusk sky with a sun, two ridges, a rippled lake, a
/// grey ramp down the left edge and six colour chips along the bottom, so a tile shows what a
/// look does to sky, shadows, saturated colours and neutrals. No photograph is involved.
fn filter_pixel(x: f32, y: f32) -> [f32; 4] {
    let (width, height) = (PREVIEW_WIDTH as f32, PREVIEW_HEIGHT as f32);
    let horizon = 0.56_f32;
    let ramp = width * 0.05;
    let chip_height = height * 0.12;
    let chips: [[f32; 3]; 6] = [
        [0.85, 0.12, 0.1],
        [0.95, 0.8, 0.1],
        [0.15, 0.7, 0.2],
        [0.1, 0.75, 0.8],
        [0.15, 0.25, 0.85],
        [0.8, 0.2, 0.75],
    ];
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let ridge = |u: f32, k: f32| {
        0.12 * k * (0.55 + 0.25 * (u * 7.3 + k).sin() + 0.15 * (u * 17.1 + 2.0 * k).sin())
    };
    let sky = |v: f32| {
        mix(
            [0.98, 0.62, 0.32],
            [0.16, 0.3, 0.62],
            (1.0 - v / horizon).clamp(0.0, 1.0).powf(0.8),
        )
    };
    let (u, v) = ((x + 0.5) / width, (y + 0.5) / height);
    let mut colour = if v < horizon {
        let mut c = sky(v);
        let distance = (((u - 0.68) * width / height).powi(2) + (v - 0.33).powi(2)).sqrt();
        if distance < 0.07 {
            c = [1.0, 0.93, 0.7];
        } else {
            let glow = (1.0 - (distance - 0.07) / 0.25).clamp(0.0, 1.0).powi(2) * 0.35;
            c = mix(c, [1.0, 0.85, 0.55], glow);
        }
        if horizon - v < ridge(u, 1.6) {
            c = mix([0.36, 0.4, 0.52], c, 0.2);
        }
        if horizon - v < ridge(u, 0.9) {
            c = mix(
                [0.16, 0.2, 0.12],
                [0.3, 0.28, 0.2],
                ((horizon - v) / 0.12).clamp(0.0, 1.0),
            );
        }
        c
    } else {
        let mirror = horizon - (v - horizon);
        let ripple = 0.04 * ((v * 140.0).sin() * (u * 9.0 + v * 30.0).cos());
        let reflection = sky(mirror.max(0.0));
        mix(
            mix(reflection, [0.05, 0.12, 0.22], 0.45 + (v - horizon) * 0.8),
            [0.0; 3],
            0.1 + ripple,
        )
    };
    if x < ramp {
        let grey = 1.0 - v;
        colour = [grey, grey, grey];
    }
    if y >= height - chip_height && x >= ramp {
        let index = (((x - ramp) / (width - ramp)) * 6.0)
            .floor()
            .clamp(0.0, 5.0) as usize;
        colour = chips[index];
    }
    let linear = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [linear(colour[0]), linear(colour[1]), linear(colour[2]), 1.0]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectPreviewStatus {
    Pending,
    Ready,
    Failed,
}

struct Flight {
    job: JobId,
    cancel: CancelToken,
    sequence: u64,
}
struct Entry {
    effect: EffectInstance,
    animations: project::Animations,
    job: Option<Flight>,
    desired: Option<u64>,
    image: Option<Arc<egui::ColorImage>>,
    presented: Option<u64>,
    failure: Option<String>,
    texture: Option<egui::TextureHandle>,
    texture_dirty: bool,
}
/// One cache per editor. Jobs start only when the effects panel first needs a
/// tile; retry replaces that kind's static request through the same contract.
pub struct EffectPreviews {
    runner: JobRunner,
    cache: EffectThumbnails,
    source: Arc<render::Image>,
    geometry: render::effects::EffectGeometry,
    entries: BTreeMap<String, Entry>,
    deferred: VecDeque<String>,
    visible: bool,
    last_clock: Option<Duration>,
    elapsed: Duration,
    sequence: u64,
}

impl EffectPreviews {
    pub fn new(cache: EffectThumbnails, budget: ByteBudget) -> Result<Self> {
        Self::with_picture(cache, budget, checker_pixel)
    }

    /// Previews that grade the filter picture (a dusk landscape with a grey ramp and colour
    /// chips) on the reference CPU compositor. The Filters tab owns one of these.
    pub fn filters() -> Result<Self> {
        let budget = ByteBudget::new(PREVIEW_BUDGET)?;
        let cache = EffectThumbnails::new(Arc::new(render::effects::CpuEffects), budget.clone());
        Self::with_picture(cache, budget, filter_pixel)
    }

    /// `pixel` maps a source pixel's column and row to its working-space RGBA.
    fn with_picture(
        cache: EffectThumbnails,
        budget: ByteBudget,
        pixel: fn(f32, f32) -> [f32; 4],
    ) -> Result<Self> {
        let source = Arc::new(render::Image::from_working_pixels(
            PREVIEW_WIDTH,
            PREVIEW_HEIGHT,
            (0..(PREVIEW_WIDTH * PREVIEW_HEIGHT) as usize).map(|index| {
                let x = (index % PREVIEW_WIDTH as usize) as f32;
                let y = (index / PREVIEW_WIDTH as usize) as f32;
                pixel(x, y)
            }),
            &ByteBudgetHooks::new(&budget, None),
        )?);
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 4,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(10),
                progress_interval: Duration::from_millis(10),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            runner,
            cache,
            source,
            geometry: render::effects::EffectGeometry::new(PREVIEW_SIZE, PREVIEW_SCALE)?,
            entries: BTreeMap::new(),
            deferred: VecDeque::new(),
            visible: true,
            last_clock: None,
            elapsed: Duration::ZERO,
            sequence: 0,
        })
    }

    /// Headless panel previews use the reference CPU compositor. Native startup
    /// replaces it through `Editor::set_effects` with the playback GPU adapter.
    pub fn cpu() -> Result<Self> {
        let budget = ByteBudget::new(PREVIEW_BUDGET)?;
        let cache = EffectThumbnails::new(Arc::new(render::effects::CpuEffects), budget.clone());
        Self::new(cache, budget)
    }

    /// Advance the visible two-second, 30fps demo from the editor clock only.
    /// Hidden time is excluded; a visibility change invalidates submitted work.
    pub fn update(&mut self, visible: bool, now: Duration) {
        if let Some(last) = self.last_clock {
            self.elapsed = self.elapsed.saturating_add(now.saturating_sub(last));
        }
        self.last_clock = visible.then_some(now);
        let sequence = (self.elapsed.as_nanos().saturating_mul(30) / 1_000_000_000)
            .min(u128::from(u64::MAX)) as u64;
        let changed = visible != self.visible;
        self.visible = visible;
        if changed {
            self.deferred.clear();
            for entry in self.entries.values_mut() {
                Self::cancel(entry);
            }
        }
        if visible && (changed || sequence != self.sequence) {
            for (kind, entry) in &mut self.entries {
                if entry.failure.is_none() {
                    entry.desired = Some(sequence);
                    if entry.job.is_none() && !self.deferred.contains(kind) {
                        self.deferred.push_back(kind.clone());
                    }
                }
            }
        }
        self.sequence = sequence;
    }

    /// Coalesce a kind's desired frame; changed authored input starts a generation.
    pub fn ensure(
        &mut self,
        kind: &str,
        effect: &EffectInstance,
        animations: &project::Animations,
    ) {
        if !self.visible {
            return;
        }
        if self
            .entries
            .get(kind)
            .is_some_and(|e| e.effect == *effect && e.animations == *animations)
        {
            return;
        }
        if let Some(entry) = self.entries.get_mut(kind) {
            Self::cancel(entry);
            entry.effect = effect.clone();
            entry.animations = animations.clone();
            entry.failure = None;
            entry.desired = self.visible.then_some(self.sequence);
        } else {
            self.entries.insert(
                kind.to_owned(),
                Entry {
                    effect: effect.clone(),
                    animations: animations.clone(),
                    job: None,
                    desired: Some(self.sequence),
                    image: None,
                    presented: None,
                    failure: None,
                    texture: None,
                    texture_dirty: false,
                },
            );
        }
        if let Err(error) = effect.validate() {
            if let Some(entry) = self.entries.get_mut(kind) {
                entry.failure = Some(error.to_string());
                entry.desired = None;
            }
            return;
        }
        if !self.deferred.iter().any(|pending| pending == kind) {
            self.deferred.push_back(kind.to_owned());
        }
        self.admit();
    }

    fn cancel(entry: &mut Entry) {
        if let Some(flight) = entry.job.take() {
            flight.cancel.cancel();
        }
        entry.desired = None;
        entry.presented = None;
    }

    /// Retry invalidates its old job ID; late output cannot replace the new entry.
    pub fn retry(&mut self, kind: &str, effect: &EffectInstance, animations: &project::Animations) {
        self.deferred.retain(|pending| pending != kind);
        if let Some(entry) = self.entries.get_mut(kind) {
            Self::cancel(entry);
            entry.failure = None;
            // Replace identical input too, retaining the last good image.
            entry.desired = self.visible.then_some(self.sequence);
            entry.effect = effect.clone();
            entry.animations = animations.clone();
            if let Err(error) = effect.validate() {
                entry.failure = Some(error.to_string());
                entry.desired = None;
                return;
            }
            if self.visible {
                self.deferred.push_back(kind.to_owned());
            }
            self.admit();
        } else {
            self.ensure(kind, effect, animations);
        }
    }

    fn local(sequence: u64) -> time::Tick {
        time::Tick((sequence % 60) as i64 * (time::TICKS_PER_SECOND / 30))
    }

    fn admit(&mut self) {
        if !self.visible {
            return;
        }
        while let Some(kind) = self.deferred.pop_front() {
            let Some(entry) = self.entries.get_mut(&kind) else {
                continue;
            };
            if entry.job.is_some() || entry.failure.is_some() {
                continue;
            }
            let Some(sequence) = entry.desired else {
                continue;
            };
            match self.cache.submit(
                &mut self.runner,
                self.geometry,
                self.source.clone(),
                entry.effect.clone(),
                entry.animations.clone(),
                Self::local(sequence),
            ) {
                Ok((job, cancel)) => {
                    entry.job = Some(Flight {
                        job,
                        cancel,
                        sequence,
                    });
                    entry.desired = None;
                }
                Err(engine::ArtifactError::Runtime(engine::RuntimeError::QueueFull)) => {
                    self.deferred.push_front(kind);
                    break;
                }
                Err(error) => {
                    entry.failure = Some(error.to_string());
                    entry.desired = None;
                }
            }
        }
    }

    /// Complete current generations before fair admission of their latest frames.
    pub fn tick(&mut self) -> Result<()> {
        self.runner.tick()?;
        for (kind, entry) in &mut self.entries {
            let Some(flight) = &entry.job else {
                continue;
            };
            let job = flight.job;
            let sequence = flight.sequence;
            let outcome = match self.runner.snapshot(job) {
                None => Some(Err(engine::control::Error::Runtime(
                    engine::RuntimeError::JobNotRetained,
                )
                .to_string())),
                Some(snapshot) => match snapshot.state {
                    JobState::Completed => Some(
                        self.runner
                            .take_output(job)
                            .filter(|output| {
                                output.bytes().len()
                                    == (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize
                            })
                            .map(|output| {
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [PREVIEW_WIDTH as usize, PREVIEW_HEIGHT as usize],
                                    output.bytes(),
                                )
                            })
                            .ok_or_else(|| "preview result unavailable".to_owned()),
                    ),
                    JobState::Failed | JobState::Cancelled => Some(Err(snapshot
                        .error
                        .map_or_else(|| "preview cancelled".to_owned(), |e| e.to_string()))),
                    _ => None,
                },
            };
            if let Some(outcome) = outcome {
                entry.job = None;
                match outcome {
                    Ok(image) => {
                        if entry.presented.is_none_or(|last| sequence > last) {
                            entry.image = Some(Arc::new(image));
                            entry.presented = Some(sequence);
                            entry.texture_dirty = true;
                        }
                    }
                    Err(reason) => {
                        entry.failure = Some(reason);
                        entry.desired = None;
                    }
                }
                if entry.desired == Some(sequence) {
                    entry.desired = None;
                }
                if entry.desired.is_some() && !self.deferred.contains(kind) {
                    self.deferred.push_back(kind.clone());
                }
            }
        }
        self.admit();
        Ok(())
    }

    /// Drain the frozen current target, including refreshes behind a Ready image.
    /// No editor clock is sampled inside this loop.
    pub fn settle(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while self
            .entries
            .values()
            .any(|e| e.job.is_some() || e.desired.is_some())
        {
            self.tick()?;
            if Instant::now() >= deadline {
                return Err(crate::Error::PreviewUnavailable(
                    "effect previews did not settle",
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
    pub fn status(&self, kind: &str) -> Option<EffectPreviewStatus> {
        self.entries.get(kind).map(|e| {
            if e.failure.is_some() {
                EffectPreviewStatus::Failed
            } else if e.image.is_some() {
                EffectPreviewStatus::Ready
            } else {
                EffectPreviewStatus::Pending
            }
        })
    }
    pub fn statuses(&self) -> BTreeMap<String, EffectPreviewStatus> {
        self.entries
            .keys()
            .filter_map(|kind| self.status(kind).map(|s| (kind.clone(), s)))
            .collect()
    }
    pub fn image(&self, kind: &str) -> Option<Arc<egui::ColorImage>> {
        self.entries.get(kind)?.image.clone()
    }
    pub fn failure(&self, kind: &str) -> Option<&str> {
        self.entries.get(kind)?.failure.as_deref()
    }

    /// Upload a newly presented frame once; stationary paints reuse the handle.
    pub(crate) fn texture(
        &mut self,
        ctx: &egui::Context,
        kind: &str,
    ) -> Option<egui::TextureHandle> {
        let entry = self.entries.get_mut(kind)?;
        let image = entry.image.as_ref()?;
        if entry.texture_dirty {
            if let Some(texture) = &mut entry.texture {
                texture.set((**image).clone(), egui::TextureOptions::LINEAR);
            } else {
                entry.texture = Some(ctx.load_texture(
                    format!("effects.preview.{kind}"),
                    (**image).clone(),
                    egui::TextureOptions::LINEAR,
                ));
            }
            entry.texture_dirty = false;
        }
        entry.texture.clone()
    }
}

impl Drop for EffectPreviews {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            if let Some(flight) = &entry.job {
                flight.cancel.cancel();
            }
        }
        self.runner.shutdown();
    }
}

/// A library tile must read as the effect at a glance, so previews use a
/// visible setting instead of an insertion default that can be an identity.
pub(in crate::editor) fn preview_instance(
    definition: &project::EffectDefinition,
) -> Option<EffectInstance> {
    let mut instance = definition.instance(definition.id);
    match definition.id {
        "blur" => {
            let (_, max) = definition.param("intensity")?.scalar_range()?;
            instance.set_scalar("intensity", max).ok()?;
        }
        "directional_blur" => {
            instance.set_scalar("length", 8.0).ok()?;
            instance.set_scalar("direction", 45.0).ok()?;
        }
        "sharpen" => instance.set_scalar("amount", 100.0).ok()?,
        // A soft 8x6 lattice shows both axes and a blended edge.
        "mosaic" => {
            instance.set_scalar("horizontal", 8.0).ok()?;
            instance.set_scalar("vertical", 6.0).ok()?;
            instance.set_scalar("softness", 50.0).ok()?;
        }
        "gamma_correction" => instance.set_scalar("gamma", 25.0).ok()?,
        "levels" => {
            instance.set_scalar("in_black", 40.0).ok()?;
            instance.set_scalar("in_white", 200.0).ok()?;
        }
        // Swap red and blue so the identity matrix does not read as no effect.
        "channel_mix" => {
            for (id, value) in [("rr", 0.0), ("rb", 100.0), ("br", 100.0), ("bb", 0.0)] {
                instance.set_scalar(id, value).ok()?;
            }
        }
        "color_replace" => {
            instance = instance
                .with(
                    "target",
                    project::EffectValue::Color(project::Color([0, 0, 255, 255])),
                )
                .ok()?
                .with(
                    "replace",
                    project::EffectValue::Color(project::Color([255, 0, 0, 255])),
                )
                .ok()?;
        }
        "strobe" => {
            instance = instance
                .with(
                    "color",
                    project::EffectValue::Color(project::Color([255, 210, 0, 255])),
                )
                .ok()?;
        }
        // Entries whose defaults are identity need a visible tile preview.
        "spherize" => instance.set_scalar("radius", 150.0).ok()?,
        "wave_warp" => {
            instance.set_scalar("height", 25.0).ok()?;
            instance.set_scalar("width", 60.0).ok()?;
        }
        "lens_distortion" => instance.set_scalar("curvature", 60.0).ok()?,
        "corner_pin" => {
            for (id, value) in [
                ("upper_left", [0.1, 0.1]),
                ("upper_right", [0.9, 0.1]),
                ("lower_left", [0.1, 0.9]),
                ("lower_right", [0.9, 0.9]),
            ] {
                instance = instance.with(id, project::EffectValue::Point(value)).ok()?;
            }
        }
        "basic_3d" => {
            instance.set_scalar("swivel", 30.0).ok()?;
            instance.set_scalar("tilt", -20.0).ok()?;
        }
        "camera_shake" => instance.set_scalar("amount", 60.0).ok()?,
        "color_key" => instance.set_scalar("tolerance", 120.0).ok()?,
        "luma_key" => {
            instance.set_scalar("threshold", 60.0).ok()?;
            instance.set_scalar("cutoff", 20.0).ok()?;
        }
        _ => {}
    }
    Some(instance)
}
