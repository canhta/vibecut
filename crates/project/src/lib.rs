//! Validated headless timeline snapshots and versioned project persistence.
pub mod atomic;
pub mod autosave;
mod bounds;
mod model;
mod persistence;
pub use atomic::atomic_write;
pub use model::*;
pub use persistence::{FORMAT_ID, Loaded, SCHEMA_VERSION, decode, encode, load, save};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid project: {0}")]
    Invalid(&'static str),
    #[error("project limit exceeded: {0}")]
    TooLarge(&'static str),
    #[error("unsupported project capability: {0}")]
    Unsupported(&'static str),
    #[error("project schema {found} is older than supported schema {supported}")]
    TooOld { found: u32, supported: u32 },
    #[error("project schema {found} is newer than supported schema {supported}")]
    TooNew { found: u32, supported: u32 },
    #[error("project I/O failed")]
    Io(#[from] std::io::Error),
    // JSON errors may embed user text; retain only safe line/column information.
    #[error("invalid project JSON at line {line}, column {column}")]
    Json { line: usize, column: usize },
    #[error("invalid project time")]
    Time(#[from] time::TimeError),
    #[error("invalid source asset")]
    Media(#[from] media::MediaError),
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json {
            line: e.line(),
            column: e.column(),
        }
    }
}
