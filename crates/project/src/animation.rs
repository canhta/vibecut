//! Validated item-relative channels. Rendering and editing share this evaluator.

mod bezier;
mod easing;
mod fade;
mod keyframes;
mod params;
mod retime;
mod speed_curve;

pub use easing::{EasingCurve, EasingPreset};
pub use fade::{Fade, FadeCurve, FadeEdge, FadeSpans, Fades, MAX_FADE};
pub use keyframes::{
    Channel, CurveHandle, Extrapolation, HandleSide, Keyframe, MAX_CHANNEL_KEYS, Segment,
    TangentMode,
};
pub(crate) use params::identifier;
pub use params::{
    AnimationPath, Animations, BuiltinPath, ColorComponent, MAX_ANIMATION_PATHS, MAX_PROJECT_KEYS,
    Param, ParamValue,
};
pub use retime::{Cut, Retime, RetimeWindow};
pub use speed_curve::{
    MAX_SPEED_KEYS, SPEED_SPAN_TOTAL, SpeedCurve, SpeedKey, SpeedPreset, SpeedSegment,
};
