//! Retains preview-only view state between presented frames.
use super::crop;
use crate::{egui, i18n::Text, services::preview};
pub(crate) struct State {
    pub(in crate::editor) failed: bool,
    pub(in crate::editor) snapshot_failed: bool,
    pub(in crate::editor) view: preview::View,
    pub(in crate::editor) viewport: egui::Vec2,
    pub(in crate::editor) fullscreen: bool,
    pub(in crate::editor) fullscreen_commanded: bool,
    pub(in crate::editor) snapshot: preview::SnapshotDialog,
    pub(in crate::editor) crop: Option<crop::Dialog>,
    pub(in crate::editor) transform: super::transform::State,
    pub(in crate::editor) mask_path: super::mask_path::State,
    pub(in crate::editor) timecode_edit: Option<TimecodeEdit>,
    pub(in crate::editor) timecode_error: Option<Text>,
    pub(in crate::editor) texture: preview::FrameTexture,
    /// Armed chroma-key eyedropper, cleared by its pick, Escape or a selection change.
    pub(in crate::editor) key_color_pick: Option<KeyPick>,
    /// Armed matte refinement brush, cleared by Escape or a selection change.
    pub(in crate::editor) matte_brush: Option<MatteBrush>,
    /// The stroke the pointer is dragging, published when the button releases.
    pub(in crate::editor) brush_trail: Option<super::matte_brush::Trail>,
    /// The matte brush's Size, 1 to 100, kept while the brush is off.
    pub(in crate::editor) matte_brush_size: u32,
    /// Last view state read from the session metadata.
    pub(in crate::editor) last_view: project::ViewState,
    /// Grid size an open drag or typed edit shows; it is published once when the gesture ends.
    pub(in crate::editor) grid_draft: Option<GridDraft>,
    /// Custom margins, as typed or dragged, an open gesture shows before it publishes.
    /// Kept while invalid so the inline error stays until the value is fixed.
    pub(in crate::editor) margin_draft: Option<[i32; 4]>,
}

/// The grid size a picker gesture is drafting, as rows and columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct GridDraft {
    pub(in crate::editor) rows: u8,
    pub(in crate::editor) cols: u8,
}
impl State {
    pub(in crate::editor) fn new(fit: bool, zoom: f64) -> Self {
        Self {
            failed: false,
            snapshot_failed: false,
            view: preview::View::new(fit, zoom),
            viewport: egui::vec2(640.0, 360.0),
            fullscreen: false,
            fullscreen_commanded: false,
            snapshot: preview::SnapshotDialog::default(),
            crop: None,
            transform: super::transform::State::default(),
            mask_path: super::mask_path::State::default(),
            timecode_edit: None,
            timecode_error: None,
            texture: preview::FrameTexture::default(),
            key_color_pick: None,
            matte_brush: None,
            brush_trail: None,
            matte_brush_size: super::matte_brush::SIZE_DEFAULT,
            last_view: project::ViewState::default(),
            grid_draft: None,
            margin_draft: None,
        }
    }
}
/// Caret and selection geometry of the open inline text session.
#[derive(Clone, Debug, PartialEq)]
pub struct TextCaret {
    /// Primary caret as a character index in the live buffer.
    pub index: usize,
    /// Selected character range; ends are equal when the caret is collapsed.
    pub range: std::ops::Range<usize>,
    /// Caret endpoints in text-layout pixels, canvas units with y down.
    pub layout: [[f32; 2]; 2],
    /// Caret endpoints in logical screen pixels.
    pub screen: [[f32; 2]; 2],
    /// Selected line quads in logical screen pixels, clockwise from top-left.
    pub selection: Vec<[[f32; 2]; 4]>,
}

/// Axis of a preview snap guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewAxis {
    Vertical,
    Horizontal,
}

/// A guide line the player draws while a preview gesture holds a snap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewGuide {
    pub axis: PreviewAxis,
    /// Canvas-pixel position: x for a vertical line, y for a horizontal one.
    pub position: f64,
}

/// The chroma-key eyedropper is armed for one item; the next player click
/// samples the rendered preview frame. `effect` is `None` while the Chroma key
/// switch is on but the clip has no effect yet, so the first pick adds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyPick {
    pub(crate) item: project::ItemId,
    pub(crate) effect: Option<String>,
}
/// The matte refinement brush is armed for one clip: the player turns each drag into one stroke
/// of `mode`, until Escape, a selection change or the panel leaves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MatteBrush {
    pub(crate) item: project::ItemId,
    pub(crate) mode: project::MatteStrokeMode,
}
pub(in crate::editor) struct TimecodeEdit {
    pub(in crate::editor) text: String,
    pub(in crate::editor) focus: bool,
}
