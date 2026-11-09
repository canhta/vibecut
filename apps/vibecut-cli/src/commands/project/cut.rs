//! Detects and removes pauses from a leased project.
use crate::args::{CliError, parse_i64, utf8};
use crate::output::{Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner};
use engine::Session;
use engine::pause_candidates::{PauseCandidatesRequest, PauseSettings};
use project::WriterLease;
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

/// `cut <project> --remove-pauses [--min-pause-ms N] [--pad-ms N]
/// [--crossfade-ms N] [--dry-run]` analyses the active scene, then applies the
/// reviewed spans as one undoable edit.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(cut_usage)?;
    let mut settings = PauseSettings::default();
    let mut dry_run = false;
    let mut remove = false;
    while let Some(flag) = args.next() {
        match utf8(&flag, "cut flag")? {
            "--remove-pauses" => remove = true,
            "--dry-run" => dry_run = true,
            "--min-pause-ms" => settings.min_pause_ms = parse_i64(args.next(), "--min-pause-ms")?,
            "--pad-ms" => settings.pad_ms = parse_i64(args.next(), "--pad-ms")?,
            "--crossfade-ms" => settings.crossfade_ms = parse_i64(args.next(), "--crossfade-ms")?,
            "--help" | "-h" => {
                return Ok(Output::new(
                    json!({"usage": cut_usage_text()}),
                    cut_usage_text().to_owned(),
                    false,
                    0,
                ));
            }
            other => return Err(CliError::Usage(format!("unknown cut flag: {other}"))),
        }
    }
    if !remove {
        return Err(cut_usage());
    }
    settings.validate()?;

    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 100)?;
    session.restore_metadata(lease.metadata())?;
    let request = PauseCandidatesRequest::new(session.project().clone(), settings)?;
    let mut runner = cli_runner(context.budget()?)?;
    let job = request.submit(&mut runner)?;
    wait_for_job(&mut runner, job.id)?;
    let candidates = job
        .finish(&mut runner)?
        .ok_or(CliError::Rejected("pause candidates produced no result"))?;
    runner.shutdown();

    let saved: i64 = candidates
        .spans
        .iter()
        .map(|span| span.end.0.saturating_sub(span.start.0))
        .sum();
    let mut human = String::new();
    for span in &candidates.spans {
        let _ = writeln!(
            human,
            "{:.3} {:.3}",
            span.start.seconds(),
            span.end.seconds()
        );
    }
    let _ = write!(
        human,
        "{} pauses, {:.3} s",
        candidates.spans.len(),
        saved as f64 / time::TICKS_PER_SECOND as f64
    );

    let changed = if dry_run || candidates.spans.is_empty() {
        false
    } else {
        let before = session.project().clone();
        let outcome = session.execute(
            "timeline.remove_pauses",
            json!({
                "spans": candidates.spans.clone(),
                "crossfade_ms": settings.crossfade_ms,
            }),
        )?;
        if outcome.changed && before != *session.project() {
            let _receipt = lease.save(session.project().as_ref())?;
            session.saved()?;
        }
        if outcome.changed {
            let mut metadata = session.metadata().clone();
            metadata.touch();
            lease.save_metadata(&metadata)?;
        }
        outcome.changed
    };
    let value = json!({
        "project": json_path(&project_path)?,
        "spans": candidates.spans,
        "analyzed_sources": candidates.analyzed_sources,
        "changed": changed,
        "dry_run": dry_run,
    });
    let human = if dry_run {
        format!("dry run: {human}")
    } else if changed {
        format!("removed {human}")
    } else {
        format!("no change: {human}")
    };
    Ok(Output::new(value, human, false, 0))
}

fn cut_usage() -> CliError {
    CliError::Usage(cut_usage_text().into())
}

fn cut_usage_text() -> &'static str {
    "usage: vibecut-cli cut <project> --remove-pauses [--min-pause-ms 500] \
     [--pad-ms 20] [--crossfade-ms 20] [--dry-run]"
}
