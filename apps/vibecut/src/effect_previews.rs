//! Effects-panel preview admission over engine-owned jobs, accounting and cache.

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
        let source = Arc::new(render::Image::from_working_pixels(
            PREVIEW_WIDTH,
            PREVIEW_HEIGHT,
            (0..(PREVIEW_WIDTH * PREVIEW_HEIGHT) as usize).map(|index| {
                let x = (index % PREVIEW_WIDTH as usize) as f32;
                let y = (index / PREVIEW_WIDTH as usize) as f32;
                let checker = if ((x as u32 / 32) + (y as u32 / 18)).is_multiple_of(2) {
                    0.68
                } else {
                    0.24
                };
                let shade = checker + x / PREVIEW_WIDTH as f32 * 0.18;
                [shade * 0.55, shade * 0.78, shade, 1.0]
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
            geometry: render::effects::EffectGeometry::new(
                [PREVIEW_WIDTH, PREVIEW_HEIGHT],
                [1.0, 1.0],
            )?,
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
                Err(engine::EditError::QueueFull) => {
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
                None => Some(Err(engine::EditError::JobNotRetained.to_string())),
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
