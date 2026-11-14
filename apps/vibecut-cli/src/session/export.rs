//! Admits CLI export jobs through the engine publication path.
use super::Context;
use crate::args::CliError;
use crate::output::wait_for_job;
use crate::output::{Output, json_path};
use crate::session::same_target;
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::control::Error as EngineError;
use engine::jobs::{JobId, JobRunner, RunnerConfig, SystemClock};
use project::Project;
use project::atomic::Overwrite;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use time::TimeRange;

#[derive(Clone, Copy)]
pub(crate) enum Aspect {
    Portrait,
    Landscape,
}

pub(crate) fn export_project(
    context: &Context,
    project_path: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
    options: render::OutputOptions,
) -> Result<Output, CliError> {
    export_configured(
        context,
        project_path,
        output,
        aspect,
        allow_degraded,
        overwrite,
        options,
        move |runner, key, project, _, settings, path| {
            let paths = options.paths(&project, &path)?;
            Ok(engine::submit_scene_exports_with_sources(
                runner,
                key,
                project,
                options,
                settings,
                paths,
                || Box::new(render::FileSources::default()),
            )?)
        },
    )
}

#[cfg(test)]
pub(crate) fn export_project_with(
    context: &Context,
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
    ) -> Result<(JobId, engine::jobs::CancelToken), EngineError>,
) -> Result<Output, CliError> {
    export_configured(
        context,
        project_path,
        output,
        aspect,
        allow_degraded,
        overwrite,
        render::OutputOptions::default(),
        submit,
    )
}

#[allow(clippy::too_many_arguments)]
fn export_configured(
    context: &Context,
    project_path: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
    options: render::OutputOptions,
    submit: impl FnOnce(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, engine::jobs::CancelToken), EngineError>,
) -> Result<Output, CliError> {
    let loaded = project::load(&project_path)?;
    let project = loaded.project;
    super::prepare_project_libraries(&project)?;
    if options.scene == render::SceneChoice::Active && project.duration() == time::Tick::ZERO {
        return Err(CliError::Rejected("project timeline is empty"));
    }
    // `<name>_%05d.png` selects a PNG sequence; its directory `<name>` is the output.
    let sequence = render::sequence_destination(&output)?;
    let output = sequence.clone().unwrap_or(output);
    if sequence.is_some() && options.scene == render::SceneChoice::All {
        return Err(render::Error::Unsupported("image sequence for all scenes").into());
    }
    let paths = options.paths(&project, &output)?;
    for path in &paths {
        if same_target(&project_path, path)? {
            return Err(CliError::Usage(
                "export output must not replace the project file".into(),
            ));
        }
    }
    let range = options
        .ranges(&project)?
        .first()
        .ok_or(CliError::Rejected("no scenes"))?
        .1;
    let mut settings = export_settings(&project, aspect, allow_degraded, overwrite);
    attach_corrections(context, &project, &mut settings)?;
    if sequence.is_some() {
        settings.image_sequence = true;
    } else {
        settings.container =
            render::OutputOptions::container_for_path(&output, options.audio_only)?;
    }
    options.apply(&mut settings)?;
    let key = export_job_key(&project, range, &settings)?;

    let budget = context.budget()?;
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
    let word_highlight_warnings = options.caption_word_warnings(
        &project,
        &settings,
        &engine::ByteBudgetHooks::new(runner.memory_budget(), None),
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
        "paths": paths.iter().map(|path| json_path(path)).collect::<Result<Vec<_>, _>>()?,
        "status": "completed",
        "word_highlight_warnings": word_highlight_warnings,
    });
    Ok(Output::new(
        value,
        format!("exported {}", output.display()),
        false,
        0,
    ))
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
        "{}x{}@{}/{}:{}:{}:{}:{:?}:{}:{}",
        settings.render.width,
        settings.render.height,
        settings.fps.num(),
        settings.fps.den(),
        settings.bit_rate,
        settings.quality,
        settings.include_audio,
        settings.container,
        settings.audio_only,
        settings.image_sequence,
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

/// Read the stored footage corrections of `project` into `settings` from the process media
/// cache. A project with no stabilized or matted clip opens no cache; a clip whose analysis is
/// absent stays uncorrected and the export job refuses, naming it.
pub(crate) fn attach_corrections(
    context: &Context,
    project: &Project,
    settings: &mut render::ExportSettings,
) -> Result<(), CliError> {
    let corrected = project
        .data()
        .scenes
        .iter()
        .flat_map(|scene| scene.timeline.iter())
        .flat_map(|track| &track.items)
        .any(|item| {
            matches!(&item.content, project::ItemParams::Video(p)
                if p.stabilize.is_some() || p.background_removal.is_some())
        });
    if corrected {
        let mut media = engine::control::MediaAccess::open(context.budget()?)?;
        media
            .footage_corrections(project)
            .attach(&mut settings.render);
    }
    Ok(())
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
