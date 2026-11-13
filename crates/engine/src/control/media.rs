//! Lends process media caches and durable LUT sources to generation/import jobs.
use super::{Error as EngineError, Result};
use crate::{
    analysis::{
        AnalysisJob, AnalysisRequest, AnalysisSettings, ReframeJob, ReframeRequest, ReframeTarget,
    },
    artifacts::thumbnail_strip::{
        self, ThumbnailStripJob, ThumbnailStripRequest, ThumbnailStripState,
    },
    artifacts::thumbnails::ThumbnailCache,
    cache::ArtifactCache,
    jobs::{JobId, JobKind, JobRunner, JobState},
    scene_detection::{DetectionJob, DetectionRequest, DetectionSettings},
    waveform::{self, WaveformJob, WaveformRequest, WaveformState},
};
use ai::vision::{FaceFrame, IdentityFrame, SceneFrame, SceneResult, SpeakerChunk, VisionDetector};
use media::{SourceAsset, THUMBNAIL_MAX_EDGE};
use project::{Item, ItemId};
use std::collections::BTreeMap;
use time::{SourceRange, Tick, TimeRange};

/// Entries each cache may retain; the CLI and desktop composition roots agree.
const MAX_CACHE_ENTRIES: usize = 128;

/// A submitted reframe analysis and the item it was generated for, so
/// `reframe.apply { job }` knows which item the job names.
struct ReframeEntry {
    job: ReframeJob,
    item: ItemId,
    published: bool,
}

/// Defers to the boxed adapter; `ReframeRequest::submit` accepts a sized value.
struct BoxedDetector(Box<dyn VisionDetector>);
impl VisionDetector for BoxedDetector {
    fn embedding(
        &mut self,
        frame: IdentityFrame<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<Option<ai::vision::FaceEmbedding>, ai::Error> {
        self.0.embedding(frame, hooks)
    }
    fn faces(
        &mut self,
        frame: FaceFrame<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<Vec<ai::vision::FaceBox>, ai::Error> {
        self.0.faces(frame, hooks)
    }
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<Vec<f32>, ai::Error> {
        self.0.speaker(chunk, hooks)
    }
    fn scene(
        &mut self,
        frame: SceneFrame<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<SceneResult, ai::Error> {
        self.0.scene(frame, hooks)
    }
}

/// One process's media stores and the generation jobs they have not published yet.
/// The composition root builds it from the same caches its display queries read.
pub struct MediaAccess {
    artifacts: ArtifactCache,
    luts: crate::store::luts::LutStore,
    thumbnails: ThumbnailCache,
    waveforms: BTreeMap<JobId, WaveformJob>,
    strips: BTreeMap<JobId, ThumbnailStripJob>,
    scenes: BTreeMap<JobId, DetectionJob>,
    reframes: BTreeMap<JobId, ReframeEntry>,
    /// Shot analyses a clip export started, with their cache identity, until published.
    analyses: BTreeMap<JobId, ([u8; 32], AnalysisJob)>,
}

impl MediaAccess {
    /// The process waveform/scene-analysis and timeline-thumbnail stores only,
    /// at most 256 registered entries total. No source/model/library roots.
    pub fn cache_maintenance(&self) -> Result<[crate::cache::CacheMaintenance; 2]> {
        let snapshots = [
            self.artifacts.maintenance()?,
            self.thumbnails.maintenance()?,
        ];
        if snapshots
            .iter()
            .map(crate::cache::CacheMaintenance::entry_count)
            .sum::<usize>()
            > 256
        {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        Ok(snapshots)
    }

    pub fn new(
        artifacts: ArtifactCache,
        thumbnails: ThumbnailCache,
        luts: crate::store::luts::LutStore,
    ) -> Self {
        Self {
            artifacts,
            luts,
            thumbnails,
            waveforms: BTreeMap::new(),
            strips: BTreeMap::new(),
            scenes: BTreeMap::new(),
            reframes: BTreeMap::new(),
            analyses: BTreeMap::new(),
        }
    }

    /// Open process waveform/thumbnail caches and the separate durable LUT store
    /// under the shared root. One owner holds them so generation and display read the same
    /// entries; a second open of the same root reports [`crate::RuntimeError::CacheBusy`]
    /// through `EngineError::Runtime`.
    pub fn open(budget: crate::ByteBudget) -> Result<Self> {
        Self::open_in(&crate::cache::CacheRoot::discover()?.models(), budget)
    }

    /// Open the caches and durable LUT store under an explicit root. The desktop keeps them with
    /// its workspace, while the headless CLI uses the shared cache root.
    pub fn open_in(root: &std::path::Path, budget: crate::ByteBudget) -> Result<Self> {
        let artifacts =
            ArtifactCache::open(&root.join("waveform"), budget.clone(), MAX_CACHE_ENTRIES)?.0;
        let thumbnails = ThumbnailCache::open(&root.join("thumbnails"), budget, MAX_CACHE_ENTRIES)?;
        let luts = crate::store::luts::LutStore::open(&root.join("luts"))?;
        Ok(Self::new(artifacts, thumbnails, luts))
    }

    pub(crate) fn lut_store(&self) -> crate::store::luts::LutStore {
        self.luts.clone()
    }

    /// Cache-only waveform bars for one clip's visible range. Maps trim and
    /// retime and reads the same summary [`Self::generate_waveform`] publishes.
    pub fn waveform_display(
        &mut self,
        asset: &SourceAsset,
        item: &Item,
        visible: TimeRange,
        columns: usize,
    ) -> crate::analysis::Result<WaveformState> {
        waveform::display(asset, item, visible, columns, &mut self.artifacts)
    }

    /// Cache-only thumbnail tiles for one clip's visible range at a timeline
    /// zoom. Reads the same frame cache [`Self::generate_clip_thumbnails`]
    /// publishes.
    pub fn thumbnail_strip_display(
        &mut self,
        asset: &SourceAsset,
        item: &Item,
        visible: TimeRange,
        pixels_per_second: f64,
    ) -> crate::artifacts::Result<ThumbnailStripState> {
        thumbnail_strip::display(
            asset,
            item,
            visible,
            pixels_per_second,
            THUMBNAIL_MAX_EDGE,
            &mut self.thumbnails,
        )
    }

    /// Submit one source's peak summary. A warm entry completes without opening
    /// the source.
    pub(crate) fn generate_waveform(
        &mut self,
        runner: &mut JobRunner,
        asset: SourceAsset,
    ) -> crate::analysis::Result<JobId> {
        self.retain(runner);
        let job = WaveformRequest::new(asset)?.submit(runner, &mut self.artifacts)?;
        let id = job.id;
        self.waveforms.insert(id, job);
        Ok(id)
    }

    /// Submit the source frames one clip's visible range needs at a timeline
    /// zoom. A warm cache completes without opening the source.
    pub(crate) fn generate_clip_thumbnails(
        &mut self,
        runner: &mut JobRunner,
        asset: SourceAsset,
        item: &Item,
        visible: TimeRange,
        pixels_per_second: f64,
    ) -> crate::artifacts::Result<JobId> {
        self.retain(runner);
        let request = ThumbnailStripRequest::for_clip(
            asset,
            item,
            visible,
            pixels_per_second,
            THUMBNAIL_MAX_EDGE,
        )?;
        let job = request.submit(runner, &mut self.thumbnails)?;
        let id = job.id;
        self.strips.insert(id, job);
        Ok(id)
    }

    /// Submit scene detection over a source's whole video. A warm entry completes
    /// without opening the source; the entry is published when `job_status` first
    /// sees the job complete.
    pub(crate) fn detect_scenes(
        &mut self,
        runner: &mut JobRunner,
        asset: SourceAsset,
        settings: DetectionSettings,
    ) -> crate::analysis::Result<JobId> {
        self.retain(runner);
        let duration = asset
            .probe()
            .duration
            .ok_or(crate::session::EditError::Unsupported(
                "scene detection needs a known duration",
            ))?;
        let range = SourceRange::new(Tick::ZERO, duration)?;
        let job =
            DetectionRequest::new(asset, range, settings)?.submit(runner, &mut self.artifacts)?;
        let id = job.id;
        self.scenes.insert(id, job);
        Ok(id)
    }

    /// Submit one item's reframe analysis. A warm entry completes without
    /// opening the source or calling the detector; the entry is published when
    /// `job_status` first sees the job complete. The item is retained so
    /// `reframe.apply { job }` can name it without reopening the job payload.
    #[allow(
        clippy::too_many_arguments,
        reason = "one control admission carrying the runner, session and authored request pieces"
    )]
    pub(crate) fn generate_reframe(
        &mut self,
        runner: &mut JobRunner,
        session: &crate::Session,
        asset: SourceAsset,
        item: Item,
        target: ReframeTarget,
        models: [u8; 32],
        settings: AnalysisSettings,
        detector: Box<dyn VisionDetector>,
    ) -> crate::analysis::Result<JobId> {
        self.retain(runner);
        let item_id = item.id;
        let request = ReframeRequest::new(asset, item.source_range, target, models, settings)?;
        let job = request.submit_for_session(
            runner,
            &mut self.artifacts,
            BoxedDetector(detector),
            session,
        )?;
        let id = job.id;
        self.reframes.insert(
            id,
            ReframeEntry {
                job,
                item: item_id,
                published: false,
            },
        );
        Ok(id)
    }

    /// The item a reframe job was generated for; `None` after eviction.
    pub(crate) fn reframe_item(&self, job: JobId) -> Option<ItemId> {
        self.reframes.get(&job).map(|entry| entry.item)
    }

    /// Publish a completed job's validated output. A job this owner no longer
    /// holds was already published or was not submitted here.
    pub(crate) fn finish(&mut self, runner: &mut JobRunner, job: JobId) -> Result<()> {
        if let Some(handle) = self.waveforms.remove(&job) {
            handle.finish(runner, &mut self.artifacts)?;
        }
        if let Some(handle) = self.strips.remove(&job) {
            handle.finish(runner, &mut self.thumbnails)?;
        }
        if let Some(handle) = self.scenes.remove(&job) {
            handle.finish(runner, &mut self.artifacts)?;
        }
        self.publish_analyses(runner);
        // Keep the reframe handle: `reframe.apply` reads the same retained
        // output after `job_status` has already published the cache entry.
        if let Some(entry) = self.reframes.get_mut(&job)
            && !entry.published
            && runner
                .snapshot(job)
                .is_some_and(|snapshot| snapshot.kind == JobKind::AnalyzeReframe)
            && entry.job.publish(runner, &mut self.artifacts)?.is_some()
        {
            entry.published = true;
        }
        Ok(())
    }

    /// Publish finished shot analyses into the cache and drop failed ones, so the next
    /// lookup of their request is a hit or a fresh submission.
    fn publish_analyses(&mut self, runner: &mut JobRunner) {
        let ids: Vec<JobId> = self.analyses.keys().copied().collect();
        for id in ids {
            match runner.snapshot(id).map(|snapshot| snapshot.state) {
                Some(JobState::Completed) => {
                    if let Some((_, job)) = self.analyses.get(&id) {
                        // A failed publication leaves the entry uncached: the next export
                        // reports it pending again and resubmits.
                        let _ = job.finish(runner, &mut self.artifacts);
                    }
                    self.analyses.remove(&id);
                }
                Some(JobState::Failed | JobState::Cancelled) | None => {
                    self.analyses.remove(&id);
                }
                Some(_) => {}
            }
        }
    }

    /// The per-shot analysis for a clip export: the cached result of `request` when there
    /// is one, else its running job (submitted here when none runs) with empty shots.
    pub(crate) fn clip_shots(
        &mut self,
        runner: &mut JobRunner,
        request: &AnalysisRequest,
        detector: Box<dyn VisionDetector>,
    ) -> (
        Vec<crate::ShotAnalysis>,
        Option<crate::session::wire::ClipShotsDiagnostic>,
    ) {
        use crate::session::wire::ClipShotsDiagnostic::{Failed, Pending};
        self.retain(runner);
        self.publish_analyses(runner);
        match request.cached(&mut self.artifacts) {
            Ok(Some(result)) => return (result.shots, None),
            Ok(None) => {}
            Err(_) => return (Vec::new(), Some(Failed)),
        }
        let key = request.key_bytes();
        if let Some(id) = self
            .analyses
            .iter()
            .find_map(|(id, (running, _))| (*running == key).then_some(*id))
        {
            return (Vec::new(), Some(Pending { job: id }));
        }
        match request.submit(runner, &mut self.artifacts, BoxedDetector(detector)) {
            Ok(job) => {
                let id = job.id;
                self.analyses.insert(id, (key, job));
                (Vec::new(), Some(Pending { job: id }))
            }
            Err(_) => (Vec::new(), Some(Failed)),
        }
    }

    /// Drop a cancelled or failed job without publishing it.
    pub(crate) fn forget(&mut self, job: JobId) {
        self.waveforms.remove(&job);
        self.strips.remove(&job);
        self.scenes.remove(&job);
        self.reframes.remove(&job);
        self.analyses.remove(&job);
    }

    fn retain(&mut self, runner: &JobRunner) {
        self.waveforms
            .retain(|id, _| runner.snapshot(*id).is_some());
        self.strips.retain(|id, _| runner.snapshot(*id).is_some());
        self.scenes.retain(|id, _| runner.snapshot(*id).is_some());
        self.reframes.retain(|id, _| runner.snapshot(*id).is_some());
        self.analyses.retain(|id, _| runner.snapshot(*id).is_some());
    }
}
