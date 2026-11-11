//! Owns the leased CLI control session through its final save.
use super::Context;
use super::libraries::with_fonts;
use crate::args::CliError;
use crate::output::Output;
use engine::Failure;
use engine::control::FontAccess;
use engine::control::{
    Caller, ControlResult, ErrorCode, MediaAccess, Policy, Response, ResponseValue,
};
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use engine::{Command, EditError, Session, control};
use project::{Project, WriterLease};
use serde_json::{Value, json};
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
        if !self.allow_overwrite
            && command
                .published_path()
                .is_some_and(|(_, overwrite)| overwrite)
        {
            return Err(EditError::Control(
                engine::control::ControlError::Permission,
            ));
        }
        Ok(())
    }
}

pub(crate) fn control(
    context: &Context,
    project_path: PathBuf,
    allow_overwrite: bool,
    save: bool,
) -> Result<Output, CliError> {
    with_fonts(|fonts| control_with(context, project_path, allow_overwrite, save, fonts))
}

fn control_with(
    context: &Context,
    project_path: PathBuf,
    allow_overwrite: bool,
    save: bool,
    fonts: Option<&FontAccess<'_>>,
) -> Result<Output, CliError> {
    // One process is one engine Session. The lease is held from load to the
    // final save, so no other writer can interleave during the stream.
    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    super::prepare_project_fonts(&loaded.project)?;
    let mut session = Session::new(loaded.project, 100)?;
    session.restore_metadata(lease.metadata())?;
    session.protect_paths(vec![
        project_path.clone(),
        project::metadata_path(&project_path)?,
    ])?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 32,
            retained: 32,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(50),
        },
        context.budget()?,
        Arc::new(SystemClock::default()),
    )?;
    // Without a usable media cache the generation commands refuse; edits still run.
    let media = MediaAccess::open(context.budget()?)
        .ok()
        .map(std::sync::Mutex::new);
    let captions = super::caption_access(context.budget()?);
    let policy = ControlPolicy { allow_overwrite };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut project_dirty = false;
    let mut metadata_dirty = false;
    let mut last_exit = 0u8;

    loop {
        super::prepare_project_fonts(session.project())?;
        runner.tick()?;
        session.observe_jobs(&runner);
        let frame = match read_control_frame(&mut input, control::MAX_FRAME_BYTES) {
            Ok(frame) => frame,
            Err(CliError::Engine(EditError::Control(engine::control::ControlError::Limit))) => {
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
                let code = error.code();
                write_control_error(&mut out, None, code, frame_error_message(&error.into()))?;
                last_exit = control_exit(code);
                continue;
            }
        };
        let request = match control::build_request(&session, value) {
            Ok(request) => request,
            Err(error) => {
                let code = error.code();
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
        let before = session.project().clone();
        let caller = Caller {
            principal: "vibecut-cli",
            external: true,
            policy: &policy,
            fonts,
            sounds: None,
            media: media.as_ref(),
            chapters: None,
            captions: Some(&captions),
        };
        // The engine owns save admission and its receipt; the closure only
        // publishes the bytes through the project owner.
        let response =
            match session.control_routed(request, caller, &mut runner, |project, metadata| {
                let _receipt = lease.save(project)?;
                lease.save_metadata(metadata)?;
                Ok(())
            }) {
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
                    let code = error.code();
                    write_control_error(
                        &mut out,
                        Some(&request_id),
                        code,
                        "control request refused before admission",
                    )?;
                    last_exit = control_exit(code);
                    continue;
                }
            };
        // A save clears dirty state only when it published the current revision.
        // Replay of an older receipt after later edits must leave the session dirty.
        if engine::lifecycle(&command_id) == Some(engine::Lifecycle::Save)
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
    Ok(Output::new(Value::Null, String::new(), true, last_exit))
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
        | ErrorCode::SchemaMismatch
        | ErrorCode::Budget => 2,
        ErrorCode::StaleRevision | ErrorCode::StaleResult => 3,
        ErrorCode::Permission
        | ErrorCode::IdempotencyConflict
        | ErrorCode::ReceiptCapacity
        | ErrorCode::TransactionActive
        | ErrorCode::EmptyProject
        | ErrorCode::Refused
        | ErrorCode::Unavailable
        | ErrorCode::Setup
        | ErrorCode::Auth
        | ErrorCode::Busy
        | ErrorCode::Conflict
        | ErrorCode::Media
        | ErrorCode::Render
        | ErrorCode::Ai
        | ErrorCode::Job
        | ErrorCode::Quota
        | ErrorCode::RateLimit
        | ErrorCode::Network
        | ErrorCode::AudioDevice
        | ErrorCode::Sounds
        | ErrorCode::SoundLimit
        | ErrorCode::SoundCancelled
        | ErrorCode::SoundIo => 5,
        ErrorCode::SessionGone
        | ErrorCode::ResyncRequired
        | ErrorCode::JobNotRetained
        | ErrorCode::Cancelled
        | ErrorCode::QueueFull
        | ErrorCode::Failed
        | ErrorCode::Io
        | ErrorCode::WorkerFailure => 7,
    }
}

fn frame_error_message(error: &EditError) -> &'static str {
    match error {
        EditError::Control(engine::control::ControlError::Limit) => {
            "control request exceeds the frame limit"
        }
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
        "error": {"code": code, "message": message, "exit_code": exit},
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
            return Err(EditError::Control(engine::control::ControlError::Limit).into());
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

#[cfg(test)]
mod failure_frames {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    #[test]
    fn every_code_has_the_preserved_control_exit_and_serde_name() {
        use ErrorCode as C;
        let cases = [
            (C::UnknownCommand, 2),
            (C::InvalidParams, 2),
            (C::Limit, 2),
            (C::SchemaMismatch, 2),
            (C::Budget, 2),
            (C::StaleRevision, 3),
            (C::StaleResult, 3),
            (C::SessionGone, 7),
            (C::ResyncRequired, 7),
            (C::JobNotRetained, 7),
            (C::Cancelled, 7),
            (C::QueueFull, 7),
            (C::Failed, 7),
            (C::Io, 7),
            (C::WorkerFailure, 7),
            (C::Permission, 5),
            (C::IdempotencyConflict, 5),
            (C::ReceiptCapacity, 5),
            (C::TransactionActive, 5),
            (C::EmptyProject, 5),
            (C::Refused, 5),
            (C::Unavailable, 5),
            (C::Setup, 5),
            (C::Auth, 5),
            (C::Busy, 5),
            (C::Conflict, 5),
            (C::Media, 5),
            (C::Render, 5),
            (C::Ai, 5),
            (C::Job, 5),
            (C::Quota, 5),
            (C::RateLimit, 5),
            (C::Network, 5),
            (C::AudioDevice, 5),
            (C::Sounds, 5),
            (C::SoundLimit, 5),
            (C::SoundCancelled, 5),
            (C::SoundIo, 5),
        ];
        for (code, exit) in cases {
            let mut bytes = Vec::new();
            write_control_error(&mut bytes, Some("request"), code, "safe").unwrap();
            let frame: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(frame["exit_code"], exit);
            assert_eq!(frame["error"]["exit_code"], exit);
            assert_eq!(frame["error"]["code"], json!(code));
            assert_eq!(frame["request_id"], "request");
        }
    }
}
