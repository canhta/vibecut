//! Runs the Animation tab's tile preview jobs and keeps each look's strip as one texture.
use crate::editor::properties::egui;
use engine::animation_previews::{AnimationPreviews, PREVIEW_EDGE, PREVIEW_FRAMES};
use engine::jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock};
use engine::{AnimationPreset as Look, AnimationSlot as Slot, ByteBudget};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const BUDGET: u64 = 16 << 20;
/// Every look of every slot fits the runner at once: 12 In, 12 Out and 13 Combo.
const QUEUE: usize = 48;
/// Strips rasterise on the CPU in parallel so a tab fills within a few frames.

enum Entry {
    Pending(JobId),
    Ready(egui::TextureHandle),
    Failed,
}

/// One cache and runner for the panel. A strip is requested the first time its tab is
/// drawn; later draws, and reopening the panel, are cache hits with no rendering.
pub(in crate::editor) struct Previews {
    runner: JobRunner,
    cache: AnimationPreviews,
    entries: BTreeMap<(&'static str, &'static str), Entry>,
}

impl Previews {
    pub(in crate::editor) fn new() -> Option<Self> {
        let budget = ByteBudget::new(BUDGET).ok()?;
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [4, 1, 1],
                queue: QUEUE,
                retained: QUEUE,
                network_attempts: 1,
                backoff: Duration::from_millis(10),
                progress_interval: Duration::from_millis(10),
            },
            budget.clone(),
            Arc::new(SystemClock::default()),
        )
        .ok()?;
        Some(Self {
            runner,
            cache: AnimationPreviews::new(budget),
            entries: BTreeMap::new(),
        })
    }

    /// Request the looks of `slot` that have no strip yet and collect finished jobs. True
    /// while a job is still outstanding, so the caller keeps repainting.
    pub(super) fn update(&mut self, ctx: &egui::Context, slot: Slot, looks: &[Look]) -> bool {
        let Self {
            runner,
            cache,
            entries,
        } = self;
        let slot_word = super::slot_word(slot);
        for look in looks {
            let key = (slot_word, look.word());
            if entries.contains_key(&key) {
                continue;
            }
            match cache.submit(runner, slot, *look) {
                Ok((job, _)) => {
                    entries.insert(key, Entry::Pending(job));
                }
                Err(engine::ArtifactError::Runtime(engine::RuntimeError::QueueFull)) => break,
                Err(_) => {
                    entries.insert(key, Entry::Failed);
                }
            }
        }
        let ticked = runner.tick().is_ok();
        for (key, entry) in entries.iter_mut() {
            let Entry::Pending(job) = *entry else {
                continue;
            };
            let done = match runner.snapshot(job) {
                _ if !ticked => Some(false),
                None => Some(false),
                Some(snapshot) => match snapshot.state {
                    JobState::Completed => Some(true),
                    JobState::Failed | JobState::Cancelled => Some(false),
                    _ => None,
                },
            };
            let Some(done) = done else {
                continue;
            };
            let strip = done
                .then(|| runner.take_output(job))
                .flatten()
                .filter(|output| {
                    output.bytes().len()
                        == (PREVIEW_EDGE * PREVIEW_EDGE * PREVIEW_FRAMES * 4) as usize
                })
                .map(|output| {
                    egui::ColorImage::from_rgba_unmultiplied(
                        [
                            (PREVIEW_EDGE * PREVIEW_FRAMES) as usize,
                            PREVIEW_EDGE as usize,
                        ],
                        output.bytes(),
                    )
                });
            *entry = match strip {
                Some(image) => Entry::Ready(ctx.load_texture(
                    format!("animation.preview.{}.{}", key.0, key.1),
                    image,
                    egui::TextureOptions::LINEAR,
                )),
                None => Entry::Failed,
            };
        }
        entries
            .values()
            .any(|entry| matches!(entry, Entry::Pending(_)))
            || looks
                .iter()
                .any(|look| !entries.contains_key(&(slot_word, look.word())))
    }

    /// The strip of one look: `PREVIEW_FRAMES` square frames side by side.
    pub(super) fn strip(&self, slot: Slot, look: Look) -> Option<&egui::TextureHandle> {
        match self.entries.get(&(super::slot_word(slot), look.word()))? {
            Entry::Ready(texture) => Some(texture),
            _ => None,
        }
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        self.runner.shutdown();
    }
}
