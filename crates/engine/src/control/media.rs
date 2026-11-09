//! Lends the process artifact caches media generation jobs read and publish through.
use crate::{
    EditError,
    artifacts::thumbnail_strip::{ThumbnailStripJob, ThumbnailStripRequest},
    artifacts::thumbnails::ThumbnailCache,
    cache::ArtifactCache,
    jobs::{JobId, JobRunner},
    waveform::{WaveformJob, WaveformRequest},
};
use media::{SourceAsset, THUMBNAIL_MAX_EDGE};
use project::Item;
use std::collections::BTreeMap;
use time::TimeRange;

/// One process's media caches and the generation jobs they have not published yet.
/// The composition root builds it from the same caches its display queries read.
pub struct MediaAccess {
    artifacts: ArtifactCache,
    thumbnails: ThumbnailCache,
    waveforms: BTreeMap<JobId, WaveformJob>,
    strips: BTreeMap<JobId, ThumbnailStripJob>,
}

impl MediaAccess {
    pub fn new(artifacts: ArtifactCache, thumbnails: ThumbnailCache) -> Self {
        Self {
            artifacts,
            thumbnails,
            waveforms: BTreeMap::new(),
            strips: BTreeMap::new(),
        }
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

    /// Publish a completed job's validated output. A job this owner no longer
    /// holds was already published or was not submitted here.
    pub(crate) fn finish(&mut self, runner: &mut JobRunner, job: JobId) -> Result<(), EditError> {
        if let Some(handle) = self.waveforms.remove(&job) {
            handle.finish(runner, &mut self.artifacts)?;
        }
        if let Some(handle) = self.strips.remove(&job) {
            handle.finish(runner, &mut self.thumbnails)?;
        }
        Ok(())
    }

    /// Drop a cancelled or failed job without publishing it.
    pub(crate) fn forget(&mut self, job: JobId) {
        self.waveforms.remove(&job);
        self.strips.remove(&job);
    }

    fn retain(&mut self, runner: &JobRunner) {
        self.waveforms
            .retain(|id, _| runner.snapshot(*id).is_some());
        self.strips.retain(|id, _| runner.snapshot(*id).is_some());
    }
}
