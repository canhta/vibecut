//! Validated headless timeline snapshots and versioned project persistence.
mod animation;
mod blend;
pub use blend::BlendMode;
mod effects;
mod mask;
pub use effects::{
    EffectDefinition, EffectInstance, EffectParamDefinition, EffectParams, effect_definitions,
};
pub use mask::{
    MASK_SCALARS, Mask, MaskMode, MaskPath, MaskShape, MaskStroke, MaskVertex, StrokeAlign,
};
pub mod atomic;
pub mod autosave;
mod bezier;
mod bounds;
pub use animation::{
    AnimationPath, Animations, Channel, ColorComponent, CurveHandle, Extrapolation, HandleSide,
    Keyframe, MAX_ANIMATION_PATHS, MAX_CHANNEL_KEYS, MAX_PROJECT_KEYS, Param, ParamValue, Segment,
    TangentMode,
};
mod metadata;
/// Read optional bounded metadata without acquiring an editing lease (catalog display).
pub fn read_metadata(path: &std::path::Path) -> Metadata {
    metadata::load(path)
}
mod model;
pub use metadata::{METADATA_VERSION, Metadata, Thumbnail, ViewState, path as metadata_path};
mod persistence;
mod writer;
pub use atomic::atomic_write;
pub use model::*;
pub use persistence::{FORMAT_ID, Loaded, SCHEMA_VERSION, decode, encode, load};
pub use writer::{SaveReceipt, WriterLease};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("project writer is busy or acquisition was cancelled")]
    Busy,
    #[error("project data or writer lease changed on disk")]
    Conflict,
    #[error("project filesystem cannot provide a writer lease")]
    LockUnavailable(#[source] std::io::Error),
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
