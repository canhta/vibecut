//! Lends process media caches and durable LUT sources to generation/import jobs.
use crate::{
    EditError,
    artifacts::thumbnail_strip::{
        self, ThumbnailStripJob, ThumbnailStripRequest, ThumbnailStripState,
    },
    artifacts::thumbnails::ThumbnailCache,
    cache::ArtifactCache,
    jobs::{JobId, JobRunner},
    scene_detection::{DetectionJob, DetectionRequest, DetectionSettings},
    waveform::{self, WaveformJob, WaveformRequest, WaveformState},
};
use media::{SourceAsset, THUMBNAIL_MAX_EDGE};
use project::Item;
use std::collections::BTreeMap;
use time::{SourceRange, Tick, TimeRange};

/// Entries each cache may retain; the CLI and desktop composition roots agree.
const MAX_CACHE_ENTRIES: usize = 128;

/// One process's media stores and the generation jobs they have not published yet.
/// The composition root builds it from the same caches its display queries read.
pub struct MediaAccess {
    artifacts: ArtifactCache,
    luts: crate::store::luts::LutStore,
    thumbnails: ThumbnailCache,
    waveforms: BTreeMap<JobId, WaveformJob>,
    strips: BTreeMap<JobId, ThumbnailStripJob>,
    scenes: BTreeMap<JobId, DetectionJob>,
}

impl MediaAccess {
    /// The process waveform/scene-analysis and timeline-thumbnail stores only,
    /// at most 256 registered entries total. No source/model/library roots.
    pub fn cache_maintenance(&self) -> Result<[crate::cache::CacheMaintenance; 2], EditError> {
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
            return Err(EditError::Control(
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
        }
    }

    /// Open process waveform/thumbnail caches and the separate durable LUT store
    /// under the shared root. One owner holds them so generation and display read the same
    /// entries; a second open of the same root reports [`crate::RuntimeError::CacheBusy`]
    /// through `EditError::Runtime`.
    pub fn open(budget: crate::ByteBudget) -> Result<Self, EditError> {
        Self::open_in(&crate::cache::CacheRoot::discover()?.models(), budget)
    }

    /// Open the caches and durable LUT store under an explicit root. The desktop keeps them with
    /// its workspace, while the headless CLI uses the shared cache root.
    pub fn open_in(root: &std::path::Path, budget: crate::ByteBudget) -> Result<Self, EditError> {
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
    ) -> Result<WaveformState, EditError> {
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
    ) -> Result<ThumbnailStripState, EditError> {
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
    ) -> Result<JobId, EditError> {
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
    ) -> Result<JobId, EditError> {
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
    ) -> Result<JobId, EditError> {
        self.retain(runner);
        let duration = asset.probe().duration.ok_or(EditError::Unsupported(
            "scene detection needs a known duration",
        ))?;
        let range = SourceRange::new(Tick::ZERO, duration)?;
        let job =
            DetectionRequest::new(asset, range, settings)?.submit(runner, &mut self.artifacts)?;
        let id = job.id;
        self.scenes.insert(id, job);
        Ok(id)
    }

    /// Publish a completed job's validated output. A job this owner no longer
    /// holds was already published or was not submitted here.
    pub(crate) fn finish(&mut self, runner: &mut JobRunner, job: JobId) -> Result<(), EditError> {
        if let Some(handle) = self.waveforms.remove(&job) {
            handle.finish(runner, &mut self.artifacts)?;
        }
        if let Some(handle) = self.strips.remove(&job) {
            handle.finish(runner, &mut self.thumbnails)?;
        }
        if let Some(handle) = self.scenes.remove(&job) {
            handle.finish(runner, &mut self.artifacts)?;
        }
        Ok(())
    }

    /// Drop a cancelled or failed job without publishing it.
    pub(crate) fn forget(&mut self, job: JobId) {
        self.waveforms.remove(&job);
        self.strips.remove(&job);
        self.scenes.remove(&job);
    }

    fn retain(&mut self, runner: &JobRunner) {
        self.waveforms
            .retain(|id, _| runner.snapshot(*id).is_some());
        self.strips.retain(|id, _| runner.snapshot(*id).is_some());
        self.scenes.retain(|id, _| runner.snapshot(*id).is_some());
    }
}
