//! Applies imported display-referred looks through the engine registry.
use crate::{
    args::{CliError, utf8},
    output::Output,
    session::{Context, Edit, History, WritePolicy, cli_runner, run_job, run_leased},
};
use std::{ffi::OsString, path::PathBuf, sync::Mutex};
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let usage = || {
        CliError::Usage("usage: vibecut-cli color lut <project> --import <file> --item <id> [--strength <0..100>]".into())
    };
    let mut args = args.into_iter();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("lut")) {
        return Err(usage());
    }
    let path = PathBuf::from(args.next().ok_or_else(usage)?);
    let mut source = None;
    let mut item = None;
    let mut strength = 100.;
    while let Some(arg) = args.next() {
        match utf8(&arg, "color option")? {
            "--import" if source.is_none() => {
                source = Some(PathBuf::from(args.next().ok_or_else(usage)?))
            }
            "--item" if item.is_none() => {
                item = Some(project::ItemId(
                    utf8(&args.next().ok_or_else(usage)?, "item")?
                        .parse()
                        .map_err(|_| usage())?,
                ))
            }
            "--strength" => {
                strength = utf8(&args.next().ok_or_else(usage)?, "strength")?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                    .ok_or_else(usage)?
            }
            _ => return Err(usage()),
        }
    }
    let source = source.ok_or_else(usage)?;
    let item = item.ok_or_else(usage)?;
    let budget = context.budget()?;
    let media = Mutex::new(engine::control::MediaAccess::open(budget.clone())?);
    let mut runner = cli_runner(budget)?;
    let outcome = run_leased(&path, History::Single, WritePolicy::Changed, |session| {
        let job = run_job(
            session,
            &mut runner,
            Some(&media),
            None,
            "color.import_lut",
            serde_json::json!({"path":source,"name":null,"licence":null,"expected_hash":null}),
        )?;
        let Some(engine::control::JobOutput::Lut(record)) = job.output else {
            return Err(CliError::Rejected("LUT job returned no resource"));
        };
        let outcome = session.apply_lut(item, record, strength)?;
        Ok(Edit {
            changed: outcome.changed,
            value: outcome,
        })
    })?
    .value;
    Ok(Output::new(
        serde_json::to_value(outcome)?,
        "LUT applied".into(),
        false,
        0,
    ))
}
