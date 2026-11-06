//! Two-pass EBU R128 normalization: content-bound gains, a cached measurement
//! job over the production mix graph, and export admission. The job never edits
//! the project; the caller applies its gains through `audio.apply_loudness`.
use crate::{
    EditError,
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass},
};
use project::{ItemId, ItemParams, LoudnessGains, Project, ProjectData};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const VERSION: &str =
    "loudness-v1:bs1770-4-kweight:gate-70-rel10:400ms-hop100ms:truepeak-polyphase";
const MAX_BYTES: usize = 1 << 20;

/// Integrated loudness and true peak of one measured signal; `None` is silence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measured {
    pub integrated_lufs: Option<f64>,
    pub true_peak_db: Option<f64>,
}
impl From<render::Measurement> for Measured {
    fn from(m: render::Measurement) -> Self {
        Self {
            integrated_lufs: m.integrated_lufs,
            true_peak_db: m.true_peak_db,
        }
    }
}

/// One finished measurement: gains bound to the measured content, plus the
/// numbers they were derived from (clips alone; master after clip gains).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoudnessResult {
    pub gains: LoudnessGains,
    pub clips: Vec<(ItemId, Measured)>,
    pub master: Option<Measured>,
}

/// Hash of everything that changes audible output or the requested targets:
/// the active scene's audible items with their volume/mute automation, track
/// mutes, sample rate and the loudness targets. Master fader and visual edits are
/// excluded because measurement ignores them.
pub fn content_key(data: &ProjectData) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(VERSION.as_bytes());
    hash.update(&data.settings.audio_sample_rate.to_le_bytes());
    for target in [data.settings.loudness.clip, data.settings.loudness.master] {
        match target {
            Some(t) => {
                hash.update(&[1]);
                hash.update(&t.target_lufs.to_bits().to_le_bytes());
                hash.update(&t.true_peak_db.to_bits().to_le_bytes());
            }
            None => {
                hash.update(&[0]);
            }
        }
    }
    hash.update(&data.scene().id.0.to_le_bytes());
    for track in data.scene().timeline.iter() {
        hash.update(&track.id.0.to_le_bytes());
        hash.update(&[u8::from(track.muted)]);
        for item in &track.items {
            let (volume, muted, source_audio) = match &item.content {
                ItemParams::Audio(p) => (p.volume, p.muted, true),
                ItemParams::Video(p) => (p.volume, p.muted, p.source_audio_enabled),
                _ => continue,
            };
            hash.update(&item.id.0.to_le_bytes());
            hash.update(&item.start.0.to_le_bytes());
            hash.update(&item.source_range.start().0.to_le_bytes());
            hash.update(&item.source_range.end().0.to_le_bytes());
            hash.update(&item.source_asset.map_or(0, |a| a.0).to_le_bytes());
            if let Some(asset) = item
                .source_asset
                .and_then(|id| data.assets.iter().find(|a| a.id() == id))
            {
                hash.update(&asset.content_hash().0);
            }
            hash.update(&volume.to_bits().to_le_bytes());
            hash.update(&[u8::from(muted), u8::from(source_audio)]);
            for (path, param) in item.animations.iter() {
                if matches!(path.as_str(), "volume" | "muted") {
                    hash.update(path.as_str().as_bytes());
                    // Param serialization is infallible for validated projects.
                    hash.update(&serde_json::to_vec(param).unwrap_or_default());
                }
            }
        }
    }
    *hash.finalize().as_bytes()
}

/// Export admission: requested normalization needs gains measured for exactly
/// this audio content, so preview and export cannot disagree.
pub fn admit(project: &Project) -> Result<(), EditError> {
    let l = &project.data().settings.loudness;
    if l.clip.is_none() && l.master.is_none() {
        return Ok(());
    }
    match &l.measured {
        Some(m) if m.key == content_key(project.data()) => Ok(()),
        _ => Err(EditError::LoudnessStale),
    }
}

/// Retained measurement job. `finish` returns None while pending.
pub struct LoudnessJob {
    pub id: JobId,
    pub cancel: CancelToken,
    key: CacheKey,
    content: [u8; 32],
}

fn cache_key(content: [u8; 32]) -> Result<CacheKey, EditError> {
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Loudness,
        source: content,
        provider: "local",
        model: "ebur128",
        version: VERSION,
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}

/// Queue one CPU measurement of the project's requested targets. A cached
/// result for identical content and targets skips the audio decode entirely.
pub fn submit_loudness(
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    project: Arc<Project>,
    sources: impl Fn() -> Box<dyn render::SourceProvider> + Send + Sync + 'static,
) -> Result<LoudnessJob, EditError> {
    let l = &project.data().settings.loudness;
    if l.clip.is_none() && l.master.is_none() {
        return Err(EditError::Invalid("no loudness target set"));
    }
    let content = content_key(project.data());
    let key = cache_key(content)?;
    let cached = match cache.get(key) {
        Ok(Some(lease)) => {
            decode(lease.bytes(), content)?;
            Some(lease.bytes().to_vec())
        }
        Ok(None) | Err(EditError::CacheCorrupt) => None,
        Err(e) => return Err(e),
    };
    let (id, cancel) = runner.submit(
        JobSpec {
            kind: JobKind::MeasureLoudness,
            class: ResourceClass::Cpu,
            input: key,
            bytes: JobBytes {
                result: MAX_BYTES as u64,
                ..JobBytes::default()
            },
        },
        move |context| {
            context.check_cancelled()?;
            if let Some(bytes) = &cached {
                return Ok(bytes.clone());
            }
            let report = render::measure_loudness(&project, sources().as_mut(), context)?;
            let mut gains = report.gains;
            gains.key = content;
            let result = LoudnessResult {
                gains,
                clips: report
                    .clips
                    .into_iter()
                    .map(|(i, m)| (i, m.into()))
                    .collect(),
                master: report.master.map(Into::into),
            };
            context.check_cancelled()?;
            let bytes = serde_json::to_vec(&result).map_err(|_| EditError::Limit)?;
            if bytes.len() > MAX_BYTES {
                return Err(EditError::Limit);
            }
            Ok(bytes)
        },
    )?;
    Ok(LoudnessJob {
        id,
        cancel,
        key,
        content,
    })
}

fn decode(bytes: &[u8], content: [u8; 32]) -> Result<LoudnessResult, EditError> {
    if bytes.len() > MAX_BYTES {
        return Err(EditError::CacheCorrupt);
    }
    let result: LoudnessResult =
        serde_json::from_slice(bytes).map_err(|_| EditError::CacheCorrupt)?;
    if result.gains.key != content || !result.gains.is_valid() {
        return Err(EditError::CacheCorrupt);
    }
    Ok(result)
}

impl LoudnessJob {
    /// Validate and cache the finished result; typed terminal failures and
    /// cancellation propagate. Apply it with `audio.apply_loudness`.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<LoudnessResult>, EditError> {
        let snapshot = runner.snapshot(self.id).ok_or(EditError::JobNotRetained)?;
        match snapshot.state {
            JobState::Cancelled => Err(EditError::Cancelled),
            JobState::Failed => Err(EditError::JobFailure(
                snapshot.error.ok_or(EditError::StaleResult)?,
            )),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                let result = decode(bytes, self.content)?;
                cache.put(self.key, bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
}
