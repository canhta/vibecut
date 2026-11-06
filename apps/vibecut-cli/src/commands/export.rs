use crate::arguments::utf8;
use crate::exit_codes::CliError;
use crate::job_runtime::CLI_MEMORY_LIMIT;
use crate::output::{Output, json_path};
use crate::paths::same_target;
use crate::progress::wait_for_job;
use engine::EditError;
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{JobId, JobRunner, RunnerConfig, SystemClock};
use project::Project;
use project::atomic::Overwrite;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use time::{Tick, TimeRange};

pub(crate) fn export(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(export_usage)?;
    let mut output = None;
    let mut aspect = None;
    let mut allow_degraded = false;
    let mut overwrite = false;
    while let Some(arg) = args.next() {
        match utf8(&arg, "export flag")? {
            "--out" => {
                output = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--out requires an output path".into())
                })?));
            }
            "--aspect" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--aspect requires 9:16 or 16:9".into()))?;
                aspect = Some(match utf8(&value, "aspect")? {
                    "9:16" => Aspect::Portrait,
                    "16:9" => Aspect::Landscape,
                    _ => {
                        return Err(CliError::Usage("--aspect must be 9:16 or 16:9".into()));
                    }
                });
            }
            "--allow-degraded" => allow_degraded = true,
            "--overwrite" => overwrite = true,
            other => {
                return Err(CliError::Usage(format!("unknown export flag: {other}")));
            }
        }
    }
    let output = output.ok_or_else(export_usage)?;
    export_project(project_path, output, aspect, allow_degraded, overwrite)
}

fn export_usage() -> CliError {
    CliError::Usage(
        "usage: vibecut-cli export <project> --out <mp4> [--aspect 9:16] [--allow-degraded] [--overwrite]"
            .into(),
    )
}

#[derive(Clone, Copy)]
pub(crate) enum Aspect {
    Portrait,
    Landscape,
}

fn export_project(
    project_path: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
) -> Result<Output, CliError> {
    export_project_with(
        project_path,
        output,
        aspect,
        allow_degraded,
        overwrite,
        engine::submit_export,
    )
}

pub(crate) fn export_project_with(
    project_path: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
    submit: impl FnOnce(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, engine::jobs::CancelToken), EditError>,
) -> Result<Output, CliError> {
    let loaded = project::load(&project_path)?;
    let project = loaded.project;
    if same_target(&project_path, &output)? {
        return Err(CliError::Usage(
            "export output must not replace the project file".into(),
        ));
    }
    let duration = project.duration();
    if duration <= Tick::ZERO {
        return Err(CliError::Rejected("project timeline is empty"));
    }
    let range = TimeRange::new(Tick::ZERO, duration)?;
    let settings = export_settings(&project, aspect, allow_degraded, overwrite);
    let key = export_job_key(&project, range, &settings)?;

    let budget = engine::ByteBudget::new(CLI_MEMORY_LIMIT)?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(20),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?;
    let (id, _cancel) = submit(
        &mut runner,
        key,
        Arc::new(project),
        range,
        settings,
        output.clone(),
    )?;
    wait_for_job(&mut runner, id)?;
    runner.shutdown();
    let value = json!({
        "path": json_path(&output)?,
        "status": "completed",
    });
    Ok(Output {
        silent: false,
        exit: 0,
        value,
        human: format!("exported {}", output.display()),
    })
}

pub(crate) fn export_job_key(
    project: &Project,
    range: TimeRange,
    settings: &render::ExportSettings,
) -> Result<CacheKey, CliError> {
    let project_bytes = project::encode(project, false)?;
    let source = media::ContentHash::from_bytes(&project_bytes).0;
    let clip_range =
        media::ContentHash::from_bytes(format!("{}:{}", range.start().0, range.end().0).as_bytes())
            .0;
    let settings_key = format!(
        "{}x{}@{}/{}:{}:{}:{}",
        settings.render.width,
        settings.render.height,
        settings.fps.num(),
        settings.fps.den(),
        settings.bit_rate,
        settings.quality,
        settings.include_audio,
    );
    // Export has no persistent artifact kind; this key only identifies the job.
    Ok(CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source,
        provider: "vibecut-cli",
        model: "render",
        version: "1",
        settings: settings_key.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range,
        translation: [0; 32],
    })?)
}

pub(crate) fn export_settings(
    project: &Project,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
) -> render::ExportSettings {
    let mut settings = render::ExportSettings::from_project(project);
    settings.render = match aspect {
        Some(Aspect::Portrait) => render::RenderSettings::portrait(project),
        Some(Aspect::Landscape) => {
            let mut render = render::RenderSettings::from_project(project);
            render.width = 1920;
            render.height = 1080;
            render
        }
        None => render::RenderSettings::from_project(project),
    };
    settings.allow_degraded = allow_degraded;
    settings.overwrite = if overwrite {
        Overwrite::Replace
    } else {
        Overwrite::Refuse
    };
    settings
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use project::ProjectId;
    #[test]
    fn portrait_and_landscape_override_only_output_geometry() {
        let project = Project::empty(ProjectId(0), "".into()).unwrap();
        let portrait = export_settings(&project, Some(Aspect::Portrait), false, false);
        assert_eq!(
            (portrait.render.width, portrait.render.height),
            (1080, 1920)
        );
        let landscape = export_settings(&project, Some(Aspect::Landscape), false, false);
        assert_eq!(
            (landscape.render.width, landscape.render.height),
            (1920, 1080)
        );
        assert_eq!(landscape.fps, project.data().canvas.fps);
        assert!(!landscape.allow_degraded);
        assert!(matches!(landscape.overwrite, Overwrite::Refuse));
    }
}
