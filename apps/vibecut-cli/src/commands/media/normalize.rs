//! Measures project loudness through engine jobs.
use crate::args::CliError;
use crate::args::utf8;
use crate::output::Output;
use crate::output::wait_for_job;
use crate::session::Context;
use engine::Session;
use engine::cache::ArtifactCache;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use project::WriterLease;
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Set loudness targets, run the two-pass measurement job over the production
/// mix graph, and store the gains: the next `export` applies them.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    const USAGE: &str = "usage: vibecut-cli normalize <project> [--clip] [--master]          [--target-lufs -14] [--true-peak-db -1]";
    let mut args = args.into_iter();
    let project_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Usage(USAGE.into()))?;
    let (mut clip, mut master) = (false, false);
    let (mut target_lufs, mut true_peak_db) = (-14.0f32, -1.0f32);
    while let Some(arg) = args.next() {
        let mut number = |name: &str| -> Result<f32, CliError> {
            let value = args
                .next()
                .ok_or_else(|| CliError::Usage(format!("{name} requires a number")))?;
            utf8(&value, name)?
                .parse()
                .map_err(|_| CliError::Usage(format!("{name} requires a number")))
        };
        match utf8(&arg, "normalize flag")? {
            "--clip" => clip = true,
            "--master" => master = true,
            "--target-lufs" => target_lufs = number("--target-lufs")?,
            "--true-peak-db" => true_peak_db = number("--true-peak-db")?,
            other => return Err(CliError::Usage(format!("unknown normalize flag: {other}"))),
        }
    }
    let scopes = match (clip, master) {
        (false, false) | (false, true) => vec!["master"],
        (true, false) => vec!["clip"],
        (true, true) => vec!["clip", "master"],
    };
    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    session.restore_metadata(lease.metadata())?;
    for scope in scopes {
        session.execute(
            "audio.set_loudness",
            json!({"scope": scope, "target_lufs": target_lufs, "true_peak_db": true_peak_db}),
        )?;
    }
    let budget = context.budget()?;
    let (mut cache, _diagnostics) = ArtifactCache::open(
        &engine::providers::default_cache_root()?,
        budget.clone(),
        128,
    )?;
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
    let job = engine::loudness::submit_loudness(
        &mut runner,
        &mut cache,
        session.project().clone(),
        || Box::new(render::FileSources::default()),
    )?;
    wait_for_job(&mut runner, job.id)?;
    let result = job
        .finish(&mut runner, &mut cache)?
        .ok_or(CliError::Rejected("loudness job produced no result"))?;
    runner.shutdown();
    session.execute("audio.apply_loudness", json!({"gains": &result.gains}))?;
    lease.save(session.project().as_ref())?;
    session.saved()?;
    let mut metadata = session.metadata().clone();
    metadata.touch();
    lease.save_metadata(&metadata)?;
    let measured = |m: &engine::loudness::Measured, gain_db: f32, item: Option<u64>| {
        json!({
            "item": item,
            "integrated_lufs": m.integrated_lufs,
            "true_peak_db": m.true_peak_db,
            "gain_db": gain_db,
        })
    };
    let value = json!({
        "clips": result
            .clips
            .iter()
            .map(|(item, m)| measured(m, result.gains.clip_db(*item), Some(item.0)))
            .collect::<Vec<_>>(),
        "master": result
            .master
            .as_ref()
            .map(|m| measured(m, result.gains.master_db, None)),
    });
    let mut human = String::new();
    for (item, m) in &result.clips {
        let _ = writeln!(
            human,
            "clip {}: {} -> gain {:+.2} dB",
            item.0,
            lufs_human(m),
            result.gains.clip_db(*item)
        );
    }
    if let Some(m) = &result.master {
        let _ = writeln!(
            human,
            "master: {} -> gain {:+.2} dB",
            lufs_human(m),
            result.gains.master_db
        );
    }
    Ok(Output::new(value, human.trim_end().to_owned(), false, 0))
}

fn lufs_human(m: &engine::loudness::Measured) -> String {
    match (m.integrated_lufs, m.true_peak_db) {
        (Some(l), Some(p)) => format!("{l:.1} LUFS, {p:.1} dBTP"),
        _ => "silent".to_owned(),
    }
}
