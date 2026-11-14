//! Allocates editor panels around draggable gutters.

use crate::{
    egui::{self, Rect, Vec2},
    kit,
};

pub(super) struct Layout {
    pub(super) top: Rect,
    pub(super) body: Rect,
    pub(super) upper: Rect,
    pub(super) left: Rect,
    pub(super) player: Rect,
    pub(super) right: Rect,
    pub(super) timeline: Rect,
    /// The agent chat docked right of the timeline, when it is open. The timeline gives it
    /// the width and keeps the rest; the chat never covers the timeline.
    pub(super) agent: Option<Rect>,
}

impl Layout {
    pub(super) fn new(rect: Rect, panels: [f32; 3], agent: bool) -> Self {
        let top = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), rect.height().min(kit::size::TOPBAR)),
        );
        let left = (kit::space::MD + kit::space::HAIRLINE).min(rect.width());
        let right = kit::space::MD.min((rect.width() - left).max(0.0));
        let bottom = kit::space::MD.min((rect.bottom() - top.bottom()).max(0.0));
        let body = Rect::from_min_max(
            egui::pos2(rect.left() + left, top.bottom()),
            egui::pos2(rect.right() - right, rect.bottom() - bottom),
        );
        let row_gap = kit::space::SM.min(body.height());
        let row_height = (body.height() - row_gap).max(0.0);
        let timeline_height = (row_height * panels[2]).round();
        let upper = Rect::from_min_max(
            body.min,
            egui::pos2(body.right(), body.bottom() - timeline_height - row_gap),
        );
        let timeline =
            Rect::from_min_max(egui::pos2(body.left(), upper.bottom() + row_gap), body.max);
        let (timeline, agent) = if agent {
            let width = kit::size::AGENT_PANEL.min(timeline.width() / 2.0).max(0.0);
            let dock = Rect::from_min_max(
                egui::pos2(timeline.right() - width, timeline.top()),
                timeline.max,
            );
            let rest = Rect::from_min_max(
                timeline.min,
                egui::pos2(
                    (dock.left() - row_gap).max(timeline.left()),
                    timeline.bottom(),
                ),
            );
            (rest, Some(dock))
        } else {
            (timeline, None)
        };
        let gutter = kit::space::PANEL_GUTTER.min(body.width() / 2.0);
        let columns = (body.width() - gutter * 2.0).max(0.0);
        let left = Rect::from_min_size(
            upper.min,
            Vec2::new((columns * panels[0]).round(), upper.height()),
        );
        let right = Rect::from_min_max(
            egui::pos2(upper.right() - (columns * panels[1]).round(), upper.top()),
            upper.max,
        );
        let player = Rect::from_min_max(
            egui::pos2(left.right() + gutter, upper.top()),
            egui::pos2(right.left() - gutter, upper.bottom()),
        );
        Self {
            top,
            body,
            upper,
            left,
            player,
            right,
            timeline,
            agent,
        }
    }
}
