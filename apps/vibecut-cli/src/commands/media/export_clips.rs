//! Exports whole source assets as clip files through the engine's `export.clips` job.
use crate::args::{CliError, parse_u64, utf8};
use crate::output::{Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner, export_job_key};
use media::AssetId;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;

/// `media export-clips <project> --asset <id>... --dir <dir> [--overwrite]` submits one
/// engine job that writes each asset's full source to `<dir>`, and waits for it.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut assets = Vec::new();
    let mut directory = None;
    let mut overwrite = false;
    while let Some(flag) = args.next() {
        match utf8(&flag, "media export-clips flag")? {
            "--asset" => assets.push(AssetId(parse_u64(args.next(), "--asset")?)),
            "--dir" => {
                directory = Some(PathBuf::from(args.next().ok_or_else(usage)?));
            }
            "--overwrite" => overwrite = true,
            "--help" | "-h" => return Ok(usage_output()),
            other => {
                return Err(CliError::Usage(format!(
                    "unknown media export-clips flag: {other}"
                )));
            }
        }
    }
    let directory = directory.ok_or_else(usage)?;
    if assets.is_empty() {
        return Err(usage());
    }

    let project = project::load(&project_path)?.project;
    let plan = engine::plan_asset_exports(&project, &assets, &directory, overwrite)?;
    let first = plan
        .first()
        .ok_or(CliError::Rejected("no assets to export"))?;
    let key = export_job_key(&project, first.range, &first.settings)?;
    let paths: Vec<PathBuf> = plan.iter().map(|export| export.path.clone()).collect();

    let budget = context.budget()?;
    let mut runner = cli_runner(budget)?;
    let (job, _cancel) = engine::submit_asset_exports(&mut runner, key, plan)?;
    wait_for_job(&mut runner, job)?;
    runner.shutdown();

    let value = json!({
        "project": json_path(&project_path)?,
        "status": "completed",
        "paths": paths
            .iter()
            .map(|path| json_path(path))
            .collect::<Result<Vec<_>, _>>()?,
    });
    let human = format!(
        "exported {} clip(s) to {}",
        paths.len(),
        directory.display()
    );
    Ok(Output::new(value, human, false, 0))
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}

fn usage_text() -> &'static str {
    "usage: vibecut-cli media export-clips <project> --asset <id> [--asset <id>...] --dir <dir> [--overwrite]"
}

fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}
