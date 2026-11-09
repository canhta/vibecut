//! Warms one clip's visible-range thumbnail strip.
use crate::args::{CliError, parse_i64, parse_u64, utf8};
use crate::output::{Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner, thumbnail_cache};
use engine::thumbnail_strip::{GENERATION_INTERVAL, ThumbnailStripRequest};
use media::THUMBNAIL_MAX_EDGE;
use project::{ItemId, WriterLease};
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;
use time::{Tick, TimeRange};

/// `media thumbnails <project> --item <id> [--start <tick> --end <tick>]`
/// generates and caches the strip frames the clip's visible range needs.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut item = None;
    let mut start = None;
    let mut end = None;
    while let Some(flag) = args.next() {
        match utf8(&flag, "media thumbnails flag")? {
            "--item" => item = Some(ItemId(parse_u64(args.next(), "--item")?)),
            "--start" => start = Some(Tick(parse_i64(args.next(), "--start")?)),
            "--end" => end = Some(Tick(parse_i64(args.next(), "--end")?)),
            "--help" | "-h" => return Ok(usage_output()),
            other => {
                return Err(CliError::Usage(format!(
                    "unknown media thumbnails flag: {other}"
                )));
            }
        }
    }
    let item = item.ok_or_else(usage)?;
    let visible = match (start, end) {
        (None, None) => None,
        (Some(start), Some(end)) => Some(TimeRange::new(start, end)?),
        _ => return Err(usage()),
    };

    let lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let data = loaded.project.data();
    let clip = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|candidate| candidate.id == item)
        .cloned()
        .ok_or(CliError::Rejected("project has no such item"))?;
    let asset_id = clip
        .source_asset
        .ok_or(CliError::Rejected("item has no source asset"))?;
    let source = data
        .assets
        .iter()
        .find(|candidate| candidate.id() == asset_id)
        .cloned()
        .ok_or(CliError::Rejected("project has no such asset"))?;
    let visible = match visible {
        Some(visible) => visible,
        None => TimeRange::new(clip.start, clip.end()?)?,
    };
    let request = ThumbnailStripRequest::for_clip(
        source,
        &clip,
        visible,
        GENERATION_INTERVAL,
        THUMBNAIL_MAX_EDGE,
    )?;
    let budget = context.budget()?;
    let mut cache = thumbnail_cache(budget.clone())?;
    let mut runner = cli_runner(budget)?;
    let job = request.submit(&mut runner, &mut cache)?;
    wait_for_job(&mut runner, job.id)?;
    let strip = job
        .finish(&mut runner, &mut cache)?
        .ok_or(CliError::Rejected("thumbnails produced no result"))?;
    runner.shutdown();

    let positions: Vec<i64> = strip.positions().iter().map(|at| at.0).collect();
    let value = json!({
        "project": json_path(&project_path)?,
        "item": item.0,
        "asset": asset_id.0,
        "positions": positions,
    });
    let human = format!(
        "item {}: {} thumbnail tile(s) at edge {}",
        item.0,
        positions.len(),
        THUMBNAIL_MAX_EDGE
    );
    Ok(Output::new(value, human, false, 0))
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}

fn usage_text() -> &'static str {
    "usage: vibecut-cli media thumbnails <project> --item <id> \
     [--start <tick> --end <tick>]"
}

fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}
