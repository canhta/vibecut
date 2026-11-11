//! Maps typed CLI refusals to stable process diagnostics.
use engine::{Error as EngineError, Failure, control::ErrorCode, providers};
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
    Engine(#[from] EngineError),
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
    Job(Arc<dyn Failure + 'static>),
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
            Self::Job(error) => edit_message(error.as_ref()),
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
            Self::Job(error) => {
                let raw = error.as_ref() as &(dyn std::error::Error + 'static);
                match raw.downcast_ref::<EngineError>() {
                    Some(EngineError::Render(render::Error::Unavailable(list)))
                    | Some(EngineError::Runtime(engine::RuntimeError::Render(
                        render::Error::Unavailable(list),
                    )))
                    | Some(EngineError::Lut(engine::luts::Error::Render(
                        render::Error::Unavailable(list),
                    ))) => Some(list),
                    _ => match raw.downcast_ref::<engine::RuntimeError>() {
                        Some(engine::RuntimeError::Render(render::Error::Unavailable(list))) => {
                            Some(list)
                        }
                        _ => None,
                    },
                }
            }
            _ => None,
        }
    }
}

fn edit_message(error: &(dyn Failure + 'static)) -> String {
    let cause: &(dyn std::error::Error + 'static) = error;
    let root = cause.downcast_ref::<EngineError>();
    let subtitles = match root {
        Some(EngineError::Subtitles(subtitles)) => Some(subtitles),
        _ => cause.downcast_ref::<engine::subtitles::Error>(),
    };
    let transcript = match subtitles {
        Some(engine::subtitles::Error::Transcript(transcript)) => Some(transcript),
        _ => match root {
            Some(EngineError::Transcript(transcript)) => Some(transcript),
            _ => cause.downcast_ref::<engine::transcript::Error>(),
        },
    };
    let providers = match subtitles {
        Some(engine::subtitles::Error::Providers(providers)) => Some(providers),
        _ => match transcript {
            Some(engine::transcript::Error::Providers(providers)) => Some(providers),
            _ => match root {
                Some(EngineError::Providers(providers)) => Some(providers),
                _ => cause.downcast_ref::<providers::Error>(),
            },
        },
    };
    if let Some(providers::Error::ModelMissing(id)) = providers {
        return format!("local model {id} is not installed; run `vibecut-cli model download {id}`");
    }
    let runtime = match subtitles {
        Some(engine::subtitles::Error::Runtime(error)) => Some(error),
        _ => match transcript {
            Some(engine::transcript::Error::Runtime(error)) => Some(error),
            _ => match root {
                Some(EngineError::Runtime(error)) => Some(error),
                _ => cause.downcast_ref::<engine::RuntimeError>(),
            },
        },
    };
    match runtime {
        Some(engine::RuntimeError::Failed(inner)) => edit_message(inner.as_ref()),
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

impl From<engine::RuntimeError> for CliError {
    fn from(error: engine::RuntimeError) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::EditError> for CliError {
    fn from(error: engine::EditError) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::models::Error> for CliError {
    fn from(error: engine::models::Error) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::providers::Error> for CliError {
    fn from(error: engine::providers::Error) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::transcript::Error> for CliError {
    fn from(error: engine::transcript::Error) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::subtitles::Error> for CliError {
    fn from(error: engine::subtitles::Error) -> Self {
        Self::Engine(error.into())
    }
}
