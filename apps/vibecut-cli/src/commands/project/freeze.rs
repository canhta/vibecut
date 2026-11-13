//! Freezes one video item's frame at the playhead as a held still.
use crate::args::{CliError, parse_f64, parse_u64, seconds_ticks, utf8};
use crate::output::{Output, json_path};
use crate::session::{Context, Edit, History, Lent, WritePolicy, cli_runner, run_command, run_leased};
use engine::CreatedId;
use engine::control::ControlResult;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;

/// Default hold in seconds; the editor's Freeze button uses the same value.
const DEFAULT_HOLD_SECONDS: f64 = 3.0;

/// `freeze <project> --item N --at SECONDS --dir DIR [--duration SECONDS]`
/// captures the project frame at the playhead into `DIR` and inserts it as a
/// held still that splits the item, as one undoable edit. `DIR` must be
/// absolute; the engine refuses a relative directory.
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(freeze_usage)?;
    let (mut item, mut at, mut duration, mut dir) = (None::<u64>, None::<f64>, None::<f64>, None);
    while let Some(flag) = args.next() {
        match utf8(&flag, "freeze flag")? {
            "--item" => item = Some(parse_u64(args.next(), "--item")?),
            "--at" => at = Some(parse_f64(args.next(), "--at")?),
            "--duration" => duration = Some(parse_f64(args.next(), "--duration")?),
            "--dir" => {
                dir = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| CliError::Usage("--dir requires a folder".into()))?,
                ));
            }
            "--help" | "-h" => {
                return Ok(Output::new(
                    json!({"usage": freeze_usage_text()}),
                    freeze_usage_text().to_owned(),
                    false,
                    0,
                ));
            }
            other => return Err(CliError::Usage(format!("unknown freeze flag: {other}"))),
        }
    }
    let item = item.ok_or_else(freeze_usage)?;
    let at = seconds_ticks(at.ok_or_else(freeze_usage)?)?;
    let hold = seconds_ticks(duration.unwrap_or(DEFAULT_HOLD_SECONDS))?;
    let dir = dir.ok_or_else(freeze_usage)?;
    let mut runner = cli_runner(context.budget()?)?;
    let outcome = run_leased(
        &project_path,
        History::Single,
        WritePolicy::Changed,
        |session| {
            let result = run_command(
                session,
                &mut runner,
                Lent::default(),
                "timeline.freeze_frame",
                json!({"item": item, "at": at, "duration": hold, "dir": dir}),
                true,
            )?;
            let ControlResult::Edit { outcome } = result else {
                return Err(CliError::Rejected("freeze returned no edit"));
            };
            let changed = outcome.changed;
            Ok(Edit {
                value: outcome,
                changed,
            })
        },
    );
    runner.shutdown();
    let outcome = outcome?.value;
    let still = outcome.created.iter().find_map(|created| match created {
        CreatedId::Item(id) => Some(id.0),
        _ => None,
    });
    let value = json!({
        "project": json_path(&project_path)?,
        "still_item": still,
        "changed": outcome.changed,
        "dir": json_path(&dir)?,
    });
    let human = match still {
        Some(id) => format!("froze item {item} at {:.3} s as item {id}", at.seconds()),
        None => format!("froze item {item}"),
    };
    Ok(Output::new(value, human, false, 0))
}

fn freeze_usage() -> CliError {
    CliError::Usage(freeze_usage_text().into())
}

fn freeze_usage_text() -> &'static str {
    "usage: vibecut-cli freeze <project> --item N --at SECONDS --dir ABSOLUTE_DIR \
     [--duration SECONDS]  (default duration 3 s)"
}
