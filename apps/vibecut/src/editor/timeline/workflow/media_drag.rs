//! Resolves a Media-panel drag over the lanes: the landing hint follows the pointer while it
//! is down, and a release over the lanes inserts the dragged assets. A single asset over an
//! occupied clip of its kind labels the clip `Replace`; releasing it shorter than that clip
//! changes nothing and toasts why.

use crate::editor::context::MediaDrag;
use crate::editor::timeline::{
    Action, Context, DropZone, ItemId, ItemView, State, Text, TimelineLayout, TrackRow, egui,
    theme,
};
use crate::i18n;

/// An occupied clip under a Media drag, and whether the dragged media is shorter than it.
#[derive(Clone, Copy, Debug)]
struct Occupied {
    item: ItemId,
    shorter: bool,
}

/// Where a release would land: the placement request and the hint to paint.
#[derive(Clone, Copy, Debug)]
struct Landing {
    placement: engine::Placement,
    zone: Option<DropZone>,
    occupied: Option<Occupied>,
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
        items: &[ItemView],
    ) {
        let Some(drag) = cx.media_drag.as_ref() else {
            self.replace_hint = None;
            return;
        };
        let (pointer, down) = ui
            .ctx()
            .input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        let landing = pointer
            .filter(|pointer| layout.body.contains(*pointer))
            .and_then(|pointer| self.media_landing(cx, drag, pointer, layout, rows, items));
        if !down {
            self.drop_zone = None;
            self.replace_hint = None;
            if let (Some(pointer), Some(landing)) = (pointer, landing) {
                if landing.occupied.is_some_and(|occupied| occupied.shorter) {
                    self.push(Action::Toast(
                        i18n::text(cx.locale, Text::ReplaceTooShort).to_owned(),
                    ));
                    return;
                }
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
        self.replace_hint = landing.and_then(|landing| landing.occupied.map(|clip| clip.item));
        if let Some(pointer) = pointer {
            paint_ghost(cx, ui, drag, pointer);
        }
    }

    /// The landing for a pointer over the lanes. A row takes only its own track kind; a gap
    /// or the space below the last row creates a track of the dragged kind.
    fn media_landing(
        &self,
        cx: &Context<'_>,
        drag: &MediaDrag,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        items: &[ItemView],
    ) -> Option<Landing> {
        match self.drop_destination(pointer, layout, rows) {
            None => Some(Landing {
                placement: engine::Placement::Auto,
                zone: None,
                occupied: None,
            }),
            Some((track, None)) => {
                let row = rows.iter().find(|row| row.id == track)?;
                (row.kind == drag.kind && !row.locked).then(|| Landing {
                    placement: engine::Placement::Track { track },
                    zone: Some(DropZone::Track(track)),
                    occupied: self.occupied_clip(cx, drag, pointer, layout, items),
                })
            }
            Some((track, Some(engine::TrackPlacement::Above { .. }))) => {
                new_track_landing(drag, track, true, rows)
            }
            Some((track, Some(engine::TrackPlacement::Below { .. }))) => {
                new_track_landing(drag, track, false, rows)
            }
            Some((track, Some(engine::TrackPlacement::Auto))) => Some(Landing {
                placement: engine::Placement::Auto,
                zone: Some(DropZone::Track(track)),
                occupied: None,
            }),
        }
    }

    /// The unlocked media clip of the dragged kind under the pointer, for one dragged asset.
    /// `shorter` is set when the asset's known duration is below the clip's length; an asset
    /// of unknown duration (a still) is never shorter.
    fn occupied_clip(
        &self,
        cx: &Context<'_>,
        drag: &MediaDrag,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        items: &[ItemView],
    ) -> Option<Occupied> {
        let [dragged] = drag.assets.as_slice() else {
            return None;
        };
        let at = self.tick_at(pointer.x, layout.body.left());
        let item = items.iter().find(|item| {
            item.kind == drag.kind
                && item.source_asset.is_some()
                && !item.locked
                && item.top <= pointer.y
                && pointer.y < item.top + item.height
                && item.start <= at
                && at < item.end
        })?;
        let duration = cx
            .session
            .project()
            .data()
            .assets
            .iter()
            .find(|asset| asset.id() == *dragged)
            .and_then(|asset| asset.probe().duration);
        Some(Occupied {
            item: item.id,
            shorter: duration.is_some_and(|duration| duration < item.end - item.start),
        })
    }
}

/// A new track of the dragged kind next to `track`. Visual tracks stay above the main track
/// and audio tracks below it, so a gap on the other side of that boundary lands on the
/// boundary itself: directly above the main track for visual media, directly below it for
/// audio.
fn new_track_landing(
    drag: &MediaDrag,
    track: project::TrackId,
    above: bool,
    rows: &[TrackRow],
) -> Option<Landing> {
    let audio = drag.kind == project::TrackKind::Audio;
    let row = rows.iter().find(|row| row.id == track)?;
    let fits = if row.main {
        above != audio
    } else {
        row.audio == audio
    };
    let (track, above) = if fits {
        (track, above)
    } else {
        (rows.iter().find(|row| row.main)?.id, !audio)
    };
    Some(Landing {
        placement: if above {
            engine::Placement::Above { track }
        } else {
            engine::Placement::Below { track }
        },
        zone: Some(DropZone::Insert { track, above }),
        occupied: None,
    })
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
        egui::Id::unique("media.drag.ghost"),
    ));
    painter.text(
        pointer + egui::vec2(12.0, 12.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::proportional(theme::type_size::CHROME),
        theme::text(cx.dark),
    );
}
