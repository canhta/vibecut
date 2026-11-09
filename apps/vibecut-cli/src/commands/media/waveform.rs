//! Warms one project asset's waveform summary.
use crate::args::{CliError, parse_u64, utf8};
use crate::output::{Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner, waveform_cache};
use engine::waveform::WaveformRequest;
use media::AssetId;
use project::WriterLease;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;

/// `media waveform <project> --asset <id>` generates and caches the source peak
/// summary and prints its layout.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut asset = None;
    while let Some(flag) = args.next() {
        match utf8(&flag, "media waveform flag")? {
            "--asset" => asset = Some(AssetId(parse_u64(args.next(), "--asset")?)),
            "--help" | "-h" => return Ok(usage_output()),
            other => {
                return Err(CliError::Usage(format!(
                    "unknown media waveform flag: {other}"
                )));
            }
        }
    }
    let asset = asset.ok_or_else(usage)?;

    let lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let source = loaded
        .project
        .data()
        .assets
        .iter()
        .find(|candidate| candidate.id() == asset)
        .cloned()
        .ok_or(CliError::Rejected("project has no such asset"))?;
    let budget = context.budget()?;
    let mut cache = waveform_cache(budget.clone())?;
    let mut runner = cli_runner(budget)?;
    let request = WaveformRequest::new(source)?;
    let job = request.submit(&mut runner, &mut cache)?;
    wait_for_job(&mut runner, job.id)?;
    let summary = job
        .finish(&mut runner, &mut cache)?
        .ok_or(CliError::Rejected("waveform produced no result"))?;
    runner.shutdown();

    let value = json!({
        "project": json_path(&project_path)?,
        "asset": asset.0,
        "sample_rate": summary.sample_rate(),
        "channels": summary.channels(),
        "total_samples": summary.total_samples(),
        "buckets": summary.bucket_count(),
    });
    let human = format!(
        "asset {}: {} Hz, {} channel(s), {} buckets",
        asset.0,
        summary.sample_rate(),
        summary.channels(),
        summary.bucket_count()
    );
    Ok(Output::new(value, human, false, 0))
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}

fn usage_text() -> &'static str {
    "usage: vibecut-cli media waveform <project> --asset <id>"
}

fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}
