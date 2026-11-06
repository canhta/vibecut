//! Owns the properties presentation boundary.
mod view;
mod workflow;
pub(super) use view::show;

use super::{
    AUDIO_GAIN_SCRUB, Arc, BTreeSet, BUTTON_ICON, Color32, ColorTarget, Editor, EffectReorder,
    ElementKind, Error, Icon, Id, ItemId, ItemParamPatch, ItemParams, OPACITY_SCRUB, Picker,
    PropertiesTab, Rect, Result, SPEED_SCRUB, ScrubDrag, ScrubSpec, ScrubTarget, SelectionScope,
    Session, StreamKind, TEXT_SIZE_SCRUB, Text, Tick, TrackKind, Vec2, catalog, effect_definition,
    effect_params_from_definition, effect_scrub_spec, egui, error_detail, icons, shared_fields,
    theme,
};

pub(super) use view::show_dialog;

mod state;

use workflow::transform_scrub;
