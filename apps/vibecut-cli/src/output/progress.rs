//! Observes engine jobs through stderr progress.
use crate::args::CliError;
use engine::control::Error as EngineError;
use engine::jobs::{JobId, JobRunner, JobStage, JobState};
use std::io;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn wait_for_job(runner: &mut JobRunner, id: JobId) -> Result<(), CliError> {
    let mut last = (JobStage::Queued, u32::MAX);
    loop {
        runner.tick()?;
        for event in runner.drain_events() {
            if event.id != id {
                continue;
            }
            let percent = progress_percent(event.progress);
            if last != (event.stage, percent) {
                last = (event.stage, percent);
                let _ = writeln!(
                    io::stderr(),
                    "progress {} {}%",
                    stage_name(event.stage),
                    percent
                );
            }
        }
        let snapshot = runner
            .snapshot(id)
            .ok_or(CliError::Rejected("export job disappeared"))?;
        match snapshot.state {
            JobState::Completed => return Ok(()),
            JobState::Failed => {
                return Err(match snapshot.error {
                    Some(error) => CliError::Job(error),
                    None => CliError::Rejected("export failed without diagnostics"),
                });
            }
            JobState::Cancelled => {
                return Err(CliError::Job(Arc::new(EngineError::Runtime(
                    engine::RuntimeError::Cancelled,
                ))));
            }
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

pub(crate) fn progress_percent(progress: f32) -> u32 {
    if progress.is_finite() {
        (progress.clamp(0.0, 1.0) * 100.0).round() as u32
    } else {
        0
    }
}

pub(crate) fn stage_name(stage: JobStage) -> &'static str {
    match stage {
        JobStage::Extract => "extract",
        JobStage::Segment => "segment",
        JobStage::Queued => "queued",
        JobStage::Starting => "starting",
        JobStage::Preparing => "preparing",
        JobStage::Analyzing => "analyzing",
        JobStage::Transcribe => "transcribe",
        JobStage::Align => "align",
        JobStage::Translate => "translate",
        JobStage::SuggestClips => "suggest_clips",
        JobStage::Assemble => "assemble",
        JobStage::Process => "process",
        JobStage::Probe => "probe",
        JobStage::Thumbnail => "thumbnail",
        JobStage::Admit => "admit",
        JobStage::Render => "render",
        JobStage::Mix => "mix",
        JobStage::Encode => "encode",
        JobStage::Finalize => "finalize",
        JobStage::Download => "download",
        JobStage::Verify => "verify",
        JobStage::Publish => "publish",
        JobStage::Backoff => "backoff",
        JobStage::Complete => "complete",
    }
}
