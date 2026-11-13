//! Applies an engine registry command to a leased project.
use crate::args::CliError;
use crate::args::utf8;
use crate::output::Output;
use crate::session::{
    Context, Edit, History, Lent, WritePolicy, cli_runner, run_command, run_leased, with_stickers,
};
use engine::control::ControlResult;
use engine::{CreatedId, control::Error as EngineError};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(exec_usage)?;
    let command_id = args.next().ok_or_else(exec_usage)?;
    let params = args.next().ok_or_else(exec_usage)?;
    if args.next().is_some() {
        return Err(exec_usage());
    }
    let command_id = utf8(&command_id, "command id")?;
    let params = utf8(&params, "command JSON")?;
    let params: Value = serde_json::from_str(params).map_err(CliError::Json)?;

    if engine::lifecycle(command_id) == Some(engine::Lifecycle::Save) {
        run_leased(
            &project_path,
            History::Single,
            WritePolicy::Always,
            |_session| {
                if params != json!({}) {
                    return Err(
                        EngineError::Control(engine::control::ControlError::InvalidParams).into(),
                    );
                }
                Ok(Edit {
                    value: (),
                    changed: false,
                })
            },
        )?;
        return Ok(Output::new(
            json!({"changed":false,"created":[]}),
            "saved".into(),
            false,
            0,
        ));
    }
    // The sticker commands need the bundled catalog, which only the composition root lends.
    let outcome = with_stickers(|stickers| {
        run_leased(
            &project_path,
            History::Single,
            WritePolicy::Changed,
            |session| {
                let (outcome, changed) = if STICKER_COMMANDS.contains(&command_id) {
                    let mut runner = cli_runner(context.budget()?)?;
                    let result = run_command(
                        session,
                        &mut runner,
                        Lent {
                            stickers: Some(stickers),
                            ..Lent::default()
                        },
                        command_id,
                        params,
                        command_id == "stickers.insert",
                    );
                    runner.shutdown();
                    match result? {
                        ControlResult::Edit { outcome } => {
                            let changed = outcome.changed;
                            (Exec::Edit(outcome), changed)
                        }
                        other => (Exec::Result(other), false),
                    }
                } else {
                    let outcome = engine::control::run_wire(session, command_id, params)?;
                    let changed = outcome.changed;
                    (Exec::Edit(outcome), changed)
                };
                Ok(Edit {
                    value: outcome,
                    changed,
                })
            },
        )
    })?
    .value;
    match outcome {
        Exec::Edit(outcome) => {
            let value = serde_json::to_value(&outcome)?;
            Ok(Output::new(value, outcome_human(&outcome), false, 0))
        }
        Exec::Result(result) => {
            let value = serde_json::to_value(&result)?;
            Ok(Output::new(value, "ok".into(), false, 0))
        }
    }
}

/// The registry commands that read or write the bundled sticker catalog.
const STICKER_COMMANDS: [&str; 2] = ["stickers.search", "stickers.insert"];

/// What one `exec` produced: an edit's outcome, or a read result that changes nothing.
enum Exec {
    Edit(engine::Outcome),
    Result(ControlResult),
}

fn exec_usage() -> CliError {
    CliError::Usage("usage: vibecut-cli exec <project> <command_id> <json>".into())
}

fn outcome_human(outcome: &engine::Outcome) -> String {
    let mut human = if outcome.changed {
        "changed\n".to_owned()
    } else {
        "no change\n".to_owned()
    };
    for created in &outcome.created {
        let _ = writeln!(human, "created {}", created_human(created));
    }
    human.trim_end().to_owned()
}

fn created_human(created: &CreatedId) -> String {
    match created {
        CreatedId::CaptionPreset(id) => format!("caption preset {}", id.0),
        CreatedId::TextPreset(id) => format!("text preset {}", id.0),
        CreatedId::Transition(id) => format!("transition {}", id.0),
        CreatedId::Scene(id) => format!("scene {}", id.0),
        CreatedId::Track(id) => format!("track {}", id.0),
        CreatedId::Item(id) => format!("item {}", id.0),
        CreatedId::Bookmark(id) => format!("bookmark {}", id.0),
    }
}
