//! Executes the admitted export jobs in file order.
use super::{PreparedBatch, PreparedJob};
use crate::output::json_path;
use crate::output::{progress_percent, stage_name};
use crate::session::Context;
use crate::session::{export_job_key, export_settings};
use crate::{CliError, Output};
use engine::EditError;
use engine::cache::CacheKey;
use engine::jobs::{CancelToken, JobRunner, JobState};
use engine::jobs::{JobId, JobStage};
use engine::jobs::{RunnerConfig, SystemClock};
use project::Project;
use serde_json::{Value, json};
use std::io;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use time::{Tick, TimeRange};

pub(super) fn execute<F>(
    context: &Context,
    batch: PreparedBatch,
    mut submit: F,
) -> Result<Output, CliError>
where
    F: FnMut(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, CancelToken), EditError>,
{
    let budget = context.budget()?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(20),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?;
    let total = batch.jobs.len();
    let mut values = Vec::new();
    values
        .try_reserve_exact(total)
        .map_err(|_| CliError::Engine(EditError::Limit))?;
    let mut human = Vec::new();
    let mut exit = 0;
    for (index, job) in batch.jobs.iter().enumerate() {
        if exit != 0 {
            values.push(job_value(job, index, "skipped", None)?);
            human.push(format!("job {}/{}: skipped", index + 1, total));
            continue;
        }
        let project = batch
            .projects
            .get(&job.project_target)
            .cloned()
            .ok_or(CliError::Rejected("batch project disappeared"))?;
        let outcome = run_job(&mut runner, job, project, index, total, &mut submit);
        match outcome {
            Ok(()) => {
                values.push(job_value(job, index, "completed", None)?);
                human.push(format!(
                    "job {}/{}: exported {}",
                    index + 1,
                    total,
                    job.output.display()
                ));
            }
            Err(error) => {
                exit = error.exit_code();
                values.push(job_value(job, index, "failed", Some(&error))?);
                human.push(format!("job {}/{}: failed: {error}", index + 1, total));
            }
        }
    }
    runner.shutdown();
    let status = if exit == 0 { "completed" } else { "failed" };
    Ok(Output::new(
        json!({"status": status, "jobs": values}),
        human.join("\n"),
        false,
        exit,
    ))
}

fn job_value(
    job: &PreparedJob,
    index: usize,
    status: &'static str,
    error: Option<&CliError>,
) -> Result<Value, CliError> {
    let mut value = json!({
        "index": index,
        "project": json_path(&job.project)?,
        "output": json_path(&job.output)?,
        "status": status,
    });
    if let Some(error) = error
        && let Value::Object(fields) = &mut value
    {
        let _ = fields.insert("error".into(), error.value());
    }
    Ok(value)
}

fn run_job<F>(
    runner: &mut JobRunner,
    job: &PreparedJob,
    project: Arc<Project>,
    index: usize,
    total: usize,
    submit: &mut F,
) -> Result<(), CliError>
where
    F: FnMut(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, CancelToken), EditError>,
{
    let duration = project.duration();
    if duration <= Tick::ZERO {
        return Err(CliError::Rejected("project timeline is empty"));
    }
    let range = TimeRange::new(Tick::ZERO, duration)?;
    let settings = export_settings(&project, job.aspect, job.allow_degraded, job.overwrite);
    let key = export_job_key(&project, range, &settings)?;
    let (id, _cancel) = submit(runner, key, project, range, settings, job.output.clone())?;
    wait_for_job(runner, id, index, total)
}

fn wait_for_job(
    runner: &mut JobRunner,
    id: JobId,
    index: usize,
    total: usize,
) -> Result<(), CliError> {
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
                    "job {}/{}: progress {} {}%",
                    index + 1,
                    total,
                    stage_name(event.stage),
                    percent
                );
            }
        }
        let snapshot = runner
            .snapshot(id)
            .ok_or(CliError::Rejected("export job disappeared"))?;
        match snapshot.state {
            JobState::Completed => {
                let _ = runner.take_output(id);
                return Ok(());
            }
            JobState::Failed => {
                return Err(match snapshot.error {
                    Some(error) => CliError::Job(error),
                    None => CliError::Rejected("export failed without diagnostics"),
                });
            }
            JobState::Cancelled => {
                return Err(CliError::Job(Arc::new(EditError::Runtime(
                    engine::RuntimeError::Cancelled,
                ))));
            }
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use project::ProjectId;
    use project::{Item, ItemId, ItemParams, OverlayParams, Project, Track, TrackId, TrackKind};
    use std::{collections::BTreeMap, fs};

    fn overlay_project() -> Project {
        let mut data = Project::empty(ProjectId(0), "batch".into())
            .unwrap()
            .into_data();
        data.canvas.width = 64;
        data.canvas.height = 64;
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(1),
            name: "Overlay".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: ItemId(0),
                source_asset: None,
                source_range: TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Overlay(OverlayParams {
                    color: project::Color([30, 160, 90, 255]),
                }),
            }],
        });
        Project::new(data).unwrap()
    }

    #[test]
    fn cancellation_leaves_no_partial_output() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().join("project.vibecut");
        let output_path = dir.path().join("output.mp4");
        let job = PreparedJob {
            project: project_path.clone(),
            project_target: project_path.clone(),
            output: output_path.clone(),
            aspect: None,
            allow_degraded: false,
            overwrite: false,
        };
        let mut projects = BTreeMap::new();
        projects.insert(project_path, Arc::new(overlay_project()));
        let batch = PreparedBatch {
            jobs: vec![job],
            projects,
        };
        let result = execute(
            &Context::default(),
            batch,
            |runner, key, project, range, settings, path| {
                let (id, token) =
                    engine::submit_export(runner, key, project, range, settings, path)?;
                token.cancel();
                Ok((id, token))
            },
        )
        .unwrap();
        assert_eq!(result.exit_code(), 7);
        assert_eq!(result.value()["status"], "failed");
        assert_eq!(result.value()["jobs"][0]["status"], "failed");
        assert!(!output_path.exists());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().starts_with(".vibecut-export-"))
            .collect();
        assert!(leftovers.is_empty(), "staging orphan: {leftovers:?}");
    }
}
