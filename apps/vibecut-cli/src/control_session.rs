use crate::arguments::utf8;
use crate::exit_codes::CliError;
use crate::job_runtime::CLI_MEMORY_LIMIT;
use crate::output::Output;
use engine::control::{Caller, ControlResult, ErrorCode, Policy, Request, Response, ResponseValue};
use engine::fonts::FontStore;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use engine::{ByteBudget, Command, EditError, Session, control};
use project::{Project, WriterLease};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Trusted admission owned by this composition root; never deserialized from
/// client JSON. The CLI grants overwrite only when the flag was passed.
struct ControlPolicy {
    allow_overwrite: bool,
}
impl Policy for ControlPolicy {
    fn snapshot_overwrite(&self) -> bool {
        self.allow_overwrite
    }

    fn authorize(&self, command: &Command, _: &Project) -> engine::Result<()> {
        let overwrite = match command {
            Command::Export { overwrite, .. }
            | Command::ExportSubtitles { overwrite, .. }
            | Command::BurnSubtitles { overwrite, .. } => Some(*overwrite),
            _ => None,
        };
        if overwrite == Some(true) && !self.allow_overwrite {
            return Err(EditError::Permission);
        }
        Ok(())
    }
}

fn control_usage() -> CliError {
    CliError::Usage("usage: vibecut-cli control <project> [--allow-overwrite] [--no-save]".into())
}

pub(crate) fn control(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(project) = args.next() else {
        return Err(control_usage());
    };
    if project
        .to_str()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        let usage = "usage: vibecut-cli control <project> [--allow-overwrite] [--no-save]\n\
                     Reads newline-delimited control requests on stdin and writes one JSON\n\
                     response per line. The session saves changed project/metadata on EOF\n\
                     unless --no-save is set; --allow-overwrite grants FilePublish overwrite.";
        return Ok(Output {
            value: json!({"usage": usage}),
            human: usage.to_owned(),
            silent: false,
            exit: 0,
        });
    }
    let project_path = PathBuf::from(project);
    let mut allow_overwrite = false;
    let mut save = true;
    for arg in args {
        match utf8(&arg, "control flag")? {
            "--allow-overwrite" => allow_overwrite = true,
            "--no-save" => save = false,
            other => return Err(CliError::Usage(format!("unknown control flag: {other}"))),
        }
    }

    // One process is one engine Session. The lease is held from load to the
    // final save, so no other writer can interleave during the stream.
    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 100)?;
    session.restore_metadata(lease.metadata())?;
    session.protect_paths(vec![
        project_path.clone(),
        project::metadata_path(&project_path)?,
    ])?;
    // Without a user cache location the font commands refuse; the session still serves edits.
    if let Ok(store) = FontStore::native() {
        session.set_fonts(Arc::new(store));
    }
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 32,
            retained: 32,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(50),
        },
        ByteBudget::new(CLI_MEMORY_LIMIT)?,
        Arc::new(SystemClock::default()),
    )?;
    let policy = ControlPolicy { allow_overwrite };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut project_dirty = false;
    let mut metadata_dirty = false;
    let mut last_exit = 0u8;

    loop {
        runner.tick()?;
        session.observe_jobs(&runner);
        let frame = match read_control_frame(&mut input, control::MAX_FRAME_BYTES) {
            Ok(frame) => frame,
            Err(CliError::Engine(EditError::Limit)) => {
                write_control_error(
                    &mut out,
                    None,
                    ErrorCode::Limit,
                    "control request exceeds the frame limit",
                )?;
                last_exit = control_exit(ErrorCode::Limit);
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(bytes) = frame else {
            break;
        };
        if bytes.is_empty() {
            continue;
        }
        let value = match control::parse_frame(&bytes) {
            Ok(value) => value,
            Err(error) => {
                let code = ErrorCode::from_error(&error);
                write_control_error(&mut out, None, code, frame_error_message(&error))?;
                last_exit = control_exit(code);
                continue;
            }
        };
        let request = match build_control_request(&session, value) {
            Ok(request) => request,
            Err(error) => {
                let code = ErrorCode::from_error(&error);
                write_control_error(
                    &mut out,
                    None,
                    code,
                    "control request has an invalid envelope",
                )?;
                last_exit = control_exit(code);
                continue;
            }
        };
        let request_id = request.request_id.clone();
        let command_id = request.command.clone();
        let response = if command_id == "project.save" {
            let caller = Caller {
                principal: "vibecut-cli",
                external: true,
                policy: &policy,
            };
            // The engine owns save admission and its receipt; this closure only
            // publishes the bytes through the project owner.
            match session.control_lifecycle_save(request, caller, |project, metadata| {
                let _receipt = lease.save(project)?;
                lease.save_metadata(metadata)?;
                Ok(())
            }) {
                Ok(response) => response,
                Err(error) => {
                    let code = ErrorCode::from_error(&error);
                    write_control_error(
                        &mut out,
                        Some(&request_id),
                        code,
                        "control request refused before admission",
                    )?;
                    last_exit = control_exit(code);
                    continue;
                }
            }
        } else {
            let before = session.project().clone();
            let caller = Caller {
                principal: "vibecut-cli",
                external: true,
                policy: &policy,
            };
            match session.control_request(request, caller, &mut runner) {
                Ok(response) => {
                    if matches!(
                        &response.value,
                        ResponseValue::Result(ControlResult::Edit { outcome }) if outcome.changed
                    ) {
                        metadata_dirty = true;
                        if command_id != "project.set_view_state" {
                            project_dirty = true;
                        }
                    }
                    if before != *session.project() {
                        project_dirty = true;
                    }
                    response
                }
                Err(error) => {
                    let code = ErrorCode::from_error(&error);
                    write_control_error(
                        &mut out,
                        Some(&request_id),
                        code,
                        "control request refused before admission",
                    )?;
                    last_exit = control_exit(code);
                    continue;
                }
            }
        };
        // A save clears dirty state only when it published the current revision.
        // Replay of an older receipt after later edits must leave the session dirty.
        if command_id == "project.save"
            && matches!(&response.value, ResponseValue::Result(_))
            && response.rev_after == session.revision()
        {
            project_dirty = false;
            metadata_dirty = false;
        }
        last_exit = control_response_exit(&response.value);
        write_control_response(&mut out, &response)?;
    }

    runner.shutdown();
    if save && (project_dirty || metadata_dirty) {
        save_control_state(&mut session, &mut lease, project_dirty, metadata_dirty)?;
    }
    Ok(Output {
        value: Value::Null,
        human: String::new(),
        silent: true,
        exit: last_exit,
    })
}

pub(crate) fn build_control_request(
    session: &Session,
    mut value: Value,
) -> Result<Request, EditError> {
    let Value::Object(fields) = &mut value else {
        return Err(EditError::InvalidParams);
    };
    fields
        .entry("api_version")
        .or_insert_with(|| json!(control::API_VERSION));
    fields
        .entry("registry_hash")
        .or_insert_with(|| json!(control::registry_hash()));
    fields
        .entry("session_id")
        .or_insert_with(|| json!(session.session_id()));
    let request: Request = serde_json::from_value(value).map_err(|_| EditError::InvalidParams)?;
    Ok(request)
}

pub(crate) fn save_control_state(
    session: &mut Session,
    lease: &mut WriterLease,
    project_dirty: bool,
    metadata_dirty: bool,
) -> Result<(), CliError> {
    if project_dirty {
        let _receipt = lease.save(session.project())?;
    }
    if metadata_dirty {
        let mut metadata = session.metadata().clone();
        metadata.touch();
        lease.save_metadata(&metadata)?;
        session.restore_metadata(metadata)?;
    }
    if project_dirty || metadata_dirty {
        session.saved()?;
    }
    Ok(())
}

fn control_response_exit(value: &ResponseValue) -> u8 {
    match value {
        ResponseValue::Result(_) => 0,
        ResponseValue::Error(code) => control_exit(*code),
    }
}

fn control_exit(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::UnknownCommand
        | ErrorCode::InvalidParams
        | ErrorCode::Limit
        | ErrorCode::SchemaMismatch => 2,
        ErrorCode::StaleRevision => 3,
        ErrorCode::Permission
        | ErrorCode::IdempotencyConflict
        | ErrorCode::ReceiptCapacity
        | ErrorCode::TransactionActive
        | ErrorCode::EmptyProject
        | ErrorCode::Refused => 5,
        ErrorCode::SessionGone
        | ErrorCode::ResyncRequired
        | ErrorCode::JobNotRetained
        | ErrorCode::Cancelled
        | ErrorCode::QueueFull
        | ErrorCode::Failed => 7,
    }
}

fn error_code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnknownCommand => "unknown_command",
        ErrorCode::InvalidParams => "invalid_params",
        ErrorCode::Limit => "limit",
        ErrorCode::SchemaMismatch => "schema_mismatch",
        ErrorCode::SessionGone => "session_gone",
        ErrorCode::StaleRevision => "stale_revision",
        ErrorCode::Permission => "permission",
        ErrorCode::IdempotencyConflict => "idempotency_conflict",
        ErrorCode::ReceiptCapacity => "receipt_capacity",
        ErrorCode::ResyncRequired => "resync_required",
        ErrorCode::JobNotRetained => "job_not_retained",
        ErrorCode::Cancelled => "cancelled",
        ErrorCode::QueueFull => "queue_full",
        ErrorCode::TransactionActive => "transaction_active",
        ErrorCode::EmptyProject => "empty_project",
        ErrorCode::Refused => "refused",
        ErrorCode::Failed => "failed",
    }
}

fn frame_error_message(error: &EditError) -> &'static str {
    match error {
        EditError::Limit => "control request exceeds the frame limit",
        _ => "control request is not valid control JSON",
    }
}

fn write_control_response(out: &mut impl io::Write, response: &Response) -> Result<(), CliError> {
    let mut value = serde_json::to_value(response)?;
    if let Value::Object(fields) = &mut value {
        fields.insert(
            "exit_code".into(),
            json!(control_response_exit(&response.value)),
        );
    }
    writeln!(out, "{value}")?;
    out.flush()?;
    Ok(())
}

fn write_control_error(
    out: &mut impl io::Write,
    request_id: Option<&str>,
    code: ErrorCode,
    message: &str,
) -> Result<(), CliError> {
    let exit = control_exit(code);
    let mut value = json!({
        "status": "error",
        "exit_code": exit,
        "error": {"code": error_code_name(code), "message": message, "exit_code": exit},
    });
    if let (Some(request_id), Value::Object(fields)) = (request_id, &mut value) {
        fields.insert("request_id".into(), json!(request_id));
    }
    writeln!(out, "{value}")?;
    out.flush()?;
    Ok(())
}

/// Read one bounded newline-framed request, draining an oversized line so the
/// next frame still starts at a line boundary.
fn read_control_frame(input: &mut impl BufRead, max: usize) -> Result<Option<Vec<u8>>, CliError> {
    let mut line = Vec::new();
    loop {
        let (take, done) = {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                break;
            }
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (buffer.len(), false),
            }
        };
        if line.len().saturating_add(take) > max.saturating_add(2) {
            input.consume(take);
            if !done {
                drain_control_line(input)?;
            }
            return Err(EditError::Limit.into());
        }
        let buffer = input.fill_buf()?;
        line.extend(buffer.iter().take(take).copied());
        input.consume(take);
        if done {
            break;
        }
    }
    while line
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        line.pop();
    }
    Ok(Some(line))
}

fn drain_control_line(input: &mut impl BufRead) -> Result<(), CliError> {
    loop {
        let (take, done) = {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                return Ok(());
            }
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (buffer.len(), false),
            }
        };
        input.consume(take);
        if done {
            return Ok(());
        }
    }
}
