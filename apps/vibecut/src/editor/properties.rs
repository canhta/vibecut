//! Owns the properties area: its presentation state, the whole-panel `show`
//! interface and the actions the frame routes on its behalf.
pub mod font_picker;
mod read;
mod state;
mod view;
mod workflow;

pub(super) use state::{Action, Widget, element_kind, single};
pub(super) use view::show;
pub(super) use workflow::apply;

use super::{
    AUDIO_GAIN_SCRUB, Arc, BTreeSet, BUTTON_ICON, Color32, ColorTarget, Context, Editor,
    EffectReorder, ElementKind, Error, Icon, Id, ItemId, ItemParamPatch, ItemParams, OPACITY_SCRUB,
    Picker, PropertiesTab, Rect, Result, SPEED_SCRUB, ScrubDrag, ScrubSpec, ScrubTarget,
    SelectionScope, Session, StreamKind, TEXT_SIZE_SCRUB, Text, Tick, TrackKind, Vec2, catalog,
    effect_definition, effect_params_from_definition, effect_scrub_spec, egui, error_detail, icons,
    shared_fields, theme,
};

use workflow::transform_scrub;

/// Transient properties presentation state, independent of every other area.
pub(super) struct State {
    /// Uncommitted text of fields being edited, by field key.
    pub(super) drafts: std::collections::BTreeMap<String, String>,
    /// Selection the drafts belong to.
    selection: BTreeSet<ItemId>,
    /// Last field that held keyboard focus. Egui surrenders focus on Escape
    /// before app UI runs, so the next frame must still defer shortcuts.
    pub(super) focus: Option<String>,
    /// One Scale control moves both axes; a view setting, not project data.
    uniform_scale: bool,
    /// Project/Details switch shown while no single element is selected.
    pub(super) details: bool,
    pub(super) scrub: Option<ScrubDrag>,
    pub(super) reorder: Option<EffectReorder>,
    pub(super) picker: Option<Picker>,
    /// Project the open picker previews against; a different project closes it.
    pub(super) color_base: Option<Arc<project::Project>>,
    /// Font picker popover, its catalog and download.
    pub(super) font_picker: font_picker::State,
    /// Rect of the last drawn panel, where the picker hangs off.
    pub(super) area: Option<Rect>,
    widgets: Vec<Widget>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            drafts: std::collections::BTreeMap::new(),
            selection: BTreeSet::new(),
            focus: None,
            uniform_scale: true,
            details: false,
            scrub: None,
            reorder: None,
            picker: None,
            color_base: None,
            font_picker: font_picker::State::default(),
            area: None,
            widgets: Vec::new(),
        }
    }
}
