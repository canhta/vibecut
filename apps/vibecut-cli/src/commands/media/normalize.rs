//! Measures project loudness through engine jobs.
use crate::args::CliError;
use crate::args::utf8;
use crate::output::Output;
use crate::session::{Context, cli_runner, run_job};
use engine::control::JobOutput;
use engine::{Command, Session};
use project::{LoudnessScope, WriterLease};
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

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
        (false, false) | (false, true) => vec![LoudnessScope::Master],
        (true, false) => vec![LoudnessScope::Clip],
        (true, true) => vec![LoudnessScope::Clip, LoudnessScope::Master],
    };
    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    session.restore_metadata(lease.metadata())?;
    for scope in scopes {
        session.apply(Command::SetLoudness {
            scope,
            target_lufs,
            true_peak_db,
        })?;
    }
    let mut runner = cli_runner(context.budget()?)?;
    let finished = run_job(
        &mut session,
        &mut runner,
        None,
        "audio.measure_loudness",
        json!({}),
    );
    runner.shutdown();
    let Some(JobOutput::Loudness(result)) = finished?.output else {
        return Err(CliError::Rejected("loudness job produced no result"));
    };
    session.apply(Command::ApplyLoudness {
        gains: result.gains.clone(),
    })?;
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
