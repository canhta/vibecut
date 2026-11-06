//! Validated item-relative channels. Rendering and editing share this evaluator.

mod bezier;
mod fade;
mod keyframes;
mod params;
mod retime;

pub use fade::{Fade, FadeCurve, FadeEdge, FadeSpans, Fades};
pub use keyframes::{
    Channel, CurveHandle, Extrapolation, HandleSide, Keyframe, MAX_CHANNEL_KEYS, Segment,
    TangentMode,
};
pub use params::{
    AnimationPath, Animations, ColorComponent, MAX_ANIMATION_PATHS, MAX_PROJECT_KEYS, Param,
    ParamValue,
};
pub use retime::{Cut, Retime, RetimeWindow};
