//! Persisted visual item properties.

mod blend;
mod crop;
mod curve;
mod effects;
mod lut;
mod mask;
pub use lut::{LutId, LutRecord, LutReference};
mod transform;
mod transition;
mod transition_catalog;
mod transition_params;
mod typography;

pub use blend::BlendMode;
pub use crop::{Crop, FitBox, FitMode, cropped_source_size, fit_box};
pub use curve::{CURVE_GRID, CURVE_MAX_POINTS, CURVE_MIN_POINTS, ToneCurve};
pub use effects::{
    ANGLE_MAX, CHROMA_KEY_COLOR_DEFAULT, CHROMA_KEY_INTENSITY_DEFAULT, CHROMA_KEY_MAX,
    CHROMA_KEY_MIN, EffectDefinition, EffectInstance, EffectParamDefinition, EffectParamKind,
    EffectParamUpdate, EffectShelf, EffectValue, FILTER_INSTANCE_ID, FilterDefinition, FilterShelf,
    GAIN_MAX, GAIN_MIN, GAMMA_MAX, GAMMA_MIN, GRADE_BASIC, GRADE_MAX, GRADE_MIN, HSL_BANDS,
    HSL_CONTROLS, LIFT_MAX, LIFT_MIN, PACKED_COLOR_MAX, POINT_MAX, effect_definition,
    effect_definitions, filter_definition, filter_definitions, filter_look, is_filter_clip,
    packed_color,
};
pub use mask::{
    MASK_SCALARS, Mask, MaskMode, MaskPath, MaskShape, MaskStroke, MaskVertex, StrokeAlign,
};
pub use transform::{DistributionAxis, FlipAxis, ItemAlignment, TransformField};
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
