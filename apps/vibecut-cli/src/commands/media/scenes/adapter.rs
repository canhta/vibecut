//! Reports scene-detection candidates for local media.
use crate::args::CliError;
use crate::args::utf8;
use crate::output::Output;
use crate::output::wait_for_job;
use crate::session::Context;
use crate::session::cli_runner;
use engine::cache::ArtifactCache;
use engine::subtitles;
use std::ffi::OsString;
use std::path::PathBuf;
use time::Tick;

/// Detect scene-change candidates for a local video; never edits a project.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    use engine::scene_detection::{DetectionRequest, DetectionSettings};
    let mut args = args.into_iter();
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Usage("scenes requires a file".into()))?;
    let mut settings = DetectionSettings::default();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| CliError::Usage("scene flag requires a number".into()))?;
        let value = utf8(&value, "scene setting")?;
        match utf8(&flag, "scene flag")? {
            "--threshold" => {
                settings.hard_threshold = value
                    .parse()
                    .map_err(|_| CliError::Usage("invalid threshold".into()))?
            }
            "--fade-threshold" => {
                settings.fade_threshold = value
                    .parse()
                    .map_err(|_| CliError::Usage("invalid fade threshold".into()))?
            }
            "--min-scene-ticks" => {
                settings.min_scene = Tick(
                    value
                        .parse()
                        .map_err(|_| CliError::Usage("invalid scene ticks".into()))?,
                )
            }
            _ => return Err(CliError::Usage("unknown scene flag".into())),
        }
    }
    let asset = media::SourceAsset::import(media::AssetId(1), std::fs::canonicalize(path)?)?;
    let duration = asset.probe().duration.ok_or(CliError::Unsupported(
        "scene input requires a known duration",
    ))?;
    let request = DetectionRequest::new(
        asset,
        time::SourceRange::new(Tick::ZERO, duration)?,
        settings,
    )?;
    let budget = context.budget()?;
    let (mut cache, _) =
        ArtifactCache::open(&subtitles::default_cache_root()?, budget.clone(), 128)?;
    let mut runner = cli_runner(budget)?;
    let job = request.submit(&mut runner, &mut cache)?;
    wait_for_job(&mut runner, job.id)?;
    let result = job
        .finish(&mut runner, &mut cache)?
        .ok_or(CliError::Rejected("scene job produced no result"))?;
    runner.shutdown();
    let human = result
        .candidates
        .iter()
        .map(|c| format!("{} {:?} {:.3}\n", c.at.0, c.kind, c.confidence))
        .collect();
    Ok(Output::new(serde_json::to_value(result)?, human, false, 0))
}
