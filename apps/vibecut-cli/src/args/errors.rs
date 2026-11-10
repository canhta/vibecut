//! Maps typed CLI refusals to stable process diagnostics.
use engine::{EditError, Failure, control::ErrorCode, providers};
use serde_json::{Value, json};
use std::io;
use std::io::Write as _;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Project(#[from] project::Error),
    #[error(transparent)]
    Engine(#[from] EditError),
    #[error(transparent)]
    Shorts(#[from] shorts::Error),
    #[error(transparent)]
    Render(#[from] render::Error),
    #[error(transparent)]
    Media(#[from] media::MediaError),
    #[error(transparent)]
    Time(#[from] time::TimeError),
    #[error(transparent)]
    Sounds(#[from] engine::SoundError),
    #[error("{0}")]
    Job(Arc<EditError>),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Rejected(&'static str),
    #[error("{0}")]
    Unsupported(&'static str),
}
impl CliError {
    /// Stable machine code and process exit status. Documented in docs/product/agent-control/cli.md.
    pub fn code(&self) -> (&'static str, u8) {
        match self {
            Self::Usage(_) | Self::Json(_) => ("usage", 2),
            Self::Project(project::Error::Busy) => ("busy", 3),
            Self::Project(project::Error::Conflict) => ("conflict", 3),
            Self::Project(project::Error::Io(_)) => ("io", 7),
            Self::Project(project::Error::Media(_)) => ("media", 6),
            Self::Project(_) => ("rejected", 5),
            Self::Engine(error) => failure_code(error.code()),
            Self::Shorts(shorts::Error::Engine(error)) => failure_code(error.code()),
            Self::Shorts(_) => ("rejected", 5),
            Self::Render(render::Error::Unavailable(_)) => ("unavailable", 4),
            Self::Render(_) => ("render", 6),
            Self::Media(_) => ("media", 6),
            Self::Time(_) => ("time", 5),
            Self::Sounds(error) => failure_code(error.code()),
            Self::Job(error) => failure_code(error.code()),
            Self::Io(_) => ("io", 7),
            Self::Rejected(_) => ("rejected", 5),
            Self::Unsupported(_) => ("unsupported", 5),
        }
    }
    pub fn exit_code(&self) -> u8 {
        self.code().1
    }
    /// Safe diagnostics: never project text or provider payloads.
    pub fn report(&self, json_output: bool) {
        let line = if json_output {
            json!({"status": "error", "error": self.value()}).to_string()
        } else {
            format!("error: {}", self.message())
        };
        let _ = writeln!(io::stderr(), "{line}");
    }
    /// The setup hint is the CLI's own text; the kernel names no command.
    fn message(&self) -> String {
        match self {
            Self::Engine(error) => edit_message(error),
            Self::Job(error) => edit_message(error),
            Self::Shorts(shorts::Error::Engine(error)) => edit_message(error),
            _ => self.to_string(),
        }
    }
    /// One typed error object for both stderr envelopes and batch job results.
    pub(crate) fn value(&self) -> Value {
        let (code, exit) = self.code();
        let mut error = json!({
            "code": code,
            "message": self.message(),
            "exit_code": exit,
        });
        if let Some(list) = self.unavailable() {
            let unavailable: Vec<Value> = list.iter().map(unavailable_json).collect();
            if let Value::Object(fields) = &mut error {
                let _ = fields.insert("unavailable".into(), Value::Array(unavailable));
            }
        }
        error
    }
    fn unavailable(&self) -> Option<&[render::Unavailable]> {
        match self {
            Self::Render(render::Error::Unavailable(list)) => Some(list),
            Self::Job(error) => match error.as_ref() {
                EditError::Render(render::Error::Unavailable(list)) => Some(list),
                _ => None,
            },
            _ => None,
        }
    }
}

fn edit_message(error: &EditError) -> String {
    match error {
        EditError::Providers(providers::Error::ModelMissing(id)) => {
            format!("local model {id} is not installed; run `vibecut-cli model download {id}`")
        }
        EditError::JobFailure(error) => edit_message(error),
        _ => error.to_string(),
    }
}

fn failure_code(code: ErrorCode) -> (&'static str, u8) {
    match code {
        ErrorCode::UnknownCommand | ErrorCode::InvalidParams | ErrorCode::Limit => ("usage", 2),
        ErrorCode::Unavailable => ("unavailable", 4),
        ErrorCode::Setup => ("setup", 4),
        ErrorCode::Auth => ("auth", 4),
        ErrorCode::Busy => ("busy", 3),
        ErrorCode::Conflict => ("conflict", 3),
        ErrorCode::Io | ErrorCode::SoundIo => ("io", 7),
        ErrorCode::Media => ("media", 6),
        ErrorCode::Render => ("render", 6),
        ErrorCode::Ai => ("ai", 4),
        ErrorCode::Job
        | ErrorCode::QueueFull
        | ErrorCode::Budget
        | ErrorCode::StaleResult
        | ErrorCode::WorkerFailure
        | ErrorCode::Cancelled => ("job", 7),
        ErrorCode::Quota => ("quota", 4),
        ErrorCode::RateLimit => ("rate_limit", 4),
        ErrorCode::Network => ("network", 7),
        ErrorCode::AudioDevice => ("audio_device", 4),
        ErrorCode::Sounds => ("sounds", 5),
        ErrorCode::SoundLimit => ("limit", 2),
        ErrorCode::SoundCancelled => ("cancelled", 7),
        ErrorCode::SchemaMismatch
        | ErrorCode::SessionGone
        | ErrorCode::StaleRevision
        | ErrorCode::Permission
        | ErrorCode::IdempotencyConflict
        | ErrorCode::ReceiptCapacity
        | ErrorCode::ResyncRequired
        | ErrorCode::JobNotRetained
        | ErrorCode::TransactionActive
        | ErrorCode::Refused
        | ErrorCode::Failed
        | ErrorCode::EmptyProject => ("rejected", 5),
    }
}

fn unavailable_json(value: &render::Unavailable) -> Value {
    json!({
        "asset": value.asset.0,
        "item": value.item.0,
        "source_start": value.range.start().0,
        "source_end": value.range.end().0,
        "reason": reason_name(value.reason),
    })
}
fn reason_name(reason: render::UnavailableReason) -> &'static str {
    match reason {
        render::UnavailableReason::Missing => "missing",
        render::UnavailableReason::Changed => "changed",
        render::UnavailableReason::Metadata => "metadata",
        render::UnavailableReason::Video => "video",
        render::UnavailableReason::Audio => "audio",
    }
}
