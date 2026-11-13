//! Computes footage corrections once per source and looks them up by key.
mod motion;

use crate::analysis::{Error as AnalysisError, Result};
use crate::session::wire::CorrectionKind;
use crate::{
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState,
        ResourceClass,
    },
};
use media::{SourceAsset, StreamKind, VisionPrepLimits};
use motion::Motion;
use project::{ItemId, ItemParams, Project};
use render::stabilize::{MAX_FRAMES, Track};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// Versioned recipe of the stabilize correction; part of every cache key.
pub(crate) const RECIPE: &str = "stabilize-v1:luma480-block32-r8-sad-parabola:sim4-irls:chain";
/// Largest decoded frame the analysis admits: 16 Mi pixels.
const MAX_PIXELS: u64 = 1 << 24;
/// The retained result: the largest track the render crate admits.
const MAX_BYTES: usize = 24 * MAX_FRAMES + 20;

/// One source's correction request. The key binds the source content, the whole-source range,
/// the kind and the recipe; the item's trim, its level and its position never enter it, so a
/// split or a level change reuses the entry.
#[derive(Clone)]
pub struct CorrectionRequest {
    asset: SourceAsset,
    kind: CorrectionKind,
    range: SourceRange,
    key: CacheKey,
}

/// A retained runner job. `finish` caches only a complete, validated track and returns it.
pub struct CorrectionJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: CorrectionRequest,
}

impl CorrectionRequest {
    /// Admit one source whose video has a known duration and a frame of at most 16 Mi pixels.
    pub fn new(asset: SourceAsset, kind: CorrectionKind) -> Result<Self> {
        let invalid = |reason| AnalysisError::Edit(crate::session::EditError::Unsupported(reason));
        let video = asset
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or_else(|| invalid("footage correction needs video"))?;
        let shape = video
            .video
            .as_ref()
            .ok_or_else(|| invalid("footage correction needs video"))?;
        if shape.width == 0
            || shape.height == 0
            || u64::from(shape.width) * u64::from(shape.height) > MAX_PIXELS
        {
            return Err(invalid("footage correction frame size"));
        }
        if Motion::new(shape.width, shape.height).is_none() {
            return Err(invalid("footage correction frame is too small"));
        }
        let duration = asset
            .probe()
            .duration
            .ok_or_else(|| invalid("footage correction needs a known duration"))?;
        let range = SourceRange::new(Tick::ZERO, duration)?;
        let settings = serde_json::to_vec(&(kind, range)).map_err(|_| {
            AnalysisError::Edit(crate::session::EditError::Invalid("correction settings"))
        })?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::FootageCorrection,
            source: asset.content_hash().0,
            provider: "local",
            model: "luma-block-match",
            version: RECIPE,
            settings: &settings,
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self {
            asset,
            kind,
            range,
            key,
        })
    }

    pub fn cache_key(&self) -> CacheKey {
        self.key
    }

    pub fn kind(&self) -> CorrectionKind {
        self.kind
    }

    /// The cached correction, if a valid one is stored. A corrupt or undecodable entry counts as
    /// absent so the next analysis replaces it.
    pub fn cached(&self, cache: &mut ArtifactCache) -> Result<Option<Arc<Track>>> {
        match cache.get(self.key) {
            Ok(Some(lease)) => Ok(self.decode(lease.bytes()).ok().map(Arc::new)),
            Ok(None) | Err(crate::RuntimeError::CacheCorrupt) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Queue one CPU job. A warm entry completes without opening the source; only `finish`
    /// publishes a complete cache entry.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<CorrectionJob> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => self
                .decode(lease.bytes())
                .ok()
                .map(|_| lease.bytes().to_vec()),
            Ok(None) | Err(crate::RuntimeError::CacheCorrupt) => None,
            Err(error) => return Err(error.into()),
        };
        let request = self.clone();
        let (id, cancel) = runner.submit(
            JobSpec {
                kind: JobKind::AnalyzeFootage,
                class: ResourceClass::Cpu,
                input: self.key,
                bytes: JobBytes {
                    decoded_frames: 3 * MAX_PIXELS * 4,
                    compositor: 16 << 20,
                    result: MAX_BYTES as u64,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                if let Some(bytes) = &cached {
                    return Ok(bytes.clone());
                }
                let track = request.analyze(ctx)?;
                ctx.check_cancelled()?;
                Ok(track.encode())
            },
        )?;
        Ok(CorrectionJob {
            id,
            cancel,
            request: self.clone(),
        })
    }

    fn decode(&self, bytes: &[u8]) -> crate::runtime::Result<Track> {
        if bytes.len() > MAX_BYTES {
            return Err(crate::RuntimeError::CacheCorrupt);
        }
        Track::decode(bytes).map_err(|_| crate::RuntimeError::CacheCorrupt)
    }

    /// Decode the whole video once, in slices of at most 300 s, and chain the measured motion.
    fn analyze(&self, ctx: &JobContext) -> Result<Track> {
        let video = self
            .asset
            .probe()
            .streams
            .iter()
            .find_map(|s| s.video.as_ref())
            .ok_or(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "footage correction needs video",
            )))?;
        let mut motion = Motion::new(video.width, video.height).ok_or(AnalysisError::Edit(
            crate::session::EditError::Unsupported("footage correction frame is too small"),
        ))?;
        let mut times: Vec<Tick> = Vec::new();
        let mut poses = Vec::new();
        let total = self.range.duration().0.max(1) as f64;
        let mut from = self.range.start();
        while from < self.range.end() {
            ctx.check_cancelled()?;
            let end = from
                .saturating_add(Tick(300 * TICKS_PER_SECOND))
                .min(self.range.end());
            media::decode_shot_frames(
                &self.asset,
                SourceRange::new(from, end)?,
                &VisionPrepLimits {
                    max_frame_pixels: MAX_PIXELS,
                    ..Default::default()
                },
                ctx.media_cancel_flag(),
                |frame| {
                    if ctx.cancelled() {
                        return Err(media::MediaError::Cancelled);
                    }
                    let pts = frame.source_pts();
                    // The frame covering a slice start may already have ended the last slice.
                    if times.last().is_some_and(|last| *last >= pts) {
                        return Ok(());
                    }
                    if times.len() >= MAX_FRAMES {
                        return Err(media::MediaError::TooLarge("footage correction frames"));
                    }
                    let pose = motion
                        .push(frame.rgba(), frame.width(), frame.height())
                        .ok_or(media::MediaError::Invalid("footage correction frame size"))?;
                    times.push(pts);
                    poses.push(pose);
                    ctx.progress(
                        JobStage::Analyzing,
                        ((pts.saturating_sub(self.range.start()).0.max(0) as f64 / total) as f32)
                            .clamp(0.0, 1.0),
                    )
                    .map_err(|_| media::MediaError::Cancelled)
                },
            )?;
            from = end;
        }
        if times.is_empty() {
            return Err(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "footage correction has no frames",
            )));
        }
        Track::new(motion.grid(), times, &poses).map_err(|_| {
            AnalysisError::Edit(crate::session::EditError::Invalid(
                "footage correction track",
            ))
        })
    }
}

impl CorrectionJob {
    /// Validate the retained output against its request and publish the cache entry once.
    /// The runner keeps the output, so `job_status` can still report the job afterwards.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<Arc<Track>>> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => Err(AnalysisError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(AnalysisError::Runtime(crate::RuntimeError::Failed(
                snapshot.error.ok_or(AnalysisError::from(
                    crate::session::SessionError::StaleResult,
                ))?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let track = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                Ok(Some(Arc::new(track)))
            }
            _ => Ok(None),
        }
    }
}

/// What the project's switched-on footage corrections resolve to against one cache.
#[derive(Clone, Default)]
pub struct Resolution {
    /// The analysed track of each item that has one, ready for `RenderSettings::stabilization`.
    pub tracks: render::stabilize::Tracks,
    /// Switched-on items with no stored correction: they render uncorrected, the properties row
    /// shows `Needs analysis` and export refuses to start naming them.
    pub missing: Vec<ItemId>,
}

/// Look up the stabilize correction of every switched-on video item in every scene. It never
/// decodes media and never fails a render: an item whose correction is absent, unreadable or
/// unsupported is listed as missing.
pub fn resolve(project: &Project, cache: &mut ArtifactCache) -> Resolution {
    let data = project.data();
    let mut resolution = Resolution::default();
    let mut decoded: BTreeMap<CacheKey, Option<Arc<Track>>> = BTreeMap::new();
    for item in data
        .scenes
        .iter()
        .flat_map(|scene| scene.timeline.iter())
        .flat_map(|track| &track.items)
    {
        let ItemParams::Video(params) = &item.content else {
            continue;
        };
        if params.stabilize.is_none() {
            continue;
        }
        let track = item
            .source_asset
            .and_then(|id| data.assets.iter().find(|asset| asset.id() == id))
            .and_then(|asset| CorrectionRequest::new(asset.clone(), CorrectionKind::Stabilize).ok())
            .and_then(|request| {
                decoded
                    .entry(request.cache_key())
                    .or_insert_with(|| request.cached(cache).ok().flatten())
                    .clone()
            });
        match track {
            Some(track) => {
                resolution.tracks.insert(item.id, track);
            }
            None => resolution.missing.push(item.id),
        }
    }
    resolution
}
