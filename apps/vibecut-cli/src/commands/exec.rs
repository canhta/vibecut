use crate::arguments::utf8;
use crate::exit_codes::CliError;
use crate::output::Output;
use engine::{CreatedId, EditError, Session};
use project::WriterLease;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

pub(crate) fn exec(args: Vec<OsString>) -> Result<Output, CliError> {
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

    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    session.restore_metadata(lease.metadata())?;
    if command_id == "project.save" {
        if params != json!({}) {
            return Err(EditError::InvalidParams.into());
        }
        let _ = lease.save(session.project())?;
        lease.save_metadata(session.metadata())?;
        return Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"changed":false,"created":[]}),
            human: "saved".into(),
        });
    }
    let before = session.project().clone();
    let outcome = session.execute(command_id, params)?;
    if outcome.changed && before != *session.project() {
        let _receipt = lease.save(session.project().as_ref())?;
        session.saved()?;
    }
    if outcome.changed {
        let mut metadata = session.metadata().clone();
        metadata.touch();
        lease.save_metadata(&metadata)?;
    }
    let value = serde_json::to_value(&outcome)?;
    Ok(Output {
        silent: false,
        exit: 0,
        value,
        human: outcome_human(&outcome),
    })
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
        CreatedId::Transition(id) => format!("transition {}", id.0),
        CreatedId::Scene(id) => format!("scene {}", id.0),
        CreatedId::Track(id) => format!("track {}", id.0),
        CreatedId::Item(id) => format!("item {}", id.0),
        CreatedId::Bookmark(id) => format!("bookmark {}", id.0),
    }
}
