//! Runs footage analysis for stabilized and background-removed clips and tracks which of them
//! still need one.
use super::super::super::State;
use super::lut::Human;
use crate::editor::{Error, ItemId, Port, Result};
use engine::{
    Session,
    control::{ControlResult, MediaAccess},
    footage::CorrectionKind,
    jobs::{JobId, JobRunner, JobSnapshot, JobState},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// One clip's correction of one kind.
type Target = (ItemId, CorrectionKind);

/// What a correction's status row shows for one switched-on clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum FootageStatus {
    /// The stored correction is there; nothing to show.
    Ready,
    /// No stored correction: the clip renders without it and export refuses to start.
    NeedsAnalysis,
    /// An analysis job is running, 0..=1.
    Analyzing(f32),
    /// The analysis needs local weights that are not installed: the model manager's id.
    ModelMissing(&'static str),
}

/// The analysis jobs this area started and the last answer about stored corrections.
#[derive(Default)]
pub(in crate::editor) struct FootageWorkflow {
    access: Option<Arc<Mutex<MediaAccess>>>,
    budget: Option<engine::ByteBudget>,
    runner: Option<JobRunner>,
    running: BTreeMap<Target, JobId>,
    progress: BTreeMap<Target, f32>,
    /// Switched-on corrections with nothing stored, as of `checked`.
    missing: BTreeSet<Target>,
    /// Analyses refused because their model is not installed, with the model's id.
    models: BTreeMap<Target, &'static str>,
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
/// The model a refused analysis needs, when the refusal is the typed missing-model one.
fn missing_model(error: &Error) -> Option<&'static str> {
    use engine::providers::Error::ModelMissing;
    match error {
        Error::Engine(engine::control::Error::Providers(ModelMissing(id)))
        | Error::Engine(engine::control::Error::Analysis(engine::AnalysisError::Providers(
            ModelMissing(id),
        ))) => Some(id),
        _ => None,
    }
}
impl State {
    /// Adopt the process media cache the analysis writes to and the rows read from.
    pub(in crate::editor) fn attach_footage(
        &mut self,
        access: Arc<Mutex<MediaAccess>>,
        budget: engine::ByteBudget,
    ) {
        self.footage.access = Some(access);
        self.footage.budget = Some(budget);
        self.footage.checked = None;
    }

    /// What the status row of `kind` shows for `item`.
    pub(in crate::editor) fn footage_status(
        &self,
        item: ItemId,
        kind: CorrectionKind,
    ) -> FootageStatus {
        let target = (item, kind);
        if self.footage.running.contains_key(&target) {
            FootageStatus::Analyzing(self.footage.progress.get(&target).copied().unwrap_or(0.0))
        } else if !self.footage.missing.contains(&target) {
            FootageStatus::Ready
        } else if let Some(id) = self.footage.models.get(&target) {
            FootageStatus::ModelMissing(id)
        } else {
            FootageStatus::NeedsAnalysis
        }
    }

    /// Start the `kind` analysis of `item`'s source; a warm cache finishes at once. A refusal for
    /// a missing model is kept for the row, which then links to the model manager.
    pub(in crate::editor) fn request_footage(
        &mut self,
        port: &mut Port<'_>,
        item: ItemId,
        kind: CorrectionKind,
    ) -> Result<()> {
        let target = (item, kind);
        if self.footage.running.contains_key(&target) {
            return Ok(());
        }
        self.footage.models.remove(&target);
        let result = port.with_session(|session| {
            self.footage
                .control(session, engine::Command::AnalyzeFootage { item, kind })
        });
        let result = match result {
            Err(error) => {
                if let Some(id) = missing_model(&error) {
                    self.footage.models.insert(target, id);
                }
                return Err(error);
            }
            Ok(result) => result,
        };
        let ControlResult::JobAccepted { job, .. } = result else {
            return Err(Error::Invalid("footage analysis result"));
        };
        self.footage.running.insert(target, job);
        self.footage.progress.insert(target, 0.0);
        Ok(())
    }

    /// Stop the running analysis; the job ends cancelled and publishes nothing.
    pub(in crate::editor) fn cancel_footage(&mut self, item: ItemId, kind: CorrectionKind) {
        if let (Some(job), Some(runner)) = (
            self.footage.running.get(&(item, kind)),
            &mut self.footage.runner,
        ) {
            let _ = runner.cancel_job(*job);
        }
    }

    /// Advance the analysis jobs, publish the finished ones and refresh which corrections still
    /// need one. The frame ticks this once per frame.
    pub(in crate::editor) fn poll_footage(&mut self, port: &mut Port<'_>) -> Result<()> {
        let Some(access) = self.footage.access.clone() else {
            return Ok(());
        };
        if let Some(runner) = self.footage.runner.as_mut() {
            runner.tick()?;
            runner.drain_events();
        }
        let snapshots: Vec<(Target, JobSnapshot)> = match &self.footage.runner {
            Some(runner) => self
                .footage
                .running
                .iter()
                .filter_map(|(target, job)| {
                    runner.snapshot(*job).map(|snapshot| (*target, snapshot))
                })
                .collect(),
            None => Vec::new(),
        };
        self.footage
            .running
            .retain(|target, _| snapshots.iter().any(|(seen, _)| seen == target));
        let mut finished = false;
        for (target, snapshot) in snapshots {
            if !matches!(
                snapshot.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            ) {
                self.footage.progress.insert(target, snapshot.progress);
                continue;
            }
            // `job_status` publishes a completed correction and forgets a failed one.
            let job = snapshot.id;
            let outcome = port.with_session(|session| {
                self.footage
                    .control(session, engine::Command::JobStatus { job })
            });
            self.footage.running.remove(&target);
            self.footage.progress.remove(&target);
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
            let missing = &self.footage.missing;
            self.footage
                .models
                .retain(|target, _| missing.contains(target));
            self.footage.checked = Some((project, epoch));
        }
        if finished {
            // The preview re-reads the stored corrections on its next seek.
            port.sync();
        }
        Ok(())
    }
}
