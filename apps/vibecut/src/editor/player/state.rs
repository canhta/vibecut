//! Retains preview-only view state between presented frames.
use crate::{egui, i18n::Text, playback::Frame, preview};
use std::sync::Arc;
use time::Tick;
pub(in crate::editor) struct State {
    pub(in crate::editor) failed: bool,
    pub(in crate::editor) snapshot_failed: bool,
    pub(in crate::editor) view: preview::View,
    pub(in crate::editor) viewport: egui::Vec2,
    pub(in crate::editor) fullscreen: bool,
    pub(in crate::editor) transform: super::transform::State,
    pub(in crate::editor) timecode_edit: Option<TimecodeEdit>,
    pub(in crate::editor) timecode_error: Option<Text>,
    pub(in crate::editor) texture: Option<egui::TextureHandle>,
    pub(in crate::editor) texture_key: Option<(u64, Tick)>,
    pub(in crate::editor) texture_owner: Option<Arc<Frame>>,
}
impl State {
    pub(in crate::editor) fn new(fit: bool, zoom: f64) -> Self {
        Self {
            failed: false,
            snapshot_failed: false,
            view: preview::View::new(fit, zoom),
            viewport: egui::vec2(640.0, 360.0),
            fullscreen: false,
            transform: super::transform::State::default(),
            timecode_edit: None,
            timecode_error: None,
            texture: None,
            texture_key: None,
            texture_owner: None,
        }
    }
}
pub(in crate::editor) struct TimecodeEdit {
    pub(super) text: String,
    pub(super) focus: bool,
}
