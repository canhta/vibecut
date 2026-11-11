//! Owns the properties area: its presentation state, the whole-panel `show`
//! interface and the actions the frame routes on its behalf.
mod color_picker;
pub mod font_picker;
mod gesture;
mod mask_picker;
mod read;
mod registry;
mod shared_field;
mod state;
mod view;
mod workflow;

pub use color_picker::ColorTarget;
pub use registry::{ElementKind, PropertiesTab};
pub use shared_field::{SharedField, shared_fields};
pub(in crate::editor) use state::{Action, Widget, element_kind};
pub(in crate::editor) use view::show;
pub(in crate::editor) use workflow::{
    add_effect_to_item, apply, apply_text_animation, effect_refusal, set_item_field,
    set_shared_field,
};

use crate::editor::{
    Arc, BTreeSet, BUTTON_ICON, Color32, Context, Error, Icon, Id, ItemId, ItemParams, Rect,
    Result, SelectionScope, StreamKind, Text, Tick, TrackKind, Vec2, egui, icons, theme,
};
use gesture::{
    AUDIO_GAIN_SCRUB, EffectReorder, OPACITY_SCRUB, SPEED_SCRUB, ScrubDrag, ScrubSpec, ScrubTarget,
    TEXT_SIZE_SCRUB, effect_scrub_spec,
};

use workflow::transform_scrub;

/// Transient properties presentation state, independent of every other area.
pub(in crate::editor) struct State {
    /// Uncommitted text of fields being edited, by field key.
    drafts: std::collections::BTreeMap<String, String>,
    /// Selection the drafts belong to.
    selection: BTreeSet<ItemId>,
    /// Last field that held keyboard focus. Egui surrenders focus on Escape
    /// before app UI runs, so the next frame must still defer shortcuts.
    focus: Option<String>,
    /// One Scale control moves both axes; a view setting, not project data.
    uniform_scale: bool,
    /// Project/Details switch shown while no single element is selected.
    details: bool,
    scrub: Option<ScrubDrag>,
    spatial_edit: Option<(ItemId, ScrubTarget, Tick)>,
    opacity_edit: Option<(Arc<project::Project>, ItemId, Tick)>,
    reorder: Option<gesture::EffectReorder>,
    picker: Option<color_picker::Draft>,
    /// Project the open picker previews against; a different project closes it.
    color_base: Option<Arc<project::Project>>,
    /// Font picker popover, its catalog and download.
    font_picker: font_picker::State,
    /// Catalog commands for the font picker; opened lazily on first use.
    font_backend: Option<Box<dyn font_picker::Backend>>,
    lut: workflow::lut::LutWorkflow,
    /// Mask row's shape and field sub-panel.
    mask: mask_picker::State,
    /// Clip whose Chroma key is switched on before any colour is picked; the
    /// first pick adds the effect, so no effect exists while this is set.
    chroma_staged: Option<ItemId>,
    /// Rect of the last drawn panel, where the picker hangs off.
    area: Option<Rect>,
    widgets: Vec<Widget>,
}

impl State {
    /// The Project/Details switch reads as details-only.
    pub(in crate::editor) fn details(&self) -> bool {
        self.details
    }

    pub(in crate::editor) fn set_details(&mut self, details: bool) {
        self.details = details;
    }

    /// Whether the colour popover owns Escape and outside clicks.
    pub(in crate::editor) fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Automation id of the scrub handle the open drag started from.
    pub(in crate::editor) fn scrub_widget(&self) -> Option<String> {
        self.scrub.as_ref().map(|scrub| scrub.target.widget_id())
    }

    /// Automation id of the effect row handle an open reorder started from.
    pub(in crate::editor) fn reorder_handle(&self) -> Option<String> {
        self.reorder
            .as_ref()
            .map(|drag| format!("effects.row.{}.handle", drag.effect))
    }

    /// Set the catalog backend the facade injects.
    pub(in crate::editor) fn set_font_backend(&mut self, backend: Box<dyn font_picker::Backend>) {
        self.font_backend = Some(backend);
    }

    /// Wire the My fonts tab once font import (80) supplies imported families.
    pub(in crate::editor) fn set_font_imports_available(&mut self, available: bool) {
        self.font_picker.has_imports = available;
    }

    /// Font picker state for the automation read.
    pub(in crate::editor) fn font_inspection(
        &self,
        preferences: &crate::preferences::FontPickerPreferences,
    ) -> font_picker::Inspection {
        self.font_picker.inspect(preferences)
    }

    /// The open colour draft, for the automation read.
    pub(in crate::editor) fn color_preview(&self) -> Option<project::Color> {
        self.picker.as_ref().map(|draft| draft.picker.color)
    }

    /// The armed screen sampler's state, for the automation read.
    pub(in crate::editor) fn eyedropper(
        &self,
    ) -> Option<crate::services::color_picker::EyedropperState> {
        self.picker.as_ref().map(|draft| draft.picker.native.state)
    }

    /// Inject the native capability boundary for an open picker.
    pub(in crate::editor) fn set_eyedropper(
        &mut self,
        sampler: crate::services::color_picker::NativeEyedropper,
    ) -> Result<()> {
        self.picker
            .as_mut()
            .ok_or(Error::Invalid("closed colour picker"))?
            .picker
            .native = sampler;
        Ok(())
    }
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
            spatial_edit: None,
            opacity_edit: None,
            reorder: None,
            picker: None,
            color_base: None,
            font_picker: font_picker::State::default(),
            font_backend: None,
            lut: workflow::lut::LutWorkflow::default(),
            mask: mask_picker::State::default(),
            chroma_staged: None,
            area: None,
            widgets: Vec::new(),
        }
    }
}
