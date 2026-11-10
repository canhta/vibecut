//! Projects timeline presentation geometry.
//!
//! A track row is its measured clip height plus one 20 px lane for every
//! expanded keyframe channel. Item views carry the clip rectangle and the lane
//! rectangles below it; nothing here reads or mutates the project.

use crate::editor::timeline::{
    AssetId, BookmarkId, ItemId, LABEL_WIDTH, Locale, RULER_HEIGHT, Rect, SCROLLBAR,
    TOOLBAR_HEIGHT, Text, Tick, TrackId, TrackKind, Vec2, egui, theme,
};
use crate::i18n;

/// Height of one expanded keyframe lane.
pub(super) const LANE_HEIGHT: f32 = 20.0;

#[derive(Clone, Debug)]
pub(super) struct TrackRow {
    pub(super) id: TrackId,
    pub(super) main: bool,
    pub(super) audio: bool,
    pub(super) display_index: u32,
    pub(super) muted: bool,
    pub(super) hidden: bool,
    pub(super) locked: bool,
    pub(super) kind: TrackKind,
    pub(super) name: String,
    pub(super) top: f32,
    pub(super) height: f32,
    /// Height of the clips themselves, before expanded lanes.
    pub(super) base_height: f32,
}
#[derive(Clone, Debug)]
pub(super) struct ItemLane {
    pub(super) path: project::AnimationPath,
    pub(super) component: Option<project::ColorComponent>,
    pub(super) top: f32,
    pub(super) label: String,
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
    /// Hidden visual item; still selectable in the timeline but not rendered.
    pub(super) hidden: bool,
    /// Video source-audio switch; `None` for items without a source track.
    pub(super) source_audio_enabled: Option<bool>,
    /// Whether the item's own audio is muted.
    pub(super) muted: bool,
    /// Base clip volume in dB for audio items, used when the channel is unkeyed.
    pub(super) volume: Option<f32>,
    /// Main-track item at the timeline origin: its start cannot move left.
    pub(super) pinned: bool,
    pub(super) lanes: Vec<ItemLane>,
}
#[derive(Clone, Debug)]
pub(super) struct BookmarkView {
    pub(super) id: BookmarkId,
    pub(super) start: Tick,
    pub(super) duration: Tick,
    pub(super) note: String,
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

struct RawItem {
    row_index: usize,
    id: ItemId,
    kind: TrackKind,
    start: Tick,
    end: Tick,
    locked: bool,
    label: String,
    overlay: Option<[u8; 4]>,
    source_asset: Option<AssetId>,
    hidden: bool,
    source_audio_enabled: Option<bool>,
    muted: bool,
    volume: Option<f32>,
    channels: Vec<ChannelLane>,
    /// Main-track item at the timeline origin: its start cannot move left.
    pinned: bool,
}
struct ChannelLane {
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    label: String,
}

/// One lane per keyed channel; audio always offers its volume lane so a key can
/// be added from the expanded item even before the first key exists.
fn channels(item: &project::Item, kind: TrackKind, locale: Locale) -> Vec<ChannelLane> {
    let mut lanes = Vec::new();
    for (path, param) in item.animations.iter() {
        for (index, channel) in param_channels(param).into_iter().enumerate() {
            if channel.keys().is_empty() {
                continue;
            }
            let component = component_of(param, index);
            lanes.push(ChannelLane {
                path: path.clone(),
                component,
                label: lane_label(path.as_str(), locale).to_owned(),
            });
        }
    }
    if kind == TrackKind::Audio
        && !lanes.iter().any(|lane| {
            lane.path.builtin() == Some(project::BuiltinPath::Volume) && lane.component.is_none()
        })
    {
        lanes.push(ChannelLane {
            path: project::AnimationPath::from(project::BuiltinPath::Volume),
            component: None,
            label: i18n::text(locale, Text::AudioGain).to_owned(),
        });
    }
    lanes
}
fn param_channels(param: &project::Param) -> Vec<&project::Channel> {
    match param {
        project::Param::Value(channel) => vec![channel],
        project::Param::Color(channels) => channels.iter().collect(),
    }
}
fn component_of(param: &project::Param, index: usize) -> Option<project::ColorComponent> {
    matches!(param, project::Param::Color(_)).then(|| {
        use project::ColorComponent as C;
        match index {
            0 => C::R,
            1 => C::G,
            2 => C::B,
            _ => C::A,
        }
    })
}
/// Creator-facing lane labels; extension paths fall back to their last segment.
fn lane_label(path: &str, locale: Locale) -> &str {
    use project::BuiltinPath as P;
    match P::parse(path) {
        Some(P::TransformPositionX) => i18n::text(locale, Text::PositionX),
        Some(P::TransformPositionY) => i18n::text(locale, Text::PositionY),
        Some(P::TransformScaleX) => i18n::text(locale, Text::ScaleX),
        Some(P::TransformScaleY) => i18n::text(locale, Text::ScaleY),
        Some(P::TransformRotate) => i18n::text(locale, Text::Rotate),
        Some(P::Opacity) => i18n::text(locale, Text::Opacity),
        Some(P::Volume) => i18n::text(locale, Text::AudioGain),
        _ => path.rsplit('.').next().unwrap_or(path),
    }
}

use crate::editor::timeline::{
    Context, State, TRACK_GAP, item_label, max_scroll_seconds, track_height,
};

impl State {
    /// Project the scene into rows, clip rectangles and expanded lane rectangles.
    pub(in crate::editor::timeline) fn views(
        &mut self,
        cx: &Context<'_>,
        layout: &TimelineLayout,
    ) -> TimelineViews {
        let data = cx.staged_project().data();
        let mut rows = Vec::new();
        let mut raw_items: Vec<RawItem> = Vec::new();
        let mut offset = 0.0;
        let mut overlay_index = 0u32;
        let mut audio_index = 0u32;
        for (row_index, track) in data.scene().timeline.iter().rev().enumerate() {
            let base_height = track_height(track.kind);
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
            let mut lane_height = 0.0;
            for item in &track.items {
                let Ok(end) = item.end() else {
                    continue;
                };
                let (label, overlay) = item_label(&item.content, cx.locale, cx.dark);
                let volume = match &item.content {
                    project::ItemParams::Audio(params) => Some(params.volume),
                    project::ItemParams::Video(params) => Some(params.volume),
                    _ => None,
                };
                let source_audio_enabled = match &item.content {
                    project::ItemParams::Video(params) => Some(params.source_audio_enabled),
                    _ => None,
                };
                let muted = match &item.content {
                    project::ItemParams::Video(params) => params.muted,
                    project::ItemParams::Audio(params) => params.muted,
                    _ => false,
                };
                let channels = if self.expanded.contains(&item.id) {
                    channels(item, track.kind, cx.locale)
                } else {
                    Vec::new()
                };
                lane_height += LANE_HEIGHT * channels.len() as f32;
                raw_items.push(RawItem {
                    row_index,
                    id: item.id,
                    kind: track.kind,
                    start: item.start,
                    end,
                    locked: track.locked,
                    label,
                    overlay,
                    source_asset: item.source_asset,
                    hidden: item.hidden,
                    source_audio_enabled,
                    muted,
                    volume,
                    channels,
                    pinned: main && item.start == Tick::ZERO,
                });
            }
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
                height: base_height + lane_height,
                base_height,
            });
            offset += base_height + lane_height + TRACK_GAP;
        }
        let duration = cx.staged_project().duration();
        let bookmarks = data
            .scene()
            .bookmarks
            .iter()
            .map(|bookmark| BookmarkView {
                id: bookmark.id,
                start: bookmark.start,
                duration: bookmark.duration,
                note: bookmark.note.as_str().to_owned(),
            })
            .collect();
        let content_height = (offset - TRACK_GAP).max(0.0);
        self.vertical_scroll = self
            .vertical_scroll
            .clamp(0.0, (content_height - layout.body.height()).max(0.0));
        // A stack shorter than the lanes sits centred, as in the captured
        // single-track editor (221643: 152 px above, 148 px below the clip).
        let centring = ((layout.body.height() - content_height) / 2.0).max(0.0);
        for row in &mut rows {
            row.top += centring;
        }
        let mut lane_offsets = vec![0.0_f32; rows.len()];
        let mut items = Vec::new();
        for raw in raw_items {
            let (row_top, base_height) = rows
                .get(raw.row_index)
                .map_or((layout.body.top(), 25.0), |row| (row.top, row.base_height));
            let top = layout.body.top() + row_top - self.vertical_scroll;
            let lane_start = top + base_height;
            let mut lanes = Vec::new();
            for channel in raw.channels {
                let lane_top = lane_start + lane_offsets.get(raw.row_index).copied().unwrap_or(0.0);
                lanes.push(ItemLane {
                    path: channel.path,
                    component: channel.component,
                    top: lane_top,
                    label: channel.label,
                });
                if let Some(offset) = lane_offsets.get_mut(raw.row_index) {
                    *offset += LANE_HEIGHT;
                }
            }
            items.push(ItemView {
                id: raw.id,
                kind: raw.kind,
                start: raw.start,
                end: raw.end,
                top,
                height: base_height,
                locked: raw.locked,
                label: raw.label,
                overlay: raw.overlay,
                source_asset: raw.source_asset,
                hidden: raw.hidden,
                source_audio_enabled: raw.source_audio_enabled,
                muted: raw.muted,
                volume: raw.volume,
                pinned: raw.pinned,
                lanes,
            });
        }
        let min = self.min_zoom(duration);
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
}
