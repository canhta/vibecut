//! Persisted visual item properties and their bounds.

mod blend;
mod bounds;
mod effects;
mod mask;
mod transform;
mod transition;
mod typography;

pub use blend::BlendMode;
pub(crate) use bounds::{MAX_BYTES, Writer, check};
pub use effects::{
    EffectDefinition, EffectInstance, EffectParamDefinition, EffectParams, GRADE_MAX, GRADE_MIN,
    effect_definitions,
};
pub use mask::{
    MASK_SCALARS, Mask, MaskMode, MaskPath, MaskShape, MaskStroke, MaskVertex, StrokeAlign,
};
pub use transform::TransformField;
pub use transition::{
    TRANSITION_DEFINITIONS, Transition, TransitionAlignment, TransitionDefinition, TransitionId,
    transition_definition,
};
pub(crate) use typography::valid;
pub use typography::{
    DEFAULT_WEIGHT, MAX_LETTER_SPACING, MAX_LINE_HEIGHT, valid_letter_spacing, valid_line_height,
    valid_weight,
};
