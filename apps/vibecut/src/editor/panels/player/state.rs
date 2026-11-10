//! Retains preview-only view state between presented frames.
use super::{crop, preview};
use crate::{egui, i18n::Text, playback::Frame};
use std::sync::Arc;
use time::Tick;
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
    pub(in crate::editor) timecode_edit: Option<TimecodeEdit>,
    pub(in crate::editor) timecode_error: Option<Text>,
    pub(in crate::editor) texture: Option<egui::TextureHandle>,
    pub(in crate::editor) texture_key: Option<(u64, Tick)>,
    pub(in crate::editor) texture_owner: Option<Arc<Frame>>,
    /// Armed chroma-key eyedropper, cleared by its pick, Escape or a selection change.
    pub(in crate::editor) key_color_pick: Option<KeyPick>,
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
            timecode_edit: None,
            timecode_error: None,
            texture: None,
            texture_key: None,
            texture_owner: None,
            key_color_pick: None,
        }
    }
}
/// The chroma-key eyedropper is armed for one item; the next player click
/// samples the rendered preview frame. `effect` is `None` while the Chroma key
/// switch is on but the clip has no effect yet, so the first pick adds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyPick {
    pub(crate) item: project::ItemId,
    pub(crate) effect: Option<String>,
}
pub(in crate::editor) struct TimecodeEdit {
    pub(super) text: String,
    pub(super) focus: bool,
}
