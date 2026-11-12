//! Turns a source range into a smoothed subject path in source ticks.
mod keys;
mod merge;
mod path;
mod sampling;
use super::{AnalysisSettings, Error as AnalysisError, Result};
use crate::{
    EditError, RuntimeError, SessionError,
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobState,
        ResourceClass,
    },
    session::wire::ControlError,
};
use ai::vision::VisionDetector;
pub use keys::{ReframeKeys, ShotKeys, emit};
use media::{SourceAsset, StreamKind};
pub(crate) use merge::merge;
use serde::{Deserialize, Serialize};
use std::{io::Write, sync::Mutex};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

const VERSION: &str = "reframe-v1:8hz:rgb-hard-cuts:yunet-iou.2-hold.5-stable4:largest-stable:deadband12-tau.35-vmax768-fwd-bwd";
/// Sampling period: 8 Hz of source time.
pub(crate) const STEP: i64 = TICKS_PER_SECOND / 8;
const MAX_SECONDS: i64 = 3600;
const MAX_SHOTS: usize = 4096;
const MAX_BYTES: usize = 8 << 20;
const MAX_PIXELS: u64 = 1 << 24;

/// Output aspect of the constant crop window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReframeTarget {
    #[serde(rename = "9:16")]
    Vertical,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "16:9")]
    Landscape,
}
impl ReframeTarget {
    fn aspect(self) -> (u64, u64) {
        match self {
            Self::Vertical => (9, 16),
            Self::Square => (1, 1),
            Self::Landscape => (16, 9),
        }
    }
    /// The widest-and-tallest window of this aspect inside a `width` x `height`
    /// frame, as fractions of that frame. One side is always the full frame.
    fn crop_size(self, width: u32, height: u32) -> (f64, f64) {
        let (tw, th) = self.aspect();
        let (w, h) = (f64::from(width), f64::from(height));
        let (tw, th) = (tw as f64, th as f64);
        if tw * h <= th * w {
            ((tw * h) / (th * w), 1.0)
        } else {
            (1.0, (th * w) / (tw * h))
        }
    }
}
/// Why a shot's path is not simply the followed face. A shot may carry several.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReframeReason {
    /// More than one face was stable at once; the largest is followed.
    Ambiguous,
    /// No face was stable; the crop is centred for the whole shot.
    NoSubject,
    /// The followed face was gone for more than 0.5 s and the crop eased to centre.
    Lost,
}
/// One smoothed top-left crop position at a source tick, in `project::Crop` units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReframeSample {
    pub at: Tick,
    pub x: f64,
    pub y: f64,
}
/// One shot between hard cuts. Samples sit every 1/8 s from the shot start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReframeShot {
    pub range: SourceRange,
    pub reasons: Vec<ReframeReason>,
    pub samples: Vec<ReframeSample>,
}
/// The cached analysis: a subject path per shot in source ticks. It is
/// deterministic for its key, so it can always be produced again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReframeAnalysis {
    version: String,
    key: [u8; 32],
    models: [u8; 32],
    pub range: SourceRange,
    pub target: ReframeTarget,
    /// Source frame size in pixels.
    pub source: [u32; 2],
    /// Constant crop width and height as fractions of the source frame.
    pub size: [f64; 2],
    pub shots: Vec<ReframeShot>,
}
/// Everything the cache key binds: source content, range, target, settings,
/// the loaded adapter's fingerprint and the analysis recipe.
#[derive(Clone)]
pub struct ReframeRequest {
    asset: SourceAsset,
    range: SourceRange,
    target: ReframeTarget,
    models: [u8; 32],
    settings: AnalysisSettings,
    source: [u32; 2],
    key: CacheKey,
}
/// Retained runner job. `finish` caches only a complete, validated output and
/// returns None while work is outstanding.
pub struct ReframeJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: ReframeRequest,
}
impl ReframeRequest {
    /// At most one hour of a video source. `models` is the loaded adapter's
    /// complete `VisionFingerprint`, not a model name.
    pub fn new(
        asset: SourceAsset,
        range: SourceRange,
        target: ReframeTarget,
        models: [u8; 32],
        settings: AnalysisSettings,
    ) -> Result<Self> {
        if range.start() < Tick::ZERO
            || range.duration().0 > MAX_SECONDS * TICKS_PER_SECOND
            || !settings.admitted()
            || asset.probe().duration.is_some_and(|end| range.end() > end)
        {
            return Err(AnalysisError::from(EditError::Invalid("reframe request")));
        }
        let video = asset
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(EditError::Unsupported("reframe requires video"))?;
        let Some(info) = video.video.as_ref() else {
            return Err(AnalysisError::from(EditError::Invalid(
                "reframe video size",
            )));
        };
        if info.width == 0
            || info.height == 0
            || u64::from(info.width) * u64::from(info.height) > MAX_PIXELS
            || video
                .duration
                .and_then(|d| video.start.unwrap_or(Tick::ZERO).checked_add(d))
                .is_some_and(|end| range.end() > end)
        {
            return Err(AnalysisError::from(EditError::Invalid(
                "reframe video range or pixels",
            )));
        }
        let source = [info.width, info.height];
        let mut canonical = Vec::new();
        canonical.extend_from_slice(&range.start().0.to_le_bytes());
        canonical.extend_from_slice(&range.end().0.to_le_bytes());
        canonical.push(match target {
            ReframeTarget::Vertical => 0,
            ReframeTarget::Square => 1,
            ReframeTarget::Landscape => 2,
        });
        canonical.extend_from_slice(&settings.cut_threshold.to_bits().to_le_bytes());
        canonical.extend_from_slice(&settings.min_shot.0.to_le_bytes());
        canonical.extend_from_slice(&models);
        canonical.extend_from_slice(super::scene_detection::RECIPE.as_bytes());
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::ReframeAnalysis,
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
            target,
            models,
            settings,
            source,
            key,
        })
    }
    pub fn cache_key(&self) -> CacheKey {
        self.key
    }
    /// Enqueue one CPU job. A cache hit opens no source and never calls the
    /// detector. Progress and cancellation travel through the job's hooks.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
        detector: impl VisionDetector + 'static,
    ) -> Result<ReframeJob> {
        self.submit_with(runner, cache, detector, None)
    }
    /// The same, bound to one Session revision so `reframe.apply` can refuse a
    /// project that changed after the job started.
    pub fn submit_for_session(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
        detector: impl VisionDetector + 'static,
        session: &crate::Session,
    ) -> Result<ReframeJob> {
        self.submit_with(runner, cache, detector, Some(session))
    }
    fn submit_with(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
        detector: impl VisionDetector + 'static,
        session: Option<&crate::Session>,
    ) -> Result<ReframeJob> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => {
                self.decode(lease.bytes())?;
                Some(lease.bytes().to_vec())
            }
            Ok(None) | Err(RuntimeError::CacheCorrupt) => None,
            Err(e) => return Err(e.into()),
        };
        let request = self.clone();
        let spec = JobSpec {
            kind: JobKind::AnalyzeReframe,
            class: ResourceClass::Cpu,
            input: self.key,
            bytes: JobBytes {
                decoded_frames: 3 * MAX_PIXELS * 4,
                compositor: 128 << 20,
                result: MAX_BYTES as u64,
                ..Default::default()
            },
        };
        let detector = Mutex::new(Some(detector));
        let task = move |ctx: &JobContext| {
            ctx.check_cancelled()?;
            if let Some(bytes) = &cached {
                return Ok(bytes.clone());
            }
            let mut detector = detector
                .lock()
                .map_err(|_| RuntimeError::Poisoned)?
                .take()
                .ok_or(SessionError::StaleResult)?;
            let result = request.analyse(&mut detector, ctx)?;
            let bytes = encode(&result)?;
            ctx.check_cancelled()?;
            Ok(bytes)
        };
        let (id, cancel) = match session {
            Some(session) => runner.submit_for_session::<AnalysisError>(session, spec, task)?,
            None => runner.submit::<AnalysisError>(spec, task)?,
        };
        Ok(ReframeJob {
            id,
            cancel,
            request: self.clone(),
        })
    }
    fn analyse(
        &self,
        detector: &mut dyn VisionDetector,
        ctx: &JobContext,
    ) -> Result<ReframeAnalysis> {
        let detection = super::scene_detection::DetectionRequest::new(
            self.asset.clone(),
            self.range,
            super::scene_detection::DetectionSettings {
                hard_threshold: self.settings.cut_threshold,
                min_scene: self.settings.min_shot,
                ..Default::default()
            },
        )?;
        let mut cuts = vec![self.range.start()];
        cuts.extend(detection.hard_cuts(ctx)?);
        cuts.push(self.range.end());
        let [width, height] = self.source;
        let (cw, ch) = self.target.crop_size(width, height);
        let mut result = ReframeAnalysis {
            version: VERSION.into(),
            key: self.key.bytes(),
            models: self.models,
            range: self.range,
            target: self.target,
            source: self.source,
            size: [cw, ch],
            shots: Vec::new(),
        };
        for pair in cuts.windows(2) {
            ctx.check_cancelled()?;
            let (Some(&start), Some(&end)) = (pair.first(), pair.get(1)) else {
                return Err(ControlError::Limit.into());
            };
            let shot = SourceRange::new(start, end)?;
            let observed = sampling::observe(&self.asset, shot, self.range, detector, ctx)?;
            result
                .shots
                .push(path::plan(shot, &observed, self.source, [cw, ch])?);
        }
        // Exercise the same validation as a reopened cache entry.
        self.decode(&encode(&result)?)?;
        Ok(result)
    }
    fn decode(&self, bytes: &[u8]) -> crate::runtime::Result<ReframeAnalysis> {
        if bytes.len() > MAX_BYTES {
            return Err(RuntimeError::CacheCorrupt);
        }
        let result: ReframeAnalysis =
            serde_json::from_slice(bytes).map_err(|_| RuntimeError::CacheCorrupt)?;
        let [cw, ch] = result.size;
        let expected = self.target.crop_size(self.source[0], self.source[1]);
        if result.version != VERSION
            || result.key != self.key.bytes()
            || result.models != self.models
            || result.range != self.range
            || result.target != self.target
            || result.source != self.source
            || (cw, ch) != expected
            || result.shots.is_empty()
            || result.shots.len() > MAX_SHOTS
        {
            return Err(RuntimeError::CacheCorrupt);
        }
        let (max_x, max_y) = (1.0 - cw, 1.0 - ch);
        let mut start = self.range.start();
        for shot in &result.shots {
            let count = (shot.range.duration().0 - 1) / STEP + 1;
            if shot.range.start() != start
                || shot.range.end() > self.range.end()
                || shot.samples.len() as i64 != count
                || shot.reasons.windows(2).any(|w| w[0] >= w[1])
            {
                return Err(RuntimeError::CacheCorrupt);
            }
            for (i, s) in shot.samples.iter().enumerate() {
                if s.at.0 != shot.range.start().0 + i as i64 * STEP
                    || !(0.0..=max_x).contains(&s.x)
                    || !(0.0..=max_y).contains(&s.y)
                {
                    return Err(RuntimeError::CacheCorrupt);
                }
            }
            start = shot.range.end();
        }
        if start != self.range.end() {
            return Err(RuntimeError::CacheCorrupt);
        }
        Ok(result)
    }
}
impl ReframeJob {
    /// Validate and cache a completed result without consuming its retained
    /// output, so `reframe.apply` can still read the same job. `None` while the
    /// job is outstanding.
    pub fn publish(
        &self,
        runner: &JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<ReframeAnalysis>> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(RuntimeError::JobNotRetained)?;
        match snapshot.state {
            JobState::Cancelled => Err(RuntimeError::Cancelled.into()),
            JobState::Failed => Err(AnalysisError::Runtime(RuntimeError::Failed(
                snapshot.error.ok_or(SessionError::StaleResult)?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(RuntimeError::JobNotRetained)?;
                let result = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<ReframeAnalysis>> {
        let result = self.publish(runner, cache)?;
        if result.is_some() {
            runner
                .take_output(self.id)
                .ok_or(RuntimeError::JobNotRetained)?;
        }
        Ok(result)
    }
}
impl ReframeAnalysis {
    /// Constant crop width and height as fractions of the source frame.
    pub fn crop_size(&self) -> (f64, f64) {
        (self.size[0], self.size[1])
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
fn encode(result: &ReframeAnalysis) -> Result<Vec<u8>> {
    let mut writer = Limited(Vec::new());
    serde_json::to_writer(&mut writer, result).map_err(|_| ControlError::Limit)?;
    writer.0.shrink_to_fit();
    Ok(writer.0)
}
