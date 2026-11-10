//! Effects-panel tile previews. The engine owns scheduling, accounting and the
//! bounded thumbnail cache; this module only keeps the panel's latest state.

use crate::{Result, egui};
use engine::ByteBudget;
use engine::effect_thumbnails::EffectThumbnails;
use engine::jobs::{CancelToken, JobId, JobRunner, JobState, RunnerConfig, SystemClock};
use project::EffectParams;
use render::{ExecutionHooks, MemoryKind, Reservation};
use std::{
    collections::BTreeMap,
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

struct PreviewHooks {
    budget: ByteBudget,
}
struct PreviewLease {
    _lease: engine::ByteLease,
}
impl Reservation for PreviewLease {}
impl ExecutionHooks for PreviewHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(&self, _: MemoryKind, bytes: u64) -> render::Result<Box<dyn Reservation>> {
        let lease = self
            .budget
            .reserve(engine::ByteOwner::Compositor, bytes)
            .map_err(|_| render::Error::Invalid("preview budget"))?;
        Ok(Box::new(PreviewLease { _lease: lease }))
    }
}

struct Entry {
    job: Option<JobId>,
    cancel: Option<CancelToken>,
    status: Status,
}
enum Status {
    Pending,
    Ready(Arc<egui::ColorImage>),
    Failed(String),
}

/// One cache per editor. Jobs start only when the effects panel first needs a
/// tile, and the same thumbnail contract serves retry and live refresh.
pub struct EffectPreviews {
    runner: JobRunner,
    cache: EffectThumbnails,
    source: Arc<render::Image>,
    entries: BTreeMap<String, Entry>,
}

impl EffectPreviews {
    pub fn new(cache: EffectThumbnails, budget: ByteBudget) -> Result<Self> {
        let hooks = PreviewHooks {
            budget: budget.clone(),
        };
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
            &hooks,
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
            entries: BTreeMap::new(),
        })
    }

    /// Headless panel previews use the reference CPU compositor. Native startup
    /// replaces it through `Editor::set_effects` with the playback GPU adapter.
    pub fn cpu() -> Result<Self> {
        let budget = ByteBudget::new(PREVIEW_BUDGET)?;
        let cache = EffectThumbnails::new(Arc::new(render::effects::CpuEffects), budget.clone());
        Self::new(cache, budget)
    }

    /// Admit one preview per effect kind. A refused submission is a rendered
    /// failure state, not a silent blank tile.
    pub fn ensure(&mut self, kind: &str, params: &EffectParams) {
        if self.entries.contains_key(kind) {
            return;
        }
        let entry = match self
            .cache
            .submit(&mut self.runner, self.source.clone(), params.clone())
        {
            Ok((job, cancel)) => Entry {
                job: Some(job),
                cancel: Some(cancel),
                status: Status::Pending,
            },
            Err(error) => Entry {
                job: None,
                cancel: None,
                status: Status::Failed(error.to_string()),
            },
        };
        self.entries.insert(kind.to_owned(), entry);
    }

    /// Submit again after a failure. The previous job, if any, is cancelled
    /// before the engine cache/runner sees another request.
    pub fn retry(&mut self, kind: &str, params: &EffectParams) {
        if let Some(entry) = self.entries.remove(kind)
            && let Some(cancel) = entry.cancel
        {
            cancel.cancel();
        }
        self.ensure(kind, params);
    }

    /// Pump the engine runner and retain only terminal truth the tile can show.
    pub fn tick(&mut self) -> Result<()> {
        self.runner.tick()?;
        let kinds: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.job.is_some())
            .map(|(kind, _)| kind.clone())
            .collect();
        for kind in kinds {
            let Some(entry) = self.entries.get_mut(&kind) else {
                continue;
            };
            let Some(job) = entry.job else {
                continue;
            };
            let Some(snapshot) = self.runner.snapshot(job) else {
                continue;
            };
            match snapshot.state {
                JobState::Completed => {
                    let expected = (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize;
                    entry.status = match self.runner.take_output(job) {
                        Some(output) if output.bytes().len() == expected => {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [PREVIEW_WIDTH as usize, PREVIEW_HEIGHT as usize],
                                output.bytes(),
                            );
                            Status::Ready(Arc::new(image))
                        }
                        Some(_) => Status::Failed("preview output size".into()),
                        None => Status::Failed("preview result unavailable".into()),
                    };
                    entry.job = None;
                    entry.cancel = None;
                }
                JobState::Failed | JobState::Cancelled => {
                    let reason = snapshot
                        .error
                        .as_ref()
                        .map_or_else(|| "preview cancelled".to_owned(), |error| error.to_string());
                    entry.status = Status::Failed(reason);
                    entry.job = None;
                    entry.cancel = None;
                }
                JobState::Queued
                | JobState::Running
                | JobState::Cancelling
                | JobState::Publishing
                | JobState::Backoff => {}
            }
        }
        Ok(())
    }

    /// Pump the engine runner until every submitted tile is terminal or the
    /// timeout passes; this blocks the caller and is meant for audits that
    /// capture a settled panel. The engine runner stays the only renderer.
    pub fn settle(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while self.entries.values().any(|entry| entry.job.is_some()) {
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
        self.entries.get(kind).map(|entry| match entry.status {
            Status::Pending => EffectPreviewStatus::Pending,
            Status::Ready(_) => EffectPreviewStatus::Ready,
            Status::Failed(_) => EffectPreviewStatus::Failed,
        })
    }

    pub fn statuses(&self) -> BTreeMap<String, EffectPreviewStatus> {
        self.entries
            .iter()
            .map(|(kind, entry)| {
                let status = match entry.status {
                    Status::Pending => EffectPreviewStatus::Pending,
                    Status::Ready(_) => EffectPreviewStatus::Ready,
                    Status::Failed(_) => EffectPreviewStatus::Failed,
                };
                (kind.clone(), status)
            })
            .collect()
    }

    pub fn image(&self, kind: &str) -> Option<Arc<egui::ColorImage>> {
        match &self.entries.get(kind)?.status {
            Status::Ready(image) => Some(image.clone()),
            Status::Pending | Status::Failed(_) => None,
        }
    }

    pub fn failure(&self, kind: &str) -> Option<&str> {
        match &self.entries.get(kind)?.status {
            Status::Failed(reason) => Some(reason),
            Status::Pending | Status::Ready(_) => None,
        }
    }
}

impl Drop for EffectPreviews {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            if let Some(cancel) = &entry.cancel {
                cancel.cancel();
            }
        }
        self.runner.shutdown();
    }
}

/// App-level conversion for an image-panel texture key.
pub fn preview_texture(
    ctx: &egui::Context,
    kind: &str,
    image: Arc<egui::ColorImage>,
) -> egui::TextureHandle {
    ctx.load_texture(
        format!("effects.preview.{kind}"),
        (*image).clone(),
        egui::TextureOptions::LINEAR,
    )
}
