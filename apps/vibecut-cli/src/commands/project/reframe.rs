//! Generates and applies the auto-reframe crop of one video item.
use crate::args::CliError;
use crate::args::{parse_u64, utf8};
use crate::output::Output;
use crate::session::{Context, Lent, cli_runner, run_command, run_job};
use engine::Session;
use engine::control::{ControlResult, JobOutput, MediaAccess};
use project::WriterLease;
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(reframe_usage)?;
    let (mut item, mut target, mut replace_manual, mut dry_run) =
        (None::<project::ItemId>, None, false, false);
    while let Some(arg) = args.next() {
        match utf8(&arg, "reframe flag")? {
            "--item" => item = Some(project::ItemId(parse_u64(args.next(), "--item")?)),
            "--target" => {
                target = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--target requires an aspect".into()))?,
                        "target",
                    )?
                    .to_owned(),
                )
            }
            "--replace-manual" => replace_manual = true,
            "--dry-run" => dry_run = true,
            "--help" | "-h" => {
                return Ok(Output::new(
                    json!({"usage": reframe_usage_text()}),
                    reframe_usage_text().to_owned(),
                    false,
                    0,
                ));
            }
            other => return Err(CliError::Usage(format!("unknown reframe flag: {other}"))),
        }
    }
    let item = item.ok_or_else(reframe_usage)?;
    let target = target.ok_or_else(reframe_usage)?;
    if !matches!(target.as_str(), "9:16" | "1:1" | "16:9") {
        return Err(CliError::Usage("--target must be 9:16, 1:1 or 16:9".into()));
    }
    let budget = context.budget()?;
    let media = Mutex::new(MediaAccess::open(budget.clone())?);
    let vision = shorts::installed_faces(budget.clone(), None);
    let mut runner = cli_runner(budget)?;
    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    session.restore_metadata(lease.metadata())?;
    let finished = run_job(
        &mut session,
        &mut runner,
        Some(&media),
        Some(&vision),
        "reframe.generate",
        json!({"item": item, "target": target}),
    )?;
    let Some(JobOutput::Reframe(report)) = finished.output else {
        runner.shutdown();
        return Err(CliError::Rejected("reframe job produced no report"));
    };
    let job = finished.id;
    let changed = if dry_run {
        false
    } else {
        let lent = Lent {
            media: Some(&media),
            vision: Some(&vision),
            ..Lent::default()
        };
        let applied = run_command(
            &mut session,
            &mut runner,
            lent,
            "reframe.apply",
            json!({"job": job, "replace_manual": replace_manual}),
            true,
        )?;
        let ControlResult::Edit { outcome } = applied else {
            runner.shutdown();
            return Err(CliError::Rejected("reframe.apply returned no edit"));
        };
        if outcome.changed {
            lease.save(session.project().as_ref())?;
            session.saved()?;
            let mut metadata = session.metadata().clone();
            metadata.touch();
            lease.save_metadata(&metadata)?;
        }
        outcome.changed
    };
    runner.shutdown();
    let value = json!({
        "status": "completed",
        "job": job,
        "target": report.target,
        "size": report.size,
        "shots": report.shots.iter().map(|shot| json!({
            "start": shot.start,
            "keys": shot.keys,
            "reasons": &shot.reasons,
        })).collect::<Vec<_>>(),
        "applied": !dry_run,
        "changed": changed,
    });
    Ok(Output::new(
        value,
        human(&report, !dry_run, changed),
        false,
        0,
    ))
}

fn human(report: &engine::control::ReframeReport, applied: bool, changed: bool) -> String {
    let mut human = format!(
        "target {}: crop {:.3} x {:.3} of the source\n",
        match report.target {
            engine::control::ReframeTarget::Vertical => "9:16",
            engine::control::ReframeTarget::Square => "1:1",
            engine::control::ReframeTarget::Landscape => "16:9",
        },
        report.size[0],
        report.size[1]
    );
    for shot in &report.shots {
        let _ = write!(human, "shot {}: {} keys", shot.start.0, shot.keys);
        for reason in &shot.reasons {
            let _ = write!(
                human,
                " ({})",
                match reason {
                    engine::control::ReframeReason::Ambiguous => "ambiguous",
                    engine::control::ReframeReason::NoSubject => "no_subject",
                    engine::control::ReframeReason::Lost => "lost",
                }
            );
        }
        human.push('\n');
    }
    if applied {
        let _ = writeln!(human, "{}", if changed { "applied" } else { "no change" });
    } else {
        human.push_str("dry run: project unchanged\n");
    }
    human.trim_end().to_owned()
}

fn reframe_usage_text() -> &'static str {
    "usage: vibecut-cli reframe <project> --item <id> --target 9:16|1:1|16:9\n\
                    [--replace-manual] [--dry-run]"
}

fn reframe_usage() -> CliError {
    CliError::Usage(reframe_usage_text().into())
}
