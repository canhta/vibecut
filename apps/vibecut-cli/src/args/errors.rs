//! Maps typed CLI refusals to stable process diagnostics.
use engine::EditError;
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
            Self::Engine(error) => edit_code(error),
            Self::Shorts(shorts::Error::Engine(error)) => edit_code(error),
            Self::Shorts(_) => ("rejected", 5),
            Self::Render(render::Error::Unavailable(_)) => ("unavailable", 4),
            Self::Render(_) => ("render", 6),
            Self::Media(_) => ("media", 6),
            Self::Time(_) => ("time", 5),
            Self::Sounds(error) => sound_code(error),
            Self::Job(error) => edit_code(error),
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
            format!("error: {self}")
        };
        let _ = writeln!(io::stderr(), "{line}");
    }
    /// One typed error object for both stderr envelopes and batch job results.
    pub(crate) fn value(&self) -> Value {
        let (code, exit) = self.code();
        let mut error = json!({
            "code": code,
            "message": self.to_string(),
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

fn edit_code(error: &EditError) -> (&'static str, u8) {
    match error {
        EditError::Render(render::Error::Unavailable(_)) => ("unavailable", 4),
        EditError::Render(_) => ("render", 6),
        EditError::Media(_) => ("media", 6),
        EditError::ModelMissing(_) | EditError::RunnerMissing | EditError::CredentialMissing(_) => {
            ("setup", 4)
        }
        EditError::Ai(ai::Error::Chapters(_)) => ("rejected", 5),
        EditError::Ai(_) => ("ai", 4),
        EditError::JobFailure(error) => edit_code(error),
        EditError::ConsentRequired | EditError::AutoLanguageUnsupported => ("rejected", 5),
        EditError::Project(project::Error::Busy) => ("busy", 3),
        EditError::Project(project::Error::Conflict) => ("conflict", 3),
        EditError::Project(project::Error::Io(_)) => ("io", 7),
        EditError::Project(project::Error::Media(_)) => ("media", 6),
        EditError::UnknownCommand | EditError::InvalidParams | EditError::Limit => ("usage", 2),
        EditError::Io(_) => ("io", 7),
        EditError::Cancelled
        | EditError::WorkerPanicked
        | EditError::JobPoisoned
        | EditError::QueueFull
        | EditError::BudgetFull
        | EditError::ArtifactTooLarge
        | EditError::NetworkTransient
        | EditError::StaleResult
        | EditError::CacheCorrupt => ("job", 7),
        _ => ("rejected", 5),
    }
}

fn sound_code(error: &engine::SoundError) -> (&'static str, u8) {
    match error {
        engine::SoundError::NoCredentials => ("setup", 4),
        engine::SoundError::Auth => ("auth", 4),
        engine::SoundError::Quota => ("quota", 4),
        engine::SoundError::RateLimit { .. } => ("rate_limit", 4),
        engine::SoundError::Network | engine::SoundError::Server => ("network", 7),
        engine::SoundError::NoAudioDevice => ("audio_device", 4),
        engine::SoundError::Cancelled => ("cancelled", 7),
        engine::SoundError::Limit => ("limit", 2),
        engine::SoundError::NotCached
        | engine::SoundError::InvalidResponse
        | engine::SoundError::LibraryCorrupt
        | engine::SoundError::Invalid(_) => ("sounds", 5),
        engine::SoundError::Io(_) | engine::SoundError::Project(_) => ("io", 7),
        engine::SoundError::Media(_) => ("media", 6),
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
