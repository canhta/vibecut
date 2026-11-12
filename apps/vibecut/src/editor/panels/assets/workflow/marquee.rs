//! Applies media marquee selection gestures.
use super::super::state::MediaMarquee;
use super::super::{AssetId, Rect, egui};
use super::State;
use crate::editor::Desk;

impl State {
    pub(in crate::editor) fn media_marquee(
        &mut self,
        desk: &mut Desk<'_>,
        ui: &mut egui::Ui,
        content: Rect,
        background: &egui::Response,
        rects: &[(AssetId, Rect)],
    ) {
        let modifiers = ui.ctx().input(|input| input.modifiers);
        if background.drag_started()
            && self.media_marquee.is_none()
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
            && !rects.iter().any(|(_, rect)| rect.contains(origin))
        {
            self.media_marquee = Some(MediaMarquee {
                origin,
                before: self.media_selection.clone(),
                additive: modifiers.command || modifiers.ctrl || modifiers.shift,
                moved: false,
            });
        }
        if let Some(marquee) = self.media_marquee.clone()
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(marquee.origin, pointer).intersect(content);
            let mut selected =
                crate::selection::intersections(rect.min, rect.max, rects.iter().copied());
            if marquee.additive {
                selected.extend(&marquee.before);
            }
            self.media_selection = selected;
            if let Some(state) = &mut self.media_marquee {
                state.moved |= marquee.origin.distance(pointer) > 4.0;
            }
            let accent = desk.editor().accent();
            ui.painter().with_clip_rect(content).rect_filled(
                rect,
                0.0,
                accent.gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(content).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, accent),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped()
            && let Some(marquee) = self.media_marquee.take()
        {
            if !marquee.moved {
                self.media_selection.clear();
                self.media_anchor = None;
            }
            desk.end_merge();
        }
        if background.clicked() {
            self.media_selection.clear();
            self.media_anchor = None;
            desk.end_merge();
        }
    }
    /// Escape: abandon an open marquee and the selection it was building.
    pub(in crate::editor) fn cancel_media_marquee(&mut self) -> bool {
        if self.media_marquee.take().is_none() {
            return false;
        }
        self.media_selection.clear();
        self.media_anchor = None;
        true
    }
    pub(in crate::editor) fn clear_media_selection(&mut self) {
        self.media_selection.clear();
        self.media_anchor = None;
    }
}
