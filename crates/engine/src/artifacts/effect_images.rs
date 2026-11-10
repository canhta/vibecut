//! Device-injected offscreen effect thumbnails, scheduled by the engine.
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass};
use crate::{ByteBudget, ByteLease, ByteOwner, EditError, Result};
use project::{Animations, EffectInstance};
use render::effects::{EffectCompositor, apply_effect};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Preview callers downsample their source frame before admission.
pub const MAX_THUMBNAIL_EDGE: u32 = 320;
/// LRU entries; at most 12.5 MiB of RGBA8, charged to the shared byte budget.
pub const MAX_THUMBNAILS: usize = 32;
const MAX_BYTES: u64 = MAX_THUMBNAIL_EDGE as u64 * MAX_THUMBNAIL_EDGE as u64 * 4;
struct Identity<'a>(&'a mut blake3::Hasher);
impl std::io::Write for Identity<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Entry {
    key: CacheKey,
    bytes: Vec<u8>,
    _lease: ByteLease,
}
/// One cache per device adapter. No IO or GPU work runs under its cache lock.
/// The desktop constructs this with its GPU adapter; headless tests may inject CPU.
/// Declared unsupported passes and adapter Unsupported errors use the shared CPU
/// reference. Other typed failures propagate without publication. Caller supplies
/// a bounded, accounted working-space image.
#[derive(Clone)]
pub struct EffectThumbnails {
    adapter: Arc<dyn EffectCompositor>,
    budget: ByteBudget,
    entries: Arc<Mutex<VecDeque<Entry>>>,
}
impl EffectThumbnails {
    pub fn new(adapter: Arc<dyn EffectCompositor>, budget: ByteBudget) -> Self {
        Self {
            adapter,
            budget,
            entries: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
    /// Start a cancellable preview of a registered effect on a source image.
    /// Output is straight sRGB RGBA8, with the same dimensions as `source`.
    /// Source content/geometry, full instance, animations and local time key the
    /// cache. Negative local time refuses; cadence is caller-driven.
    /// Cache hits are jobs too, preserving the same cancellation/result contract.
    /// The shared executor retries only Unsupported on CPU; budget, cancellation
    /// and invalid-data failures never fall back or publish an unfinished result.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        source: Arc<render::Image>,
        effect: EffectInstance,
        animations: Animations,
        local: time::Tick,
    ) -> Result<(JobId, CancelToken)> {
        if local < time::Tick::ZERO {
            return Err(EditError::Invalid("thumbnail local time"));
        }
        effect.validate()?;
        if source.width().max(source.height()) > MAX_THUMBNAIL_EDGE {
            return Err(EditError::Limit);
        }
        let mut hash = blake3::Hasher::new();
        hash.update(&source.width().to_le_bytes());
        hash.update(&source.height().to_le_bytes());
        for pixel in source.pixels() {
            for value in pixel {
                hash.update(&value.to_le_bytes());
            }
        }
        let mut identity = blake3::Hasher::new();
        serde_json::to_writer(Identity(&mut identity), &(&effect, &animations, local))
            .map_err(|_| EditError::InvalidParams)?;
        let settings = identity.finalize();
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: *hash.finalize().as_bytes(),
            provider: "local",
            model: "effect-thumbnail",
            version: "2",
            settings: settings.as_bytes(),
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let cache = self.clone();
        runner.submit(
            JobSpec {
                kind: JobKind::EffectThumbnail,
                class: ResourceClass::GpuEncoder,
                input: key,
                bytes: JobBytes {
                    result: MAX_BYTES,
                    ..Default::default()
                },
            },
            move |context| {
                context.check_cancelled()?;
                if !cache.budget.same_policy(context.memory_budget()) {
                    return Err(EditError::Invalid("thumbnail byte policy"));
                }
                {
                    let mut entries = cache.entries.lock().map_err(|_| EditError::JobPoisoned)?;
                    if let Some(index) = entries.iter().position(|e| e.key == key) {
                        let entry = entries.remove(index).ok_or(EditError::CacheCorrupt)?;
                        let result = entry.bytes.clone();
                        entries.push_back(entry);
                        return Ok(result);
                    }
                }
                let passes = render::effects::evaluate_effect(
                    &animations,
                    &effect,
                    local,
                    source.width(),
                    source.height(),
                    true,
                )?;
                let owned = render::Image::from_working_pixels(
                    source.width(),
                    source.height(),
                    source.pixels().iter().copied(),
                    context,
                )?;
                let image = apply_effect(owned, &passes, &source, cache.adapter.as_ref(), context)?;
                if image.width() != source.width() || image.height() != source.height() {
                    return Err(EditError::Invalid("thumbnail output dimensions"));
                }
                let packed = image.rgba8(13, context)?;
                context.check_cancelled()?;
                // Evict before admission so a full cache can progress under pressure.
                {
                    let mut entries = cache.entries.lock().map_err(|_| EditError::JobPoisoned)?;
                    if entries.len() >= MAX_THUMBNAILS {
                        entries.pop_front();
                    }
                }
                let lease = cache
                    .budget
                    .reserve(ByteOwner::JobResults, packed.bytes.len() as u64)?;
                let retained = packed.bytes.clone();
                let output = packed.bytes.clone();
                context.commit_publication(|| {
                    let mut entries = cache.entries.lock().map_err(|_| EditError::JobPoisoned)?;
                    // Concurrent identical jobs replace, rather than duplicate, entries.
                    entries.retain(|entry| entry.key != key);
                    while entries.len() >= MAX_THUMBNAILS {
                        entries.pop_front();
                    }
                    entries.push_back(Entry {
                        key,
                        bytes: retained,
                        _lease: lease,
                    });
                    Ok(output)
                })
            },
        )
    }
}
