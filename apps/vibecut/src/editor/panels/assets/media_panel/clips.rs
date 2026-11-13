//! Export clips from the Media tab: the selected video assets go to one registry
//! `export.clips` job after a folder pick. The engine writes and publishes the files;
//! this owner keeps the queued request, the job id and the cancel in flight.
use crate::editor::panels::assets::State;
use crate::{Error, Result, i18n::Text, services::ai_access};
use engine::{
    ByteBudget, Session,
    jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use media::AssetId;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// The one job the owner started and whether its cancel was requested.
struct Active {
    job: JobId,
    cancelling: bool,
}

/// Presentation owner for the Media tab's clip export.
pub(in crate::editor) struct ClipExports {
    runner: JobRunner,
    /// Assets waiting for the folder picker's answer.
    pending: Option<Vec<AssetId>>,
    active: Option<Active>,
}

/// What the strip shows for a running export.
#[derive(Clone, Copy)]
pub(in crate::editor) struct ClipStatus {
    pub(in crate::editor) progress: f32,
    pub(in crate::editor) cancelling: bool,
}

impl Drop for ClipExports {
    fn drop(&mut self) {
        self.runner.shutdown();
    }
}

impl ClipExports {
    fn new(budget: ByteBudget) -> Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 4,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(10),
                progress_interval: Duration::from_millis(20),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            runner,
            pending: None,
            active: None,
        })
    }
}

impl State {
    /// Queue the selected assets until the folder picker answers. Refused while a
    /// job runs so one queued request never races the running one.
    pub(in crate::editor) fn queue_clip_export(
        &mut self,
        budget: &ByteBudget,
        ids: Vec<AssetId>,
    ) -> Result<()> {
        if self.clip_exports.is_none() {
            self.clip_exports = Some(ClipExports::new(budget.clone())?);
        }
        let Some(exports) = self.clip_exports.as_mut() else {
            return Ok(());
        };
        if exports.active.is_some() {
            return Err(Error::Invalid("clip export already running"));
        }
        exports.pending = Some(ids);
        Ok(())
    }

    /// The folder picker was dismissed; the queued assets are not exported.
    pub(in crate::editor) fn cancel_clip_pick(&mut self) {
        if let Some(exports) = self.clip_exports.as_mut() {
            exports.pending = None;
        }
    }

    /// Start `export.clips` for the queued assets in `directory`. Without overwrite the
    /// engine refuses the whole job before the first render if a file already exists.
    pub(in crate::editor) fn start_clip_export(
        &mut self,
        session: &mut Session,
        directory: PathBuf,
    ) -> Result<()> {
        let Some(exports) = self.clip_exports.as_mut() else {
            return Ok(());
        };
        let Some(ids) = exports.pending.take() else {
            return Ok(());
        };
        let result = ai_access::run(
            session,
            &mut exports.runner,
            None,
            None,
            "export.clips",
            serde_json::json!({
                "asset_ids": ids,
                "dir": directory,
                "overwrite": false,
            }),
        )?;
        let (job, _) = ai_access::accepted(result)?;
        exports.active = Some(Active {
            job,
            cancelling: false,
        });
        Ok(())
    }

    /// Ask the engine to stop the running export. Files already published stay complete.
    pub(in crate::editor) fn cancel_clip_export(&mut self, session: &mut Session) -> Result<()> {
        let Some(exports) = self.clip_exports.as_mut() else {
            return Ok(());
        };
        let Some(active) = exports.active.as_mut() else {
            return Ok(());
        };
        let job = active.job;
        ai_access::run(
            session,
            &mut exports.runner,
            None,
            None,
            "job_cancel",
            serde_json::json!({ "job": job }),
        )?;
        active.cancelling = true;
        Ok(())
    }

    /// The running export's progress for the strip, if one runs.
    pub(in crate::editor) fn clip_export_status(&self) -> Option<ClipStatus> {
        let exports = self.clip_exports.as_ref()?;
        let active = exports.active.as_ref()?;
        let progress = exports
            .runner
            .snapshot(active.job)
            .map_or(0.0, |snapshot| snapshot.progress);
        Some(ClipStatus {
            progress: progress.clamp(0.0, 1.0),
            cancelling: active.cancelling,
        })
    }

    /// Advance the job. A finished one yields its toast text and detail exactly once.
    pub(in crate::editor) fn poll_clip_exports(&mut self) -> Option<(Text, Option<String>)> {
        let exports = self.clip_exports.as_mut()?;
        let job = exports.active.as_ref()?.job;
        if let Err(error) = exports.runner.tick() {
            exports.active = None;
            return Some((Text::ClipExportFailed, Some(error.to_string())));
        }
        let snapshot = exports.runner.snapshot(job)?;
        let report = match snapshot.state {
            JobState::Completed => (Text::ClipExportDone, None),
            JobState::Cancelled => (Text::ClipExportCancelled, None),
            JobState::Failed => (
                Text::ClipExportFailed,
                snapshot.error.map(|error| error.to_string()),
            ),
            _ => return None,
        };
        exports.active = None;
        Some(report)
    }
}
