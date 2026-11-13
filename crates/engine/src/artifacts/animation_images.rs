//! Short motion previews of the built-in clip animations, scheduled by the engine.
use super::{Error as ArtifactError, Result as ArtifactResult};
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass};
use crate::session::{DEFAULT_ANIMATION_DURATION, EditError, sample_animations};
use crate::{AnimationPreset, AnimationSlot, ByteBudget, ByteLease, ByteOwner};
use project::{Animations, BuiltinPath, Canvas, ParamValue, TransformField};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use time::Tick;

/// Edge of one square preview frame, in pixels.
pub const PREVIEW_EDGE: u32 = 64;
/// Frames of one preview. They sit side by side in one strip, so a result is
/// `PREVIEW_FRAMES * PREVIEW_EDGE` pixels wide and `PREVIEW_EDGE` high.
pub const PREVIEW_FRAMES: u32 = 12;
/// LRU entries: one per slot and look, 37 in all; at most 7.5 MiB of RGBA8.
pub const MAX_PREVIEWS: usize = 40;
const MAX_BYTES: u64 = PREVIEW_EDGE as u64 * PREVIEW_EDGE as u64 * PREVIEW_FRAMES as u64 * 4;
/// The neutral sample is a square this many pixels from centre to edge.
const SAMPLE_HALF: f64 = 16.0;
/// Every preview plays one second of the sample clip.
const LENGTH: Tick = Tick(time::TICKS_PER_SECOND);
/// Sample shades, light to dark along the local diagonal, so a turn reads.
const LIGHT: f64 = 238.0;
const DARK: f64 = 138.0;

struct Entry {
    key: CacheKey,
    bytes: Vec<u8>,
    _lease: ByteLease,
}
/// One cache per editor. The strip is rasterised on the CPU, so it needs no device.
#[derive(Clone)]
pub struct AnimationPreviews {
    budget: ByteBudget,
    entries: Arc<Mutex<VecDeque<Entry>>>,
    renders: Arc<AtomicUsize>,
}

fn invalid() -> ArtifactError {
    ArtifactError::Edit(EditError::Invalid("animation preview"))
}

/// The pose of the sample at `at`: position, scale, rotation and opacity.
struct Pose {
    x: f64,
    y: f64,
    scale: [f64; 2],
    rotate: f64,
    opacity: f64,
}
fn pose(animations: &Animations, at: Tick) -> ArtifactResult<Pose> {
    let field = |field: TransformField| field.value_at(animations, at).map_err(|_| invalid());
    let opacity = match animations
        .value_at(&BuiltinPath::Opacity.into(), at)
        .map_err(|_| invalid())?
    {
        None => 1.0,
        Some(ParamValue::Scalar(value)) if value.is_finite() => value.clamp(0.0, 1.0),
        Some(_) => return Err(invalid()),
    };
    Ok(Pose {
        x: field(TransformField::PositionX)?,
        y: field(TransformField::PositionY)?,
        scale: [
            field(TransformField::ScaleX)?,
            field(TransformField::ScaleY)?,
        ],
        rotate: field(TransformField::Rotate)?,
        opacity,
    })
}

/// Straight sRGB RGBA8 of one frame pixel: 2x2 samples of the rotated, scaled square.
fn pixel(pose: &Pose, x: u32, y: u32) -> [u8; 4] {
    let (sin, cos) = pose.rotate.to_radians().sin_cos();
    let centre = f64::from(PREVIEW_EDGE) / 2.0;
    let (mut hits, mut shade) = (0.0_f64, 0.0_f64);
    for sub_y in 0..2 {
        for sub_x in 0..2 {
            let px = f64::from(x) + (f64::from(sub_x) + 0.5) / 2.0 - centre - pose.x;
            let py = f64::from(y) + (f64::from(sub_y) + 0.5) / 2.0 - centre - pose.y;
            // Rotation is clockwise on a y-down canvas; undo it, then the scale.
            let local_x = (px * cos + py * sin) / pose.scale[0];
            let local_y = (-px * sin + py * cos) / pose.scale[1];
            if local_x.abs() <= SAMPLE_HALF && local_y.abs() <= SAMPLE_HALF {
                hits += 1.0;
                let along = (local_x + local_y) / (4.0 * SAMPLE_HALF) + 0.5;
                shade += LIGHT + (DARK - LIGHT) * along.clamp(0.0, 1.0);
            }
        }
    }
    if hits == 0.0 {
        return [0; 4];
    }
    let grey = (shade / hits).round().clamp(0.0, 255.0) as u8;
    let alpha = (pose.opacity * hits / 4.0 * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    [grey, grey, grey, alpha]
}

/// The strip of one look: `PREVIEW_FRAMES` frames across one second, built from the same
/// keys `clip.set_animation` writes and evaluated on a neutral square. Pure: the same look
/// always yields the same bytes.
pub fn preview_bytes(slot: AnimationSlot, preset: AnimationPreset) -> ArtifactResult<Vec<u8>> {
    let canvas = Canvas {
        width: PREVIEW_EDGE,
        height: PREVIEW_EDGE,
        ..Canvas::default()
    };
    let animations = sample_animations(slot, preset, DEFAULT_ANIMATION_DURATION, LENGTH, &canvas)?;
    let width = (PREVIEW_EDGE * PREVIEW_FRAMES) as usize;
    let mut out = vec![0_u8; width * PREVIEW_EDGE as usize * 4];
    for frame in 0..PREVIEW_FRAMES {
        // `checked_scale` refuses a zero factor, so the first frame is the clip start.
        let at = if frame == 0 {
            Tick::ZERO
        } else {
            LENGTH
                .checked_scale(
                    i64::from(frame),
                    i64::from(PREVIEW_FRAMES),
                    time::Rounding::Nearest,
                )
                .map_err(|_| invalid())?
        };
        let pose = pose(&animations, at)?;
        for y in 0..PREVIEW_EDGE {
            for x in 0..PREVIEW_EDGE {
                let offset = (y as usize * width + (frame * PREVIEW_EDGE + x) as usize) * 4;
                if let Some(target) = out.get_mut(offset..offset + 4) {
                    target.copy_from_slice(&pixel(&pose, x, y));
                }
            }
        }
    }
    Ok(out)
}

impl AnimationPreviews {
    pub fn new(budget: ByteBudget) -> Self {
        Self {
            budget,
            entries: Arc::new(Mutex::new(VecDeque::new())),
            renders: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many strips were rasterised, as opposed to served from the cache.
    pub fn renders(&self) -> usize {
        self.renders.load(Ordering::SeqCst)
    }

    /// Start a cancellable preview of one look in one slot at its default duration.
    /// The result is the strip of [`preview_bytes`]. The look, slot, duration, sample and
    /// frame geometry key the cache; a hit is a job too, with the same cancellation and
    /// result contract. Entries are charged to the runner's shared byte budget.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        slot: AnimationSlot,
        preset: AnimationPreset,
    ) -> ArtifactResult<(JobId, CancelToken)> {
        let mut sample = blake3::Hasher::new();
        sample.update(b"animation-preview-sample");
        sample.update(&PREVIEW_EDGE.to_le_bytes());
        sample.update(&PREVIEW_FRAMES.to_le_bytes());
        sample.update(&SAMPLE_HALF.to_bits().to_le_bytes());
        sample.update(&LENGTH.0.to_le_bytes());
        let settings = serde_json::to_vec(&(slot, preset, DEFAULT_ANIMATION_DURATION.0))
            .map_err(|_| invalid())?;
        let settings = blake3::hash(&settings);
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: *sample.finalize().as_bytes(),
            provider: "local",
            model: "animation-preview",
            version: "1",
            settings: settings.as_bytes(),
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let cache = self.clone();
        Ok(runner.submit(
            JobSpec {
                kind: JobKind::AnimationPreview,
                class: ResourceClass::Cpu,
                input: key,
                bytes: JobBytes {
                    result: MAX_BYTES,
                    ..Default::default()
                },
            },
            move |context| {
                context.check_cancelled()?;
                if !cache.budget.same_policy(context.memory_budget()) {
                    return Err(invalid());
                }
                {
                    let mut entries = cache
                        .entries
                        .lock()
                        .map_err(|_| crate::RuntimeError::Poisoned)?;
                    if let Some(index) = entries.iter().position(|e| e.key == key) {
                        let entry = entries
                            .remove(index)
                            .ok_or(ArtifactError::Runtime(crate::RuntimeError::CacheCorrupt))?;
                        let result = entry.bytes.clone();
                        entries.push_back(entry);
                        return Ok(result);
                    }
                }
                cache.renders.fetch_add(1, Ordering::SeqCst);
                let bytes = preview_bytes(slot, preset)?;
                context.check_cancelled()?;
                // Evict before admission so a full cache can progress under pressure.
                {
                    let mut entries = cache
                        .entries
                        .lock()
                        .map_err(|_| crate::RuntimeError::Poisoned)?;
                    if entries.len() >= MAX_PREVIEWS {
                        entries.pop_front();
                    }
                }
                let lease = cache
                    .budget
                    .reserve(ByteOwner::JobResults, bytes.len() as u64)?;
                let retained = bytes.clone();
                context.commit_publication(|| {
                    let mut entries = cache
                        .entries
                        .lock()
                        .map_err(|_| crate::RuntimeError::Poisoned)?;
                    entries.retain(|entry| entry.key != key);
                    while entries.len() >= MAX_PREVIEWS {
                        entries.pop_front();
                    }
                    entries.push_back(Entry {
                        key,
                        bytes: retained,
                        _lease: lease,
                    });
                    Ok(bytes)
                })
            },
        )?)
    }
}
