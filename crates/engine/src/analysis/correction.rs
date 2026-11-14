//! Computes footage corrections once per source and looks them up by key.
mod flicker;
mod motion;
mod person;

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
use render::{flicker::FlickerTrack, matte::MatteTrack, stabilize::Track};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// Versioned recipe of the stabilize correction; part of every stabilize cache key.
pub(crate) const RECIPE: &str = "stabilize-v1:luma480-block32-r8-sad-parabola:sim4-irls:chain";
/// Versioned recipe of the flicker measurement; part of every flicker cache key. The window,
/// cut threshold and gain range live in [`render::flicker::RECIPE`].
pub(crate) const FLICKER_RECIPE: &str = "flicker-v1:linear-luma709-mean:grid80x45-cut0.25";
/// The pinned segmentation weights behind the person matte.
pub const PERSON_MODEL: &str = "pphumanseg-2023mar";
/// Inference threads of the person segmenter. It is fixed, and part of the matte key, so the
/// same source always gives the same matte.
pub const SEGMENTATION_THREADS: usize = 2;
/// The retained flicker result: the largest track the render crate admits.
const MAX_FLICKER_BYTES: usize = 13 * render::flicker::MAX_FRAMES + 12;
/// Largest decoded frame the analysis admits: 16 Mi pixels.
const MAX_PIXELS: u64 = 1 << 24;
/// The retained stabilize result: the largest track the render crate admits.
const MAX_TRACK_BYTES: usize = 24 * render::stabilize::MAX_FRAMES + 20;

/// A loaded local person segmenter: one call per prepared frame. The composition root builds it
/// from the pinned weights; engine never downloads them.
pub type Segmenter = Box<
    dyn FnMut(
            ai::vision::PersonFrame<'_>,
            &dyn ai::ExecutionHooks,
        ) -> std::result::Result<ai::vision::PersonMatte, ai::Error>
        + Send,
>;

/// One stored correction of one source.
#[derive(Clone, Debug, PartialEq)]
pub enum Correction {
    /// The measured camera pose of every frame.
    Stabilize(Arc<Track>),
    /// The person probability plane of every frame.
    PersonMatte(Arc<MatteTrack>),
    /// The mean brightness and shot starts of every frame.
    Flicker(Arc<FlickerTrack>),
}

/// One source's correction request. The key binds the source content, the whole-source range,
/// the kind and the recipe (for the matte also the pinned weights and thread count); the item's
/// trim, its level or feather and its position never enter it, so a split or a level change
/// reuses the entry.
#[derive(Clone)]
pub struct CorrectionRequest {
    asset: SourceAsset,
    kind: CorrectionKind,
    range: SourceRange,
    key: CacheKey,
}

/// A retained runner job. `finish` caches only a complete, validated correction and returns it.
pub struct CorrectionJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: CorrectionRequest,
}

impl CorrectionRequest {
    /// Admit one source whose video has a known duration and a frame of at most 16 Mi pixels.
    /// Stabilize also needs a frame that holds one analysis block.
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
        if kind == CorrectionKind::Stabilize && Motion::new(shape.width, shape.height).is_none() {
            return Err(invalid("footage correction frame is too small"));
        }
        let duration = asset
            .probe()
            .duration
            .ok_or_else(|| invalid("footage correction needs a known duration"))?;
        let range = SourceRange::new(Tick::ZERO, duration)?;
        let settings_error =
            || AnalysisError::Edit(crate::session::EditError::Invalid("correction settings"));
        let (model, version, settings) = match kind {
            CorrectionKind::Stabilize => (
                "luma-block-match",
                RECIPE.to_owned(),
                serde_json::to_vec(&(kind, range)).map_err(|_| settings_error())?,
            ),
            CorrectionKind::PersonMatte => {
                let weights = crate::models::catalog_model(PERSON_MODEL)
                    .map_err(|_| invalid("person matte weights are not in the catalog"))?
                    .files
                    .first()
                    .map(|file| file.sha256.clone())
                    .ok_or_else(|| invalid("person matte weights are not in the catalog"))?;
                (
                    PERSON_MODEL,
                    format!("{}|{}", ai::vision::PERSON_RECIPE, render::matte::RECIPE),
                    serde_json::to_vec(&(kind, range, weights, SEGMENTATION_THREADS))
                        .map_err(|_| settings_error())?,
                )
            }
            CorrectionKind::Flicker => (
                "luma-mean",
                format!("{FLICKER_RECIPE}|{}", render::flicker::RECIPE),
                serde_json::to_vec(&(kind, range)).map_err(|_| settings_error())?,
            ),
        };
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::FootageCorrection,
            source: asset.content_hash().0,
            provider: "local",
            model,
            version: &version,
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
    pub fn cached(&self, cache: &mut ArtifactCache) -> Result<Option<Correction>> {
        match cache.get(self.key) {
            Ok(Some(lease)) => Ok(self.decode(lease.bytes()).ok()),
            Ok(None) | Err(crate::RuntimeError::CacheCorrupt) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Queue one CPU job. A warm entry completes without opening the source or the model; only
    /// `finish` publishes a complete cache entry. A cold person matte needs `segmenter` and
    /// refuses with `ModelMissing` without one.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
        segmenter: Option<Segmenter>,
    ) -> Result<CorrectionJob> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => self
                .decode(lease.bytes())
                .ok()
                .map(|_| lease.bytes().to_vec()),
            Ok(None) | Err(crate::RuntimeError::CacheCorrupt) => None,
            Err(error) => return Err(error.into()),
        };
        let segmenter = match (self.kind, &cached, segmenter) {
            (CorrectionKind::PersonMatte, None, None) => {
                return Err(AnalysisError::Providers(
                    crate::providers::Error::ModelMissing(PERSON_MODEL),
                ));
            }
            (_, _, segmenter) => segmenter.map(Mutex::new),
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
                    result: self.max_bytes() as u64,
                    ..Default::default()
                },
            },
            move |ctx| -> Result<Vec<u8>> {
                ctx.check_cancelled()?;
                if let Some(bytes) = &cached {
                    return Ok(bytes.clone());
                }
                let bytes = match request.kind {
                    CorrectionKind::Stabilize => request.analyze(ctx)?.encode(),
                    CorrectionKind::Flicker => request.analyze_flicker(ctx)?.encode(),
                    CorrectionKind::PersonMatte => {
                        let mut segmenter = segmenter
                            .as_ref()
                            .ok_or(AnalysisError::Providers(
                                crate::providers::Error::ModelMissing(PERSON_MODEL),
                            ))?
                            .lock()
                            .map_err(|_| AnalysisError::Runtime(crate::RuntimeError::Poisoned))?;
                        request.analyze_matte(ctx, &mut segmenter)?.encode()
                    }
                };
                ctx.check_cancelled()?;
                Ok(bytes)
            },
        )?;
        Ok(CorrectionJob {
            id,
            cancel,
            request: self.clone(),
        })
    }

    fn max_bytes(&self) -> usize {
        match self.kind {
            CorrectionKind::Stabilize => MAX_TRACK_BYTES,
            CorrectionKind::PersonMatte => render::matte::MAX_BYTES,
            CorrectionKind::Flicker => MAX_FLICKER_BYTES,
        }
    }

    fn decode(&self, bytes: &[u8]) -> crate::runtime::Result<Correction> {
        if bytes.len() > self.max_bytes() {
            return Err(crate::RuntimeError::CacheCorrupt);
        }
        match self.kind {
            CorrectionKind::Stabilize => Track::decode(bytes)
                .map(|track| Correction::Stabilize(Arc::new(track)))
                .map_err(|_| crate::RuntimeError::CacheCorrupt),
            CorrectionKind::PersonMatte => MatteTrack::decode(bytes)
                .map(|matte| Correction::PersonMatte(Arc::new(matte)))
                .map_err(|_| crate::RuntimeError::CacheCorrupt),
            CorrectionKind::Flicker => FlickerTrack::decode(bytes)
                .map(|track| Correction::Flicker(Arc::new(track)))
                .map_err(|_| crate::RuntimeError::CacheCorrupt),
        }
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
                    if times.len() >= render::stabilize::MAX_FRAMES {
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
    ) -> Result<Option<Correction>> {
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
                let correction = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                Ok(Some(correction))
            }
            _ => Ok(None),
        }
    }
}

/// What the project's switched-on footage corrections resolve to against one cache.
#[derive(Clone, Default)]
pub struct Resolution {
    /// The analysed track of each stabilized item that has one.
    pub tracks: render::stabilize::Tracks,
    /// The stored matte of each item with background removal on that has one.
    pub mattes: render::matte::Mattes,
    /// The stored brightness track of each item with flicker removal on that has one.
    pub flicker: render::flicker::Tracks,
    /// Switched-on corrections with nothing stored, in item order: the item renders without
    /// that correction, its properties row shows `Needs analysis` and export refuses to start
    /// naming it.
    pub missing: Vec<(ItemId, CorrectionKind)>,
}
impl Resolution {
    /// Hand the stored corrections to render.
    pub fn attach(self, settings: &mut render::RenderSettings) {
        settings.stabilization = self.tracks;
        settings.person_mattes = self.mattes;
        settings.flicker = self.flicker;
    }
}

/// Look up the stored corrections of every switched-on video item in every scene. It never
/// decodes media and never fails a render: a correction that is absent, unreadable or
/// unsupported is listed as missing.
pub fn resolve(project: &Project, cache: &mut ArtifactCache) -> Resolution {
    let data = project.data();
    let mut resolution = Resolution::default();
    let mut decoded: BTreeMap<CacheKey, Option<Correction>> = BTreeMap::new();
    for item in data
        .scenes
        .iter()
        .flat_map(|scene| scene.timeline.iter())
        .flat_map(|track| &track.items)
    {
        let ItemParams::Video(params) = &item.content else {
            continue;
        };
        let kinds = [
            (CorrectionKind::Stabilize, params.stabilize.is_some()),
            (
                CorrectionKind::PersonMatte,
                params.background_removal.is_some(),
            ),
            (CorrectionKind::Flicker, params.remove_flicker),
        ];
        for (kind, on) in kinds {
            if !on {
                continue;
            }
            let stored = item
                .source_asset
                .and_then(|id| data.assets.iter().find(|asset| asset.id() == id))
                .and_then(|asset| CorrectionRequest::new(asset.clone(), kind).ok())
                .and_then(|request| {
                    decoded
                        .entry(request.cache_key())
                        .or_insert_with(|| request.cached(cache).ok().flatten())
                        .clone()
                });
            match stored {
                Some(Correction::Stabilize(track)) => {
                    resolution.tracks.insert(item.id, track);
                }
                Some(Correction::PersonMatte(matte)) => {
                    resolution.mattes.insert(item.id, matte);
                }
                Some(Correction::Flicker(track)) => {
                    resolution.flicker.insert(item.id, track);
                }
                None => resolution.missing.push((item.id, kind)),
            }
        }
    }
    resolution
}
