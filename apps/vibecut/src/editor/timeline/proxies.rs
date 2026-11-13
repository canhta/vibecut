//! Keeps the preview proxies of the project's video sources current.
use crate::editor::context::{ProxyAction, ProxyPhase, ProxyUi};
use engine::{
    control::MediaAccess,
    jobs::{JobId, JobRunner, JobState},
    proxies::ProxyIndex,
};
use media::SourceAsset;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

/// One queue of proxy jobs, run one at a time on the media owner's runner.
#[derive(Default)]
pub(super) struct Proxies {
    queue: VecDeque<SourceAsset>,
    running: Option<(JobId, SourceAsset)>,
    ui: ProxyUi,
    /// Contents whose cached proxy was already offered to preview this round.
    offered: BTreeSet<[u8; 32]>,
}

/// Stills and audio-only files have no proxy.
fn has_video(asset: &SourceAsset) -> bool {
    !asset.is_image()
        && asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == media::StreamKind::Video)
}

/// Video sources, once per content.
fn video_sources(assets: &[SourceAsset]) -> Vec<SourceAsset> {
    let mut seen = BTreeSet::new();
    assets
        .iter()
        .filter(|asset| has_video(asset) && seen.insert(asset.content_hash().0))
        .cloned()
        .collect()
}

impl Proxies {
    pub(super) fn set_index(&mut self, index: Option<ProxyIndex>) {
        self.ui.index = index;
        self.offered.clear();
    }

    pub(super) fn ui(&self) -> ProxyUi {
        self.ui.clone()
    }

    /// The runner must exist before a generation is queued.
    pub(super) fn request(
        &mut self,
        action: ProxyAction,
        assets: &[SourceAsset],
        access: Option<&Arc<Mutex<MediaAccess>>>,
    ) {
        self.offered.clear();
        match action {
            // The choice lives on the session; a turn-on re-offers cached proxies.
            ProxyAction::Toggle(_) => {}
            ProxyAction::Generate => {
                let Some(index) = &self.ui.index else {
                    return;
                };
                self.queue = video_sources(assets)
                    .into_iter()
                    .filter(|asset| !index.contains(asset.content_hash()))
                    .collect();
                self.ui.done = 0;
                self.ui.total = self.queue.len() + usize::from(self.running.is_some());
                self.ui.cause = None;
                self.ui.phase = if self.ui.total == 0 {
                    ProxyPhase::Idle
                } else {
                    ProxyPhase::Generating
                };
            }
            ProxyAction::Cancel => {
                self.queue.clear();
                if let (Some((job, _)), Some(access)) = (&self.running, access)
                    && let Ok(access) = access.lock()
                {
                    access.cancel_proxy(*job);
                }
                if self.running.is_none() {
                    self.ui.phase = ProxyPhase::Idle;
                }
            }
        }
    }

    /// Advance the running job, start the next queued one and offer cached
    /// proxies of the project's sources to preview while the choice is on.
    pub(super) fn tick(
        &mut self,
        access: &Arc<Mutex<MediaAccess>>,
        runner: Option<&mut JobRunner>,
        assets: &[SourceAsset],
    ) {
        let Some(index) = self.ui.index.clone() else {
            return;
        };
        if let Some(runner) = runner {
            if let Some((job, asset)) = self.running.clone() {
                match runner.snapshot(job) {
                    Some(snapshot)
                        if matches!(
                            snapshot.state,
                            JobState::Completed | JobState::Cancelled | JobState::Failed
                        ) =>
                    {
                        self.settle(access, runner, job, &asset, &index);
                    }
                    Some(snapshot) => self.ui.progress = snapshot.progress,
                    None => self.running = None,
                }
            }
            if self.running.is_none()
                && let Some(asset) = self.queue.pop_front()
                && let Ok(mut access) = access.lock()
            {
                match access.generate_proxy(runner, asset.clone()) {
                    Ok(job) => {
                        self.ui.progress = 0.0;
                        self.running = Some((job, asset));
                    }
                    Err(error) => self.fail(error),
                }
            }
            if self.running.is_none()
                && self.queue.is_empty()
                && self.ui.phase == ProxyPhase::Generating
            {
                self.ui.phase = if self.ui.cause.is_some() {
                    ProxyPhase::Failed
                } else {
                    ProxyPhase::Idle
                };
            }
        }
        if index.enabled()
            && let Ok(mut access) = access.try_lock()
        {
            for asset in assets.iter().filter(|asset| has_video(asset)) {
                if index.contains(asset.content_hash())
                    || !self.offered.insert(asset.content_hash().0)
                {
                    continue;
                }
                // A missing or corrupt cache entry keeps the source playing.
                let _ = access.adopt_proxy(asset, &index);
            }
        }
    }

    fn settle(
        &mut self,
        access: &Arc<Mutex<MediaAccess>>,
        runner: &mut JobRunner,
        job: JobId,
        asset: &SourceAsset,
        index: &ProxyIndex,
    ) {
        self.running = None;
        let Ok(mut access) = access.lock() else {
            return;
        };
        match access.finish_proxy(runner, job) {
            Ok(Some(_)) => {
                self.ui.done += 1;
                if let Err(error) = access.adopt_proxy(asset, index) {
                    self.fail(error);
                }
            }
            Ok(None) => {}
            Err(engine::ArtifactError::Runtime(engine::RuntimeError::Cancelled)) => {
                self.queue.clear();
            }
            Err(error) => self.fail(error),
        }
    }

    fn fail(&mut self, error: engine::ArtifactError) {
        self.ui.cause.get_or_insert_with(|| Arc::new(error));
    }
}
