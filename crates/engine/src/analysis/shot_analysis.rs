//! Produces conservative source-time shot analysis.
mod reidentification;
mod tracking;
use crate::{
    EditError,
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState,
        ResourceClass,
    },
};
use ai::vision::{SceneKind, VisionDetector};
use media::{MfccProgress, SourceAsset, StreamKind, VisionFrame, VisionPrepLimits};
use serde::{Deserialize, Serialize};
use std::{io::Write, sync::Mutex};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

const VERSION: &str = "shot-analysis-v5:rgb-hard-only-f32-order:sface-cosine.30-margin.08-consensus3:negative-all-visible-scored:grid32x18-rgb-mad:25fps-hold:8hz-iou.3:3s:mean1s:.5-.4";
const MAX_SHOTS: usize = 4096;
const MAX_SECONDS: usize = 86_400;
const MAX_BYTES: usize = 32 << 20;
const MAX_PIXELS: u64 = 1 << 24;
const VIDEO_STEP: i64 = TICKS_PER_SECOND / 25;
const AUDIO_STEP: i64 = TICKS_PER_SECOND / 100;

/// Explicit source-time speech evidence, typically from the accepted transcript.
/// Without it, absent visible speaker evidence is Unknown, never a speech claim.
#[derive(Clone)]
pub struct AnalysisRequest {
    asset: SourceAsset,
    range: SourceRange,
    speech: Vec<SourceRange>,
    models: [u8; 32],
    settings: AnalysisSettings,
    key: CacheKey,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSettings {
    /// Mean normalized RGB grid change in 0.01..=1. Default 0.28.
    pub cut_threshold: f32,
    /// Minimum spacing between detected cuts, in source ticks: 0.04..=10 s.
    pub min_shot: Tick,
}
impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            cut_threshold: 0.28,
            min_shot: Tick(TICKS_PER_SECOND / 2),
        }
    }
}
/// A reference to a face track inside one shot, never a person identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakerRef {
    pub shot: u32,
    pub track: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpeakerState {
    Visible { local: SpeakerRef, probability: f32 },
    OffScreenSpeech,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    Camera,
    Screen,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondAnalysis {
    pub range: SourceRange,
    pub people_count: usize,
    pub speaker: SpeakerState,
}
/// Source-pixel box of a shot-local track, retained for layout consumers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceTrack {
    /// Cross-shot anchor only after a confident, unambiguous embedding match.
    /// None is Unknown, including the first observation of an individual.
    pub identity: Option<SpeakerRef>,
    pub id: u32,
    pub bounds: [f32; 4],
    pub observed_frames: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShotAnalysis {
    pub range: SourceRange,
    pub people_count: usize,
    pub tracks: Vec<FaceTrack>,
    pub speaker: SpeakerState,
    /// Last confirmed shot-local speaker; this does NOT link a current face to it.
    pub continuity_hint: Option<SpeakerRef>,
    pub scene: Scene,
    pub seconds: Vec<SecondAnalysis>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisResult {
    version: String,
    key: [u8; 32],
    models: [u8; 32],
    pub shots: Vec<ShotAnalysis>,
}
/// Retained runner job. `finish` caches only a complete, validated output; it is
/// retryable after cache pressure and returns None while work is outstanding.
pub struct AnalysisJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: AnalysisRequest,
}
impl AnalysisRequest {
    /// At most one day, 4096 shots and 64 tracks per shot. Each shot has
    /// ceil(duration / 1 s) bins, including its partial final second.
    /// Speech ranges must be ordered, disjoint and inside the requested range.
    /// `models` is the loaded adapter's complete VisionFingerprint, not a model name.
    pub fn new(
        asset: SourceAsset,
        range: SourceRange,
        speech: Vec<SourceRange>,
        models: [u8; 32],
        settings: AnalysisSettings,
    ) -> Result<Self, EditError> {
        if range.start() < Tick::ZERO
            || range.duration().0 > MAX_SECONDS as i64 * TICKS_PER_SECOND
            || speech.len() > 100_000
            || !settings.cut_threshold.is_finite()
            || !(0.01..=1.).contains(&settings.cut_threshold)
            || !(VIDEO_STEP..=10 * TICKS_PER_SECOND).contains(&settings.min_shot.0)
            || asset.probe().duration.is_some_and(|end| range.end() > end)
        {
            return Err(EditError::Invalid("shot analysis request"));
        }
        let mut end = range.start();
        for r in &speech {
            if r.start() < end || r.end() > range.end() {
                return Err(EditError::Invalid("analysis speech ranges"));
            }
            end = r.end();
        }
        let video = asset
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(EditError::Unsupported("shot analysis requires video"))?;
        if video
            .video
            .as_ref()
            .is_none_or(|v| u64::from(v.width) * u64::from(v.height) > MAX_PIXELS)
            || video
                .duration
                .and_then(|d| video.start.unwrap_or(Tick::ZERO).checked_add(d))
                .is_some_and(|end| range.end() > end)
        {
            return Err(EditError::Invalid("analysis video range or pixels"));
        }
        let mut canonical = Vec::new();
        canonical.extend_from_slice(&range.start().0.to_le_bytes());
        canonical.extend_from_slice(&range.end().0.to_le_bytes());
        canonical.extend_from_slice(&settings.cut_threshold.to_bits().to_le_bytes());
        canonical.extend_from_slice(&settings.min_shot.0.to_le_bytes());
        // Key settings remain below the cache owner's 1 MiB bound.
        let mut speech_hash = blake3::Hasher::new();
        for r in &speech {
            speech_hash.update(&r.start().0.to_le_bytes());
            speech_hash.update(&r.end().0.to_le_bytes());
        }
        canonical.extend_from_slice(speech_hash.finalize().as_bytes());
        canonical.extend_from_slice(&models);
        canonical.extend_from_slice(crate::scene_detection::RECIPE.as_bytes());
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::ShotAnalysis,
            source: asset.content_hash().0,
            provider: "local",
            model: "vision",
            version: VERSION,
            settings: &canonical,
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self {
            asset,
            range,
            speech,
            models,
            settings,
            key,
        })
    }
    pub fn cache_key(&self) -> CacheKey {
        self.key
    }
    /// Enqueue one CPU job. Model loading remains with the composition root;
    /// caller-owned model weights/plans are not duplicated by this job.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
        detector: impl VisionDetector + 'static,
    ) -> Result<AnalysisJob, EditError> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => {
                self.decode(lease.bytes())?;
                Some(lease.bytes().to_vec())
            }
            Ok(None) | Err(EditError::CacheCorrupt) => None,
            Err(e) => return Err(e),
        };
        let request = self.clone();
        // Reserve three overlapping 16 Mi-pixel frames and bounded crop/bin/JSON
        // storage. The media owner checks packed area before conversion.
        let spec = JobSpec {
            kind: JobKind::AnalyzeShots,
            class: ResourceClass::Cpu,
            input: self.key,
            bytes: JobBytes {
                decoded_frames: 3 * MAX_PIXELS * 4,
                compositor: 384 << 20,
                result: MAX_BYTES as u64,
                ..Default::default()
            },
        };
        let detector = Mutex::new(Some(detector));
        let (id, cancel) = runner.submit(spec, move |ctx| {
            ctx.check_cancelled()?;
            if let Some(bytes) = &cached {
                return Ok(bytes.clone());
            }
            let mut detector = detector
                .lock()
                .map_err(|_| EditError::JobPoisoned)?
                .take()
                .ok_or(EditError::StaleResult)?;
            let result = request.analyze(&mut detector, ctx)?;
            let bytes = encode(&result)?;
            ctx.check_cancelled()?;
            Ok(bytes)
        })?;
        Ok(AnalysisJob {
            id,
            cancel,
            request: self.clone(),
        })
    }
    fn decode(&self, bytes: &[u8]) -> Result<AnalysisResult, EditError> {
        if bytes.len() > MAX_BYTES {
            return Err(EditError::CacheCorrupt);
        }
        let result: AnalysisResult =
            serde_json::from_slice(bytes).map_err(|_| EditError::CacheCorrupt)?;
        if result.version != VERSION
            || result.key != self.key.bytes()
            || result.models != self.models
            || result.shots.is_empty()
            || result.shots.len() > MAX_SHOTS
        {
            return Err(EditError::CacheCorrupt);
        }
        let mut start = self.range.start();
        let mut bins = 0;
        let mut hint = None;
        for (i, shot) in result.shots.iter().enumerate() {
            if shot.range.start() != start
                || shot.range.end() > self.range.end()
                || shot.tracks.len() > 64
                || shot.people_count > shot.tracks.len()
                || shot.seconds.is_empty()
            {
                return Err(EditError::CacheCorrupt);
            }
            let mut identities = std::collections::BTreeSet::new();
            let mut ids = std::collections::BTreeSet::new();
            for t in &shot.tracks {
                if let Some(anchor) = t.identity {
                    let valid = (anchor.shot as usize) < i
                        && result.shots.get(anchor.shot as usize).is_some_and(|s| {
                            s.tracks
                                .iter()
                                .any(|t| t.id == anchor.track && t.identity.is_none())
                        });
                    if !valid || !identities.insert((anchor.shot, anchor.track)) {
                        return Err(EditError::CacheCorrupt);
                    }
                }
                if !ids.insert(t.id)
                    || t.observed_frames < 13
                    || !t.bounds.iter().all(|v| v.is_finite())
                    || t.bounds.get(2).is_none_or(|v| *v <= 0.)
                    || t.bounds.get(3).is_none_or(|v| *v <= 0.)
                {
                    return Err(EditError::CacheCorrupt);
                }
            }
            let valid = |state: SpeakerState| match state {
                SpeakerState::Visible { local, probability } => {
                    local.shot as usize == i
                        && ids.contains(&local.track)
                        && probability.is_finite()
                        && (0.4..=1.).contains(&probability)
                }
                _ => true,
            };
            if !valid(shot.speaker) {
                return Err(EditError::CacheCorrupt);
            }
            if let SpeakerState::Visible { local, .. } = shot.speaker {
                hint = Some(local);
            }
            if shot.continuity_hint != hint {
                return Err(EditError::CacheCorrupt);
            }
            let mut second_start = start;
            for second in &shot.seconds {
                if second.range.start() != second_start
                    || second.range.end() > shot.range.end()
                    || second.range.duration().0 > TICKS_PER_SECOND
                    || second.people_count > shot.tracks.len()
                    || !valid(second.speaker)
                {
                    return Err(EditError::CacheCorrupt);
                }
                second_start = second.range.end();
                bins += 1;
            }
            if second_start != shot.range.end() || bins > MAX_SECONDS + MAX_SHOTS {
                return Err(EditError::CacheCorrupt);
            }
            start = shot.range.end();
        }
        if start != self.range.end() {
            return Err(EditError::CacheCorrupt);
        }
        Ok(result)
    }
    fn analyze(
        &self,
        detector: &mut dyn VisionDetector,
        ctx: &JobContext,
    ) -> Result<AnalysisResult, EditError> {
        let cuts = self.cuts(ctx)?;
        let mut result = AnalysisResult {
            version: VERSION.into(),
            key: self.key.bytes(),
            models: self.models,
            shots: Vec::new(),
        };
        let mut hint = None;
        let mut identities = reidentification::Identities::default();
        for (index, pair) in cuts.windows(2).enumerate() {
            ctx.check_cancelled()?;
            let range = SourceRange::new(
                *pair.first().ok_or(EditError::Limit)?,
                *pair.get(1).ok_or(EditError::Limit)?,
            )?;
            let (mut shot, evidence) = self.shot(index as u32, range, detector, ctx)?;
            identities.assign(index as u32, &mut shot.tracks, evidence, ctx)?;
            if let SpeakerState::Visible { local, .. } = shot.speaker {
                hint = Some(local);
            }
            shot.continuity_hint = hint;
            result.shots.push(shot);
        }
        // Exercise the same structural validation as a reopened cache entry.
        self.decode(&encode(&result)?)
    }
    fn cuts(&self, ctx: &JobContext) -> Result<Vec<Tick>, EditError> {
        let request = crate::scene_detection::DetectionRequest::new(
            self.asset.clone(),
            self.range,
            crate::scene_detection::DetectionSettings {
                hard_threshold: self.settings.cut_threshold,
                min_scene: self.settings.min_shot,
                ..Default::default()
            },
        )?;
        let mut cuts = vec![self.range.start()];
        cuts.extend(request.hard_cuts(ctx)?);
        cuts.push(self.range.end());
        Ok(cuts)
    }
    fn shot(
        &self,
        index: u32,
        range: SourceRange,
        detector: &mut dyn VisionDetector,
        ctx: &JobContext,
    ) -> Result<(ShotAnalysis, reidentification::Evidence), EditError> {
        let audio = self
            .asset
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Audio);
        let origin = audio.and_then(|s| s.start).unwrap_or(Tick::ZERO);
        let mut tracks = tracking::Tracks::new(index, range);
        let mut start = range.start();
        while start < range.end() {
            let end = Tick((start.0.saturating_add(3 * TICKS_PER_SECOND)).min(range.end().0));
            let window = SourceRange::new(start, end)?;
            let mut mfcc = Vec::new();
            if audio.is_some() {
                media::mfcc_range_with_progress(
                    &self.asset,
                    window,
                    &VisionPrepLimits {
                        max_frame_pixels: MAX_PIXELS,
                        ..Default::default()
                    },
                    ctx.media_cancel_flag(),
                    |progress| {
                        if ctx.cancelled() {
                            return Err(media::MediaError::Cancelled);
                        }
                        let (stage, value) = match progress {
                            MfccProgress::Preparing { decoded_to } => (
                                JobStage::Preparing,
                                (decoded_to.0 as f64 / start.0.max(1) as f64).clamp(0., 1.) as f32,
                            ),
                            MfccProgress::Analyzing { .. } => (
                                JobStage::Analyzing,
                                ((start.0 - self.range.start().0) as f64
                                    / self.range.duration().0 as f64)
                                    as f32,
                            ),
                        };
                        ctx.progress(stage, value)
                            .map_err(|_| media::MediaError::Cancelled)
                    },
                    |frame| {
                        if ctx.cancelled() {
                            return Err(media::MediaError::Cancelled);
                        }
                        mfcc.push(frame);
                        Ok(())
                    },
                )?;
            }
            ctx.progress(
                JobStage::Analyzing,
                ((start.0 - self.range.start().0) as f64 / self.range.duration().0 as f64) as f32,
            )?;
            // The video grid is anchored to the source audio origin (or zero).
            // Each sample selects the decoded presentation interval containing it.
            let relative = i128::from(start.0) - i128::from(origin.0);
            let grid = i128::from(origin.0)
                + relative.div_euclid(i128::from(VIDEO_STEP)) * i128::from(VIDEO_STEP);
            let mut next = i64::try_from(if grid < i128::from(start.0) {
                grid + i128::from(VIDEO_STEP)
            } else {
                grid
            })
            .unwrap_or(i64::MAX);
            let mut previous: Option<VisionFrame> = None;
            let mut failure = None;
            let mut emit = |frame: &VisionFrame, at: Tick| -> media::Result<()> {
                if ctx.cancelled() {
                    return Err(media::MediaError::Cancelled);
                }
                match tracks.frame(frame, at, detector, ctx) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        failure = Some(e);
                        Err(media::MediaError::Invalid("analysis frame"))
                    }
                }
            };
            let decoded = media::decode_shot_frames(
                &self.asset,
                window,
                &VisionPrepLimits {
                    max_frame_pixels: MAX_PIXELS,
                    ..Default::default()
                },
                ctx.media_cancel_flag(),
                |frame| {
                    if let Some(prev) = previous.as_ref() {
                        while next < frame.source_pts().0 && next < end.0 {
                            if next >= prev.source_pts().0 {
                                emit(prev, Tick(next))?;
                            }
                            next = next.saturating_add(VIDEO_STEP);
                        }
                    }
                    previous = Some(frame);
                    Ok(())
                },
            );
            if let Some(e) = failure {
                return Err(e);
            }
            decoded?;
            if let Some(frame) = previous.as_ref() {
                while next < end.0 {
                    if next >= frame.source_pts().0 {
                        tracks.frame(frame, Tick(next), detector, ctx)?;
                    }
                    next = next.saturating_add(VIDEO_STEP);
                }
            }
            tracks.flush(&mfcc, detector, ctx)?;
            start = end;
        }
        tracks.finish(&self.speech, audio.is_some())
    }
}
impl AnalysisJob {
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<AnalysisResult>, EditError> {
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
                let result = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode(result: &AnalysisResult) -> Result<Vec<u8>, EditError> {
    let mut writer = Limited(Vec::new());
    serde_json::to_writer(&mut writer, result).map_err(|_| EditError::Limit)?;
    writer.0.shrink_to_fit();
    Ok(writer.0)
}
