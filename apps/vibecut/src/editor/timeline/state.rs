//! Projects timeline presentation geometry.

use crate::editor::timeline::{
    AssetId, BookmarkId, ItemId, LABEL_WIDTH, RULER_HEIGHT, Rect, SCROLLBAR, TOOLBAR_HEIGHT, Tick,
    TrackId, TrackKind, Vec2, egui, theme,
};

#[derive(Clone, Debug)]
pub(super) struct TrackRow {
    pub(super) id: TrackId,
    pub(super) main: bool,
    pub(super) audio: bool,
    pub(super) display_index: usize,
    pub(super) muted: bool,
    pub(super) hidden: bool,
    pub(super) locked: bool,
    pub(super) kind: TrackKind,
    pub(super) name: String,
    pub(super) top: f32,
    pub(super) height: f32,
}
#[derive(Clone, Debug)]
pub(super) struct ItemView {
    pub(super) id: ItemId,
    pub(super) kind: TrackKind,
    pub(super) start: Tick,
    pub(super) end: Tick,
    pub(super) top: f32,
    pub(super) height: f32,
    pub(super) locked: bool,
    pub(super) label: String,
    pub(super) overlay: Option<[u8; 4]>,
    pub(super) source_asset: Option<AssetId>,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct BookmarkView {
    pub(super) id: BookmarkId,
    pub(super) start: Tick,
}
pub(super) struct TimelineViews {
    pub(super) rows: Vec<TrackRow>,
    pub(super) items: Vec<ItemView>,
    pub(super) duration: Tick,
    pub(super) bookmarks: Vec<BookmarkView>,
}
pub(super) struct TimelineLayout {
    pub(super) toolbar: Rect,
    pub(super) ruler: Rect,
    pub(super) body: Rect,
    pub(super) labels: Rect,
    pub(super) horizontal: Rect,
    pub(super) vertical: Rect,
}
impl TimelineLayout {
    pub(in crate::editor) fn new(rect: Rect) -> Self {
        let toolbar = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), TOOLBAR_HEIGHT.min(rect.height())),
        );
        // A 1 px rule closes the toolbar before the ruler starts.
        let top = (toolbar.bottom() + 1.0).min(rect.bottom());
        let label_right = (rect.left() + LABEL_WIDTH).min(rect.right());
        let ruler_bottom = (top + RULER_HEIGHT).min(rect.bottom());
        let body_top = (ruler_bottom + theme::size::RULER_GAP).min(rect.bottom());
        let right = (rect.right() - SCROLLBAR).max(label_right);
        let bottom = (rect.bottom() - theme::size::TIMELINE_FOOT).max(body_top);
        let ruler = Rect::from_min_max(
            egui::pos2(label_right, top),
            egui::pos2(right, ruler_bottom),
        );
        let body = Rect::from_min_max(egui::pos2(label_right, body_top), egui::pos2(right, bottom));
        let labels = Rect::from_min_max(
            egui::pos2(rect.left(), body_top),
            egui::pos2(label_right, bottom),
        );
        // The 6 px bar sits 4 px under the lanes inside the foot strip.
        let bar_top = (bottom + 4.0).min(rect.bottom());
        let horizontal = Rect::from_min_max(
            egui::pos2(label_right, bar_top),
            egui::pos2(right, (bar_top + SCROLLBAR).min(rect.bottom())),
        );
        let vertical = Rect::from_min_max(
            egui::pos2(right, body_top),
            egui::pos2(rect.right(), bottom),
        );
        Self {
            toolbar,
            ruler,
            body,
            labels,
            horizontal,
            vertical,
        }
    }
}

use super::{Editor, Rounding, TRACK_GAP, item_label, track_height};

impl Editor {
    pub(in crate::editor::timeline) fn timeline_views(
        &mut self,
        layout: &TimelineLayout,
    ) -> TimelineViews {
        let (mut rows, raw_items, duration, bookmarks, content_height) = {
            let data = self.session.project().data();
            let mut rows = Vec::new();
            let mut raw_items = Vec::new();
            let mut offset = 0.0;
            let mut overlay_index = 0usize;
            let mut audio_index = 0usize;
            for (row_index, track) in data.scene().timeline.iter().rev().enumerate() {
                let height = track_height(track.kind);
                let main = track.id == data.scene().timeline.main.id;
                let audio = track.kind == TrackKind::Audio;
                let display_index = if main {
                    0
                } else if audio {
                    let index = audio_index;
                    audio_index += 1;
                    index
                } else {
                    let index = overlay_index;
                    overlay_index += 1;
                    index
                };
                rows.push(TrackRow {
                    id: track.id,
                    main,
                    audio,
                    display_index,
                    muted: track.muted,
                    hidden: track.hidden,
                    locked: track.locked,
                    kind: track.kind,
                    name: track.name.as_str().to_owned(),
                    top: offset,
                    height,
                });
                for item in &track.items {
                    let Ok(end) = item.end() else {
                        continue;
                    };
                    let (label, overlay) = item_label(
                        &item.content,
                        self.preferences.locale,
                        self.preferences.dark,
                    );
                    raw_items.push((
                        row_index,
                        item.id,
                        track.kind,
                        item.start,
                        end,
                        track.locked,
                        label,
                        overlay,
                        item.source_asset,
                    ));
                }
                offset += height + TRACK_GAP;
            }
            (
                rows,
                raw_items,
                self.session.project().duration(),
                data.scene()
                    .bookmarks
                    .iter()
                    .map(|bookmark| BookmarkView {
                        id: bookmark.id,
                        start: bookmark.start,
                    })
                    .collect(),
                (offset - TRACK_GAP).max(0.0),
            )
        };
        self.vertical_scroll = self
            .vertical_scroll
            .clamp(0.0, (content_height - layout.body.height()).max(0.0));
        // A stack shorter than the lanes sits centred, as in the captured
        // single-track editor (221643: 152 px above, 148 px below the clip).
        let centring = ((layout.body.height() - content_height) / 2.0).max(0.0);
        for row in &mut rows {
            row.top += centring;
        }
        let mut items = Vec::new();
        for (row_index, id, kind, start, end, locked, label, overlay, source_asset) in raw_items {
            let (top, height) = rows
                .get(row_index)
                .map_or((layout.body.top(), 25.0), |row| {
                    (
                        layout.body.top() + row.top - self.vertical_scroll,
                        row.height,
                    )
                });
            items.push(ItemView {
                id,
                kind,
                start,
                end,
                top,
                height,
                locked,
                label,
                overlay,
                source_asset,
            });
        }
        let min = self.min_zoom(layout.body.width());
        let max_scroll =
            max_scroll_seconds(duration.seconds(), layout.body.width(), self.zoom, min);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        TimelineViews {
            rows,
            items,
            duration,
            bookmarks,
        }
    }
    pub(in crate::editor) fn tick_at(&self, x: f32, left: f32) -> Tick {
        Tick::from_seconds_f64(
            f64::from(self.scroll + (x - left) / self.zoom),
            Rounding::Nearest,
        )
        .unwrap_or(Tick::ZERO)
    }
    pub(in crate::editor) fn x_at(&self, at: Tick, left: f32) -> f32 {
        left + (at.seconds() as f32 - self.scroll) * self.zoom
    }
}

use super::max_scroll_seconds;
