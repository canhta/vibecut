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

struct Entry {
    job: Option<JobId>,
    cancel: Option<CancelToken>,
    requested: Option<EffectInstance>,
    status: Status,
}
enum Status {
    Pending,
    Ready(Arc<egui::ColorImage>),
    Failed(String),
}

/// One cache per editor. Jobs start only when the effects panel first needs a
/// tile; retry replaces that kind's static request through the same contract.
pub struct EffectPreviews {
    runner: JobRunner,
    cache: EffectThumbnails,
    source: Arc<render::Image>,
    entries: BTreeMap<String, Entry>,
    deferred: VecDeque<String>,
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
            entries: BTreeMap::new(),
            deferred: VecDeque::new(),
        })
    }

    /// Headless panel previews use the reference CPU compositor. Native startup
    /// replaces it through `Editor::set_effects` with the playback GPU adapter.
    pub fn cpu() -> Result<Self> {
        let budget = ByteBudget::new(PREVIEW_BUDGET)?;
        let cache = EffectThumbnails::new(Arc::new(render::effects::CpuEffects), budget.clone());
        Self::new(cache, budget)
    }

    /// Request one preview per kind. Only queue saturation defers admission;
    /// every other refusal remains an explicit failure tile.
    pub fn ensure(&mut self, kind: &str, effect: &EffectInstance) {
        if self.entries.contains_key(kind) {
            return;
        }
        let entry = self.request(effect.clone());
        if entry.requested.is_some() {
            self.deferred.push_back(kind.to_owned());
        }
        self.entries.insert(kind.to_owned(), entry);
    }

    fn request(&mut self, effect: EffectInstance) -> Entry {
        match self
            .cache
            .submit(&mut self.runner, self.source.clone(), effect.clone())
        {
            Ok((job, cancel)) => Entry {
                job: Some(job),
                cancel: Some(cancel),
                requested: None,
                status: Status::Pending,
            },
            Err(engine::EditError::QueueFull) => Entry {
                job: None,
                cancel: None,
                requested: Some(effect),
                status: Status::Pending,
            },
            Err(error) => Entry {
                job: None,
                cancel: None,
                requested: None,
                status: Status::Failed(error.to_string()),
            },
        }
    }

    /// Submit again after a failure. The previous job, if any, is cancelled
    /// before the engine cache/runner sees another request.
    pub fn retry(&mut self, kind: &str, effect: &EffectInstance) {
        self.deferred.retain(|pending| pending != kind);
        if let Some(entry) = self.entries.remove(kind)
            && let Some(cancel) = entry.cancel
        {
            cancel.cancel();
        }
        self.ensure(kind, effect);
    }

    /// Drain submitted jobs before admitting deferred tile requests.
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
                entry.status = Status::Failed(engine::EditError::JobNotRetained.to_string());
                entry.job = None;
                entry.cancel = None;
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
        // Consume outputs before filling the bounded runner again. FIFO keeps
        // a retry behind existing requests, and a permanent refusal moves on.
        while let Some(kind) = self.deferred.pop_front() {
            let Some(effect) = self
                .entries
                .get_mut(&kind)
                .and_then(|entry| entry.requested.take())
            else {
                continue;
            };
            let entry = self.request(effect);
            let deferred = entry.requested.is_some();
            self.entries.insert(kind.clone(), entry);
            if deferred {
                self.deferred.push_front(kind);
                break;
            }
        }
        Ok(())
    }

    /// Pump the engine runner until every requested tile is terminal or the
    /// timeout passes; this blocks the caller and is meant for audits that
    /// capture a settled panel. The engine runner stays the only renderer.
    pub fn settle(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while self
            .entries
            .values()
            .any(|entry| matches!(entry.status, Status::Pending))
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
