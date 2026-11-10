//! Persisted visual item properties.

mod blend;
mod crop;
mod effects;
mod effects_catalog;
mod mask;
mod transform;
mod transition;
mod transition_catalog;
mod transition_params;
mod typography;

pub use blend::BlendMode;
pub use crop::Crop;
pub use effects::{
    ANGLE_MAX, CHROMA_KEY_COLOR_DEFAULT, CHROMA_KEY_INTENSITY_DEFAULT, CHROMA_KEY_MAX,
    CHROMA_KEY_MIN, EffectDefinition, EffectInstance, EffectParamDefinition, EffectParamKind,
    EffectShelf, EffectValue, GRADE_MAX, GRADE_MIN, PACKED_COLOR_MAX, POINT_MAX, effect_definition,
    effect_definitions, packed_color,
};
pub use mask::{
    MASK_SCALARS, Mask, MaskMode, MaskPath, MaskShape, MaskStroke, MaskVertex, StrokeAlign,
};
pub use transform::TransformField;
pub use transition::{Transition, TransitionAlignment, TransitionId};
pub use transition_catalog::{TRANSITION_DEFINITIONS, TransitionDefinition, transition_definition};
pub use transition_params::{
    TransitionParamDef, TransitionParamKind, TransitionParams, TransitionValue,
};
pub(crate) use typography::valid;
pub use typography::{
    DEFAULT_WEIGHT, MAX_LETTER_SPACING, MAX_LINE_HEIGHT, valid_letter_spacing, valid_line_height,
    valid_weight,
};
