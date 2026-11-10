//! Persisted visual item properties.

mod blend;
mod crop;
mod effects;
mod mask;
mod transform;
mod transition;
mod typography;

pub use blend::BlendMode;
pub use crop::Crop;
pub use effects::{
    CHROMA_KEY_COLOR_DEFAULT, CHROMA_KEY_COLOR_MAX, CHROMA_KEY_INTENSITY_DEFAULT, CHROMA_KEY_MAX,
    CHROMA_KEY_MIN, EffectDefinition, EffectInstance, EffectParamDefinition, EffectParams,
    EffectPlacement, GRADE_MAX, GRADE_MIN, chroma_key_color, effect_definitions,
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
