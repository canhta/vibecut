//! Fixed labelled evaluation of the clip pipeline (selection + boundary snapping).
//! Reports are local artifacts under `target/clip-eval/`.
pub mod asr;
pub mod baseline;
pub mod budget;
pub mod harness;
pub mod label;
pub mod metrics;
pub mod transcript;

pub use harness::{Options, Provider, Report, RunOutput, run};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Label(#[from] label::LabelError),
    #[error("label set is missing: {0}")]
    LabelsMissing(String),
    #[error("label set has no JSON label files: {0}")]
    EmptySet(String),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Captions(#[from] captions::Error),
    #[error(transparent)]
    Time(#[from] time::TimeError),
    #[error(transparent)]
    Media(#[from] media::MediaError),
    #[error(transparent)]
    Ai(#[from] ai::Error),
    #[error(transparent)]
    Engine(#[from] engine::EditError),
    #[error("invalid options: {0}")]
    Options(String),
    #[error("missing transcript: {0}")]
    MissingTranscript(String),
    #[error("transcript language mismatch: expected {expected}, actual {actual}")]
    TranscriptLanguage { expected: String, actual: String },
    #[error("live LLM calls require --live")]
    LiveRequired,
    #[error("provider cost estimate is unknown")]
    UnknownCost,
    #[error("cost estimate {estimate} micros exceeds cap {cap} micros")]
    CostCap { estimate: u64, cap: u64 },
    #[error("payload exceeds a bound")]
    Payload,
    #[error("pinned baseline is missing: {0}")]
    BaselineMissing(String),
    #[error("pinned baseline is invalid")]
    BaselineInvalid,
    #[error("pinned baseline does not cover this label set")]
    BaselineVideoSet,
    #[error("pinned baseline input differs for label {0}")]
    BaselineMismatch(String),
}
