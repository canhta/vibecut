use crate::exit_codes::CliError;
use engine::EditError;
use engine::jobs::JobId;
use engine::jobs::JobRunner;
use engine::jobs::JobSnapshot;
use engine::jobs::JobStage;
use engine::jobs::JobState;
use engine::subtitles::Progress;
use engine::subtitles::Stage;
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
                return Err(CliError::Job(Arc::new(EditError::Cancelled)));
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

pub(crate) struct CliProgress {
    pub(crate) last: std::sync::Mutex<(JobStage, u32)>,
}
impl Progress for CliProgress {
    fn stage(&self, stage: Stage) {
        let _ = writeln!(io::stderr(), "stage {}", subtitle_stage_name(stage));
    }
    fn job(&self, snapshot: &JobSnapshot) {
        let percent = progress_percent(snapshot.progress);
        if let Ok(mut last) = self.last.lock()
            && *last != (snapshot.stage, percent)
        {
            *last = (snapshot.stage, percent);
            let _ = writeln!(
                io::stderr(),
                "progress {} {}%",
                stage_name(snapshot.stage),
                percent
            );
        }
    }
}
fn subtitle_stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Extract => "extract",
        Stage::Transcribe => "transcribe",
        Stage::Align => "align",
        Stage::Assemble => "assemble",
        Stage::Segment => "segment",
        Stage::SuggestClips => "suggest_clips",
        Stage::Snap => "snap",
        Stage::Translate => "translate",
        Stage::ExportSrt => "export_srt",
        Stage::ExportBurn => "export_burn",
    }
}
