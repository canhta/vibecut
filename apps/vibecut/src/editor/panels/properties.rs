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

pub use color_picker::{ColorTarget, TextPart};
pub use registry::{ElementKind, PropertiesTab};
pub use shared_field::{SharedField, shared_fields};
pub(in crate::editor) use state::{ANIMATION_DRAFT, Action, Widget, element_kind};
pub(in crate::editor) use view::show;
pub(in crate::editor) use workflow::{
    add_effect_to_item, apply, effect_refusal, set_item_field, set_shared_field,
};

use crate::editor::{
    Arc, BTreeSet, Color32, Context, Icon, Id, ItemId, ItemParams, Rect, SelectionScope,
    StreamKind, Text, Tick, TrackKind, Vec2, egui, icons, theme,
};
use gesture::{
    AUDIO_GAIN_SCRUB, EffectReorder, GraphDrag, OPACITY_SCRUB, SPEED_SCRUB, ScrubDrag, ScrubSpec,
    ScrubTarget, TEXT_SIZE_SCRUB, TRANSITION_DURATION_SCRUB, effect_scrub_spec,
};

use workflow::transform_scrub;

/// The Adjust tab's sub-tabs in reference order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum AdjustTab {
    #[default]
    Basic,
    Hsl,
    Curves,
    Wheel,
}
impl AdjustTab {
    pub(in crate::editor) const ALL: [Self; 4] =
        [Self::Basic, Self::Hsl, Self::Curves, Self::Wheel];
    pub(in crate::editor) const fn id(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Hsl => "hsl",
            Self::Curves => "curves",
            Self::Wheel => "wheel",
        }
    }
    pub(in crate::editor) const fn label(self) -> Text {
        match self {
            Self::Basic => Text::Basic,
            Self::Hsl => Text::Hsl,
            Self::Curves => Text::Curves,
            Self::Wheel => Text::ColorWheel,
        }
    }
}

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
    /// The blend mode an unchecked Blend checkbox replaced with Normal, so a re-check
    /// restores it. A view memory only: the project's blend mode is the one truth.
    blend_restore: Option<(ItemId, project::BlendMode)>,
    /// Project/Details switch shown while no single element is selected.
    details: bool,
    /// Set by Modify: the next Settings frame focuses and scrolls to the canvas width.
    canvas_focus: bool,
    scrub: Option<ScrubDrag>,
    spatial_edit: Option<(ItemId, ScrubTarget, Tick)>,
    /// Wheel or curve drag in progress and whether it wrote a value.
    graph: Option<GraphDrag>,
    graph_changed: bool,
    /// Selected tone-curve channel, an index into the view's channel list.
    curve_channel: usize,
    /// Selected Adjust sub-tab and HSL band.
    adjust_tab: AdjustTab,
    /// Animation sub-tab (In, Out or Combo) shown by the shared Animation control.
    animation_slot: engine::AnimationSlot,
    hsl_band: usize,
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
    font_import: Option<font_picker::PendingImport>,
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

    pub(in crate::editor) fn request_canvas_focus(&mut self) {
        self.canvas_focus = true;
    }

    /// Takes the pending Modify focus request.
    pub(in crate::editor) fn take_canvas_focus(&mut self) -> bool {
        std::mem::take(&mut self.canvas_focus)
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
        self.font_import = None;
        self.font_picker = font_picker::State::default();
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

    /// Whether the colour popover's eyedropper is armed: the player samples its next click.
    pub(in crate::editor) fn sampling_colour(&self) -> bool {
        self.picker.as_ref().is_some_and(|draft| draft.picker.sampling)
    }

    /// Hand the player's sampled pixel to the open popover, which applies it on its next pass.
    pub(in crate::editor) fn sample_colour(&mut self, color: project::Color) {
        if let Some(draft) = self.picker.as_mut() {
            draft.picker.sampled = Some(color);
        }
    }

    /// Leave the eyedropper's sampling mode without closing the popover.
    pub(in crate::editor) fn stop_sampling(&mut self) {
        if let Some(draft) = self.picker.as_mut() {
            draft.picker.sampling = false;
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            drafts: std::collections::BTreeMap::new(),
            selection: BTreeSet::new(),
            focus: None,
            uniform_scale: true,
            blend_restore: None,
            details: false,
            canvas_focus: false,
            scrub: None,
            spatial_edit: None,
            graph: None,
            graph_changed: false,
            curve_channel: 0,
            adjust_tab: AdjustTab::default(),
            animation_slot: engine::AnimationSlot::In,
            hsl_band: 0,
            opacity_edit: None,
            reorder: None,
            picker: None,
            color_base: None,
            font_picker: font_picker::State::default(),
            font_backend: None,
            lut: workflow::lut::LutWorkflow::default(),
            font_import: None,
            mask: mask_picker::State::default(),
            chroma_staged: None,
            area: None,
            widgets: Vec::new(),
        }
    }
}
