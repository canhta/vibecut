//! Runs footage analysis for stabilized clips and tracks which of them still need one.
use super::super::super::State;
use super::lut::Human;
use crate::editor::{Error, ItemId, Port, Result};
use engine::{
    Session,
    control::{ControlResult, MediaAccess},
    jobs::{JobId, JobRunner, JobSnapshot, JobState},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// What the Stabilize row shows for one switched-on clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum FootageStatus {
    /// The stored correction is there; nothing to show.
    Ready,
    /// No stored correction: the clip renders uncorrected and export refuses to start.
    NeedsAnalysis,
    /// An analysis job is running, 0..=1.
    Analyzing(f32),
}

/// The analysis jobs this area started and the last answer about stored corrections.
#[derive(Default)]
pub(in crate::editor) struct FootageWorkflow {
    access: Option<Arc<Mutex<MediaAccess>>>,
    budget: Option<engine::ByteBudget>,
    runner: Option<JobRunner>,
    running: BTreeMap<ItemId, JobId>,
    progress: BTreeMap<ItemId, f32>,
    /// Switched-on clips with no stored correction, as of `checked`.
    missing: BTreeSet<ItemId>,
    /// The project snapshot and cache epoch `missing` was read for.
    checked: Option<(Arc<project::Project>, u64)>,
}
impl FootageWorkflow {
    fn control(
        &mut self,
        session: &mut Session,
        command: engine::Command,
    ) -> Result<ControlResult> {
        if self.runner.is_none() {
            self.runner = Some(JobRunner::new(
                engine::jobs::RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 4,
                    retained: 8,
                    network_attempts: 1,
                    backoff: std::time::Duration::from_millis(100),
                    progress_interval: std::time::Duration::from_millis(40),
                },
                self.budget
                    .clone()
                    .ok_or(Error::Invalid("footage budget unavailable"))?,
                Arc::new(engine::jobs::SystemClock::default()),
            )?);
        }
        let access = self
            .access
            .as_ref()
            .ok_or(Error::Invalid("footage cache unavailable"))?;
        Ok(session.control_command(
            command,
            engine::control::Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Human,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: Some(access.as_ref()),
                chapters: None,
                captions: None,
                vision: None,
            },
            self.runner
                .as_mut()
                .ok_or(Error::Invalid("footage runner unavailable"))?,
        )?)
    }
}
impl State {
    /// Adopt the process media cache the analysis writes to and the row reads from.
    pub(in crate::editor) fn attach_footage(
        &mut self,
        access: Arc<Mutex<MediaAccess>>,
        budget: engine::ByteBudget,
    ) {
        self.footage.access = Some(access);
        self.footage.budget = Some(budget);
        self.footage.checked = None;
    }

    /// What the Stabilize row shows for `item`.
    pub(in crate::editor) fn footage_status(&self, item: ItemId) -> FootageStatus {
        if self.footage.running.contains_key(&item) {
            FootageStatus::Analyzing(self.footage.progress.get(&item).copied().unwrap_or(0.0))
        } else if self.footage.missing.contains(&item) {
            FootageStatus::NeedsAnalysis
        } else {
            FootageStatus::Ready
        }
    }

    /// Start the stabilize analysis of `item`'s source; a warm cache completes at once.
    pub(in crate::editor) fn request_footage(
        &mut self,
        port: &mut Port<'_>,
        item: ItemId,
    ) -> Result<()> {
        if self.footage.running.contains_key(&item) {
            return Ok(());
        }
        let result = port.with_session(|session| {
            self.footage.control(
                session,
                engine::Command::AnalyzeFootage {
                    item,
                    kind: engine::footage::CorrectionKind::Stabilize,
                },
            )
        })?;
        let ControlResult::JobAccepted { job, .. } = result else {
            return Err(Error::Invalid("footage analysis result"));
        };
        self.footage.running.insert(item, job);
        self.footage.progress.insert(item, 0.0);
        Ok(())
    }

    /// Stop the running analysis of `item`; the job ends cancelled and publishes nothing.
    pub(in crate::editor) fn cancel_footage(&mut self, item: ItemId) {
        if let (Some(job), Some(runner)) =
            (self.footage.running.get(&item), &mut self.footage.runner)
        {
            let _ = runner.cancel_job(*job);
        }
    }

    /// Advance the analysis jobs, publish the finished ones and refresh which clips still need
    /// one. The frame ticks this once per frame.
    pub(in crate::editor) fn poll_footage(&mut self, port: &mut Port<'_>) -> Result<()> {
        let Some(access) = self.footage.access.clone() else {
            return Ok(());
        };
        if let Some(runner) = self.footage.runner.as_mut() {
            runner.tick()?;
            runner.drain_events();
        }
        let snapshots: Vec<(ItemId, JobSnapshot)> = match &self.footage.runner {
            Some(runner) => self
                .footage
                .running
                .iter()
                .filter_map(|(item, job)| runner.snapshot(*job).map(|snapshot| (*item, snapshot)))
                .collect(),
            None => Vec::new(),
        };
        self.footage
            .running
            .retain(|item, _| snapshots.iter().any(|(seen, _)| seen == item));
        let mut finished = false;
        for (item, snapshot) in snapshots {
            if !matches!(
                snapshot.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            ) {
                self.footage.progress.insert(item, snapshot.progress);
                continue;
            }
            // `job_status` publishes a completed correction and forgets a failed one.
            let job = snapshot.id;
            let outcome = port.with_session(|session| {
                self.footage
                    .control(session, engine::Command::JobStatus { job })
            });
            self.footage.running.remove(&item);
            self.footage.progress.remove(&item);
            self.footage.checked = None;
            finished = true;
            if let Err(error) = outcome {
                port.fail(error.to_string());
            } else if let (JobState::Failed, Some(error)) = (snapshot.state, snapshot.error) {
                port.fail(
                    Error::Engine(engine::control::Error::Runtime(
                        engine::RuntimeError::Failed(error),
                    ))
                    .to_string(),
                );
            }
        }
        let project = port.view().session.project().clone();
        let epoch = access
            .lock()
            .map_err(|_| Error::Invalid("media cache poisoned"))?
            .footage_epoch();
        let fresh = matches!(
            &self.footage.checked,
            Some((seen, at)) if Arc::ptr_eq(seen, &project) && *at == epoch
        );
        if !fresh {
            let resolution = access
                .lock()
                .map_err(|_| Error::Invalid("media cache poisoned"))?
                .footage_corrections(&project);
            self.footage.missing = resolution.missing.into_iter().collect();
            self.footage.checked = Some((project, epoch));
        }
        if finished {
            // The preview re-reads the stored corrections on its next seek.
            port.sync();
        }
        Ok(())
    }
}
