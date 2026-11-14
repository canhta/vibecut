//! Allocates editor panels around draggable gutters.

use crate::{
    egui::{self, Rect, Vec2},
    kit,
    preferences::LayoutPreset,
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
    pub(super) left: Rect,
    pub(super) player: Rect,
    pub(super) right: Rect,
    pub(super) timeline: Rect,
    /// The agent chat, when it is open. Docked, the timeline gives it the width and keeps the
    /// rest; expanded, it takes the whole right column and the media, player, properties and
    /// timeline areas share the width left of it (capture 151706). The chat never covers them.
    pub(super) agent: Option<Rect>,
    /// Room the three columns share, for splitter drags.
    pub(super) columns: f32,
    /// Hit areas of the splitters: after the media column, after the middle column and the
    /// timeline row seam. A preset without that seam has none.
    pub(super) splits: [Option<Rect>; 3],
}

impl Layout {
    /// `timeline_hidden` collapses the timeline row to its toolbar and progress line; the
    /// freed height goes to the upper row.
    pub(super) fn new(
        rect: Rect,
        panels: [f32; 3],
        dock: Dock,
        timeline_hidden: bool,
        preset: LayoutPreset,
    ) -> Self {
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
        let timeline_height = if timeline_hidden {
            crate::editor::timeline::HIDDEN_HEIGHT.min(row_height)
        } else {
            (row_height * panels[2]).round()
        };
        let upper = Rect::from_min_max(
            main.min,
            egui::pos2(main.right(), main.bottom() - timeline_height - row_gap),
        );
        let timeline =
            Rect::from_min_max(egui::pos2(main.left(), upper.bottom() + row_gap), main.max);
        let gutter = kit::space::PANEL_GUTTER.min(main.width() / 2.0);
        let columns = (main.width() - gutter * 2.0).max(0.0);
        let [first, _, last] = crate::preferences::column_widths(panels, columns);
        let c0 = Rect::from_min_size(upper.min, Vec2::new(first.round(), upper.height()));
        let c2 = Rect::from_min_max(
            egui::pos2(upper.right() - last.round(), upper.top()),
            upper.max,
        );
        let c1 = Rect::from_min_max(
            egui::pos2(c0.right() + gutter, upper.top()),
            egui::pos2(c2.left() - gutter, upper.bottom()),
        );
        let full = |column: Rect| {
            Rect::from_min_max(column.min, egui::pos2(column.right(), main.bottom()))
        };
        let ends_at = |column: Rect| {
            Rect::from_min_max(timeline.min, egui::pos2(column.right(), timeline.bottom()))
        };
        // (media, player, properties, timeline, seam between columns 0 and 1, and 1 and 2)
        let (left, player, right, timeline, seams) = match preset {
            LayoutPreset::Default | LayoutPreset::Media => (c0, c1, c2, timeline, (upper, upper)),
            LayoutPreset::Attributes => (c0, c1, full(c2), ends_at(c1), (upper, main)),
            LayoutPreset::Vertical => (c0, full(c2), c1, ends_at(c1), (upper, main)),
            LayoutPreset::Generative => {
                let player = Rect::from_min_max(c1.min, upper.max);
                let bottom =
                    Rect::from_min_max(timeline.min, egui::pos2(c0.right(), timeline.bottom()));
                let rest = Rect::from_min_max(egui::pos2(c1.left(), timeline.top()), timeline.max);
                (c0, player, bottom, rest, (main, Rect::NOTHING))
            }
        };
        let seam = |span: Rect, left: f32, right: f32| {
            (span.width() > 0.0).then(|| {
                Rect::from_center_size(
                    egui::pos2((left + right) / 2.0, span.center().y),
                    Vec2::new(gutter, span.height()),
                )
            })
        };
        let splits = [
            seam(seams.0, c0.right(), c1.left()),
            if preset == LayoutPreset::Generative {
                None
            } else {
                seam(seams.1, c1.right(), c2.left())
            },
            Some(Rect::from_center_size(
                egui::pos2(
                    (if preset == LayoutPreset::Generative {
                        main.left()
                    } else {
                        timeline.left()
                    } + timeline.right())
                        / 2.0,
                    (upper.bottom() + timeline.top()) / 2.0,
                ),
                Vec2::new(
                    timeline.right()
                        - if preset == LayoutPreset::Generative {
                            main.left()
                        } else {
                            timeline.left()
                        },
                    row_gap,
                ),
            )),
        ];
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
        Self {
            top,
            body,
            left,
            player,
            right,
            timeline,
            agent,
            columns,
            splits,
        }
    }
}
