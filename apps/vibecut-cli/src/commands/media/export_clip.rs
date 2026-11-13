//! Downloads one timeline clip through the engine's `export.clip` job.
use crate::args::{CliError, parse_u64, utf8};
use crate::output::{Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner, export_job_key, prepare_project_fonts, same_target};
use engine::ClipExportMode;
use project::ItemId;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

/// `media export-clip <project> --clip <id> --mode clip|duration --out <path> [--overwrite]`
/// submits one engine job over the clip's own span and waits for it. The project is read
/// and never edited; `clip` renders only that clip, `duration` every track over its span.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut clip = None;
    let mut mode = None;
    let mut out = None;
    let mut overwrite = false;
    while let Some(flag) = args.next() {
        match utf8(&flag, "media export-clip flag")? {
            "--clip" => clip = Some(ItemId(parse_u64(args.next(), "--clip")?)),
            "--mode" => {
                let value = args.next().ok_or_else(usage)?;
                mode = Some(match utf8(&value, "--mode")? {
                    "clip" => ClipExportMode::Clip,
                    "duration" => ClipExportMode::Duration,
                    _ => {
                        return Err(CliError::Usage("--mode must be clip or duration".into()));
                    }
                });
            }
            "--out" => out = Some(PathBuf::from(args.next().ok_or_else(usage)?)),
            "--overwrite" => overwrite = true,
            "--help" | "-h" => return Ok(usage_output()),
            other => {
                return Err(CliError::Usage(format!(
                    "unknown media export-clip flag: {other}"
                )));
            }
        }
    }
    let (Some(clip), Some(mode), Some(path)) = (clip, mode, out) else {
        return Err(usage());
    };

    let project = project::load(&project_path)?.project;
    if same_target(&project_path, &path)? {
        return Err(CliError::Usage(
            "export output must not replace the project file".into(),
        ));
    }
    prepare_project_fonts(&project)?;
    let range = engine::clip_export_range(&project, clip)?;
    let key = export_job_key(
        &project,
        range,
        &render::ExportSettings::from_project(&project),
    )?;

    let budget = context.budget()?;
    let mut runner = cli_runner(budget)?;
    let (job, _cancel) = engine::submit_clip_export(
        &mut runner,
        key,
        Arc::new(project),
        clip,
        mode,
        overwrite,
        path.clone(),
    )?;
    wait_for_job(&mut runner, job)?;
    runner.shutdown();

    let mode_name = match mode {
        ClipExportMode::Clip => "clip",
        ClipExportMode::Duration => "duration",
    };
    let value = json!({
        "project": json_path(&project_path)?,
        "status": "completed",
        "clip": clip.0,
        "mode": mode_name,
        "paths": [json_path(&path)?],
    });
    let human = format!(
        "downloaded {mode_name} of clip {} to {}",
        clip.0,
        path.display()
    );
    Ok(Output::new(value, human, false, 0))
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}

fn usage_text() -> &'static str {
    "usage: vibecut-cli media export-clip <project> --clip <id> --mode <clip|duration> --out <path> [--overwrite]"
}

fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}
