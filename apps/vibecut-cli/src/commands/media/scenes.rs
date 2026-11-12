//! Reports scene-detection candidates for local media.
use crate::args::CliError;
use crate::args::utf8;
use crate::output::Output;
use crate::session::{Context, cli_runner, run_job};
use engine::control::{JobOutput, MediaAccess};
use engine::{Command, Session};
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;
use time::Tick;

/// Detect scene-change candidates for a local video; never edits a project.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    use engine::scene_detection::DetectionSettings;
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
    let id = asset.id();
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), String::new().into())?,
        1,
    )?;
    session.apply(Command::AddAsset { asset })?;
    let media = std::sync::Mutex::new(MediaAccess::open(context.budget()?)?);
    let mut runner = cli_runner(context.budget()?)?;
    let finished = run_job(
        &mut session,
        &mut runner,
        Some(&media),
        None,
        "media.detect_scenes",
        json!({"asset": id, "settings": settings}),
    );
    runner.shutdown();
    let Some(JobOutput::Scenes(result)) = finished?.output else {
        return Err(CliError::Rejected("scene job produced no result"));
    };
    let human = result
        .candidates
        .iter()
        .map(|c| {
            format!(
                "{} {:?} {:.3}
",
                c.at.0, c.kind, c.confidence
            )
        })
        .collect();
    Ok(Output::new(serde_json::to_value(result)?, human, false, 0))
}
