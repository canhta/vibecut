//! Allocates editor panels around draggable gutters.

use crate::{
    egui::{self, Rect, Vec2},
    kit,
};

/// Where the agent chat sits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dock {
    Closed,
    /// Right of the timeline, below the properties panel.
    Docked,
    /// A full-height column at the right; the other areas keep the rest.
    Expanded,
}

pub(super) struct Layout {
    pub(super) top: Rect,
    pub(super) body: Rect,
    pub(super) upper: Rect,
    pub(super) left: Rect,
    pub(super) player: Rect,
    pub(super) right: Rect,
    pub(super) timeline: Rect,
    /// The agent chat, when it is open. Docked, the timeline gives it the width and keeps the
    /// rest; expanded, it takes the whole right column and the media, player, properties and
    /// timeline areas share the width left of it (capture 151706). The chat never covers them.
    pub(super) agent: Option<Rect>,
}

impl Layout {
    pub(super) fn new(rect: Rect, panels: [f32; 3], dock: Dock) -> Self {
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
        let (main, column) = if dock == Dock::Expanded {
            let width = kit::size::AGENT_EXPANDED.min(body.width() / 2.0).max(0.0);
            let column = Rect::from_min_max(egui::pos2(body.right() - width, body.top()), body.max);
            let main = Rect::from_min_max(
                body.min,
                egui::pos2(
                    (column.left() - kit::space::PANEL_GUTTER).max(body.left()),
                    body.bottom(),
                ),
            );
            (main, Some(column))
        } else {
            (body, None)
        };
        let row_gap = kit::space::SM.min(body.height());
        let row_height = (body.height() - row_gap).max(0.0);
        let timeline_height = (row_height * panels[2]).round();
        let upper = Rect::from_min_max(
            main.min,
            egui::pos2(main.right(), main.bottom() - timeline_height - row_gap),
        );
        let timeline =
            Rect::from_min_max(egui::pos2(main.left(), upper.bottom() + row_gap), main.max);
        let (timeline, agent) = if dock == Dock::Docked {
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
            (timeline, column)
        };
        let gutter = kit::space::PANEL_GUTTER.min(main.width() / 2.0);
        let columns = (main.width() - gutter * 2.0).max(0.0);
        let [media, _, properties] = crate::preferences::column_widths(panels, columns);
        let left = Rect::from_min_size(upper.min, Vec2::new(media.round(), upper.height()));
        let right = Rect::from_min_max(
            egui::pos2(upper.right() - properties.round(), upper.top()),
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
