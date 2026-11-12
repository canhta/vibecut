//! Validated headless timeline snapshots and versioned project persistence.

mod animation;
mod model;
mod persistence;
mod visual;

pub use animation::{
    AnimationPath, Animations, BuiltinPath, Channel, ColorComponent, CurveHandle, Cut, EasingCurve,
    EasingPreset, Extrapolation, Fade, FadeCurve, FadeEdge, FadeSpans, Fades, HandleSide, Keyframe,
    MAX_ANIMATION_PATHS, MAX_CHANNEL_KEYS, MAX_PROJECT_KEYS, Param, ParamValue, Retime,
    RetimeWindow, Segment, TangentMode,
};
pub use model::*;
pub use persistence::atomic;
pub use persistence::autosave;
pub use persistence::{
    FORMAT_ID, Loaded, MAX_SEPARATORS, METADATA_VERSION, Metadata, SCHEMA_VERSION, SaveReceipt,
    Thumbnail, ViewState, WriterLease, atomic_write, decode, encode, load, metadata_path,
    read_metadata,
};
pub use visual::{
    ANGLE_MAX, BlendMode, CHROMA_KEY_COLOR_DEFAULT, CHROMA_KEY_INTENSITY_DEFAULT, CHROMA_KEY_MAX,
    CHROMA_KEY_MIN, CURVE_GRID, CURVE_MAX_POINTS, CURVE_MIN_POINTS, Crop, DEFAULT_WEIGHT,
    DistributionAxis, EffectDefinition, EffectInstance, EffectParamDefinition, EffectParamKind,
    EffectParamUpdate, EffectShelf, EffectValue, FlipAxis, GAIN_MAX, GAIN_MIN, GAMMA_MAX,
    GAMMA_MIN, GRADE_BASIC, GRADE_MAX, GRADE_MIN, HSL_BANDS, HSL_CONTROLS, ItemAlignment, LIFT_MAX,
    LIFT_MIN, LutId, LutRecord, LutReference, MASK_SCALARS, MAX_LETTER_SPACING, MAX_LINE_HEIGHT,
    Mask, MaskMode, MaskPath, MaskShape, MaskStroke, MaskVertex, PACKED_COLOR_MAX, POINT_MAX,
    StrokeAlign, TRANSITION_DEFINITIONS, ToneCurve, TransformField, Transition,
    TransitionAlignment, TransitionDefinition, TransitionId, TransitionParamDef,
    TransitionParamKind, TransitionParams, TransitionValue, effect_definition, effect_definitions,
    packed_color, transition_definition, valid_letter_spacing, valid_line_height, valid_weight,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("project identity generation failed")]
    Identity(#[source] getrandom::Error),
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
    #[error("unsupported transition kind: {0}")]
    UnsupportedTransition(String),
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
