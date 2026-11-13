//! Resolves a Media-panel drag over the lanes: the landing hint follows the pointer while it
//! is down, and a release over the lanes inserts the dragged assets.

use crate::editor::context::MediaDrag;
use crate::editor::timeline::{
    Action, Context, DropZone, State, TimelineLayout, TrackRow, egui, theme,
};

/// Where a release would land: the placement request and the hint to paint.
#[derive(Clone, Copy, Debug)]
struct Landing {
    placement: engine::Placement,
    zone: Option<DropZone>,
}

impl State {
    /// Follow a Media drag. While the pointer is down it paints the landing hint and a ghost
    /// label. On release over the lanes it asks the frame to insert; a release elsewhere
    /// drops nothing.
    pub(in crate::editor::timeline) fn advance_media_drag(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
    ) {
        let Some(drag) = cx.media_drag.as_ref() else {
            return;
        };
        let (pointer, down) = ui
            .ctx()
            .input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        let landing = pointer
            .filter(|pointer| layout.body.contains(*pointer))
            .and_then(|pointer| self.media_landing(drag, pointer, layout, rows));
        if !down {
            self.drop_zone = None;
            if let (Some(pointer), Some(landing)) = (pointer, landing) {
                let at = self.tick_at(pointer.x, layout.body.left());
                self.push(Action::InsertMedia {
                    assets: drag.assets.clone(),
                    at,
                    placement: landing.placement,
                });
            }
            return;
        }
        self.drop_zone = landing.and_then(|landing| landing.zone);
        if let Some(pointer) = pointer {
            paint_ghost(cx, ui, drag, pointer);
        }
    }

    /// The landing for a pointer over the lanes. A row takes only its own track kind; a gap
    /// or the space below the last row creates a track of the dragged kind.
    fn media_landing(
        &self,
        drag: &MediaDrag,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
    ) -> Option<Landing> {
        match self.drop_destination(pointer, layout, rows) {
            None => Some(Landing {
                placement: engine::Placement::Auto,
                zone: None,
            }),
            Some((track, None)) => {
                let row = rows.iter().find(|row| row.id == track)?;
                (row.kind == drag.kind && !row.locked).then_some(Landing {
                    placement: engine::Placement::Track { track },
                    zone: Some(DropZone::Track(track)),
                })
            }
            Some((track, Some(engine::TrackPlacement::Above { .. }))) => Some(Landing {
                placement: engine::Placement::Above { track },
                zone: Some(DropZone::Insert { track, above: true }),
            }),
            Some((track, Some(engine::TrackPlacement::Below { .. }))) => Some(Landing {
                placement: engine::Placement::Below { track },
                zone: Some(DropZone::Insert {
                    track,
                    above: false,
                }),
            }),
            Some((track, Some(engine::TrackPlacement::Auto))) => Some(Landing {
                placement: engine::Placement::Auto,
                zone: Some(DropZone::Track(track)),
            }),
        }
    }
}

/// The dragged asset's name beside the pointer, on a layer above the panels.
fn paint_ghost(cx: &Context<'_>, ui: &egui::Ui, drag: &MediaDrag, pointer: egui::Pos2) {
    let name = drag
        .assets
        .first()
        .and_then(|id| {
            cx.session
                .project()
                .data()
                .assets
                .iter()
                .find(|asset| asset.id() == *id)
        })
        .and_then(|asset| asset.path().file_name())
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let label = if drag.assets.len() > 1 {
        format!("{name} +{}", drag.assets.len() - 1)
    } else {
        name
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("media.drag.ghost"),
    ));
    painter.text(
        pointer + egui::vec2(12.0, 12.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::proportional(theme::type_size::CHROME),
        theme::text(cx.dark),
    );
}
