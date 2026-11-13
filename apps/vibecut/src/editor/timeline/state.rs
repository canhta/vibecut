//! Holds the timeline's own presentation state and viewport arithmetic.

use crate::editor::timeline::graph::GraphPopover;
use crate::editor::timeline::{
    Action, BTreeSet, BookmarkId, Context, DEFAULT_ZOOM, ItemId, KeyframeSelection, MAX_ZOOM,
    MIN_ZOOM_FLOOR, Media, Rect, Rounding, SourceRange, Tick, TrackId, ZOOM_STEP, egui,
    workflow::{anchored_scroll, fit_zoom},
};
use crate::services::color_picker::Picker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor::timeline) enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}
#[derive(Clone, Copy, Debug)]
pub(in crate::editor::timeline) struct DragOrigin {
    pub(in crate::editor::timeline) item: ItemId,
    pub(in crate::editor::timeline) track: TrackId,
    pub(in crate::editor::timeline) start: Tick,
    pub(in crate::editor::timeline) end: Tick,
    /// Source window and rate: trim bounds read the member's own headroom.
    pub(in crate::editor::timeline) source_range: SourceRange,
    pub(in crate::editor::timeline) speed: time::Speed,
}
#[derive(Clone, Debug)]
pub(in crate::editor::timeline) struct DragState {
    pub(in crate::editor::timeline) item: ItemId,
    pub(in crate::editor::timeline) mode: DragMode,
    pub(in crate::editor::timeline) press_x: f32,
    /// Last snapped delta staged for this gesture.
    pub(in crate::editor::timeline) delta: Tick,
    pub(in crate::editor::timeline) origins: Vec<DragOrigin>,
    pub(in crate::editor::timeline) source_range: SourceRange,
    pub(in crate::editor::timeline) speed: time::Speed,
}

/// Where a clip drop would land: an existing row or a new track between rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum DropZone {
    Track(TrackId),
    Insert { track: TrackId, above: bool },
}
#[derive(Clone, Copy, Debug)]
pub(in crate::editor::timeline) struct BookmarkDrag {
    pub(in crate::editor::timeline) bookmark: BookmarkId,
    pub(in crate::editor::timeline) press_x: f32,
    pub(in crate::editor::timeline) start: Tick,
}

/// The marker editor opened from a bookmark on the ruler.
pub(in crate::editor::timeline) struct BookmarkPopover {
    pub(in crate::editor::timeline) bookmark: BookmarkId,
    /// Window top-left in screen coordinates.
    pub(in crate::editor::timeline) anchor: egui::Pos2,
    /// Typed note not yet committed; `None` reads the project value.
    pub(in crate::editor::timeline) note: Option<String>,
    /// Typed duration in seconds not yet committed; `None` reads the project value.
    pub(in crate::editor::timeline) duration: Option<String>,
    /// Project values at the last draw; a draft that differs commits on blur.
    pub(in crate::editor::timeline) note_base: String,
    pub(in crate::editor::timeline) duration_base: Tick,
    pub(in crate::editor::timeline) picker: Option<Picker>,
}

/// One keyframe retime gesture over the pressed key.
#[derive(Clone, Debug)]
pub(in crate::editor::timeline) struct KeyframeDrag {
    pub(in crate::editor::timeline) selection: KeyframeSelection,
    /// Time currently held by the preview project.
    pub(in crate::editor::timeline) current: Tick,
    pub(in crate::editor::timeline) item_start: Tick,
    pub(in crate::editor::timeline) item_end: Tick,
}

/// A marquee inside expanded keyframe lanes.
#[derive(Clone, Debug)]
pub(in crate::editor::timeline) struct KeyframeMarquee {
    pub(in crate::editor::timeline) origin: egui::Pos2,
    pub(in crate::editor::timeline) before: BTreeSet<KeyframeSelection>,
    pub(in crate::editor::timeline) additive: bool,
}

/// A volume-line drag: one time point, one 0.1 dB value.
#[derive(Clone, Copy, Debug)]
pub(in crate::editor::timeline) struct VolumeDrag {
    pub(in crate::editor::timeline) item: ItemId,
    /// Item-relative key time the drag writes.
    pub(in crate::editor::timeline) time: Tick,
    /// The channel already had keys, so the drag writes a key instead of the base.
    pub(in crate::editor::timeline) keyed: bool,
    pub(in crate::editor::timeline) db: f32,
    pub(in crate::editor::timeline) top: f32,
    pub(in crate::editor::timeline) height: f32,
}

/// A fade-handle drag: one edge of one clip, measured from the press point.
#[derive(Clone, Copy, Debug)]
pub(in crate::editor::timeline) struct FadeDrag {
    pub(in crate::editor::timeline) item: ItemId,
    pub(in crate::editor::timeline) edge: project::FadeEdge,
    /// Curve the edge keeps while its duration changes.
    pub(in crate::editor::timeline) curve: project::FadeCurve,
    /// Duration the handle was grabbed with, so a press alone changes nothing.
    pub(in crate::editor::timeline) base: Tick,
    pub(in crate::editor::timeline) press_x: f32,
    /// Longest fade the clip and the other edge admit.
    pub(in crate::editor::timeline) room: Tick,
    /// Duration last staged by this gesture.
    pub(in crate::editor::timeline) duration: Tick,
}

/// A control the panel painted this frame, recorded for automation by the frame.
pub(in crate::editor) struct Remembered {
    pub(in crate::editor) id: String,
    pub(in crate::editor) icon: Option<&'static str>,
    pub(in crate::editor) tooltip: Option<String>,
    pub(in crate::editor) response: egui::Response,
}

/// The scene sheet's transient state.
#[derive(Default)]
pub(in crate::editor) struct Scenes {
    pub(in crate::editor::timeline) open: bool,
    pub(in crate::editor::timeline) select: bool,
    pub(in crate::editor::timeline) selected: BTreeSet<project::SceneId>,
    pub(in crate::editor::timeline) confirm: bool,
    pub(in crate::editor::timeline) rename: Option<(project::SceneId, String)>,
    pub(in crate::editor::timeline) rename_focus: bool,
}
impl Scenes {
    pub(in crate::editor) fn is_open(&self) -> bool {
        self.open
    }
    pub(in crate::editor) fn is_selecting(&self) -> bool {
        self.select
    }
    pub(in crate::editor) fn is_confirming(&self) -> bool {
        self.confirm
    }
    pub(in crate::editor) fn is_renaming(&self) -> bool {
        self.rename.is_some()
    }
    pub(in crate::editor) fn selected(&self) -> &BTreeSet<project::SceneId> {
        &self.selected
    }
    pub(in crate::editor) fn open(&mut self) {
        self.open = true;
    }
    pub(in crate::editor) fn close(&mut self) {
        self.open = false;
        self.confirm = false;
        self.rename = None;
    }
    pub(in crate::editor) fn toggle_selecting(&mut self) {
        self.select = !self.select;
        self.selected.clear();
    }
    pub(in crate::editor) fn request_delete(&mut self) {
        self.confirm = !self.selected.is_empty();
    }
    pub(in crate::editor) fn cancel_delete(&mut self) {
        self.confirm = false;
    }
    pub(in crate::editor) fn begin_rename(&mut self, scene: project::SceneId, name: String) {
        self.rename = Some((scene, name));
        self.rename_focus = true;
    }
    pub(in crate::editor) fn end_rename(&mut self) {
        self.rename = None;
    }
    pub(in crate::editor) fn toggle(&mut self, scene: project::SceneId, is_main: bool) {
        if !is_main && !self.selected.remove(&scene) {
            self.selected.insert(scene);
        }
    }
    pub(in crate::editor) fn clear_selection(&mut self) {
        self.selected.clear();
        self.select = false;
        self.confirm = false;
    }
    /// Keyboard Escape: the confirmation first, then the sheet itself.
    pub(in crate::editor) fn escape(&mut self) {
        if self.confirm {
            self.confirm = false;
        } else {
            self.open = false;
        }
    }
}

/// Everything the timeline panel remembers between frames.
pub(in crate::editor) struct State {
    pub(in crate::editor::timeline) zoom: f32,
    pub(in crate::editor::timeline) scroll: f32,
    pub(in crate::editor::timeline) vertical_scroll: f32,
    /// Content viewport width used for the dynamic fit minimum.
    pub(in crate::editor::timeline) viewport: f32,
    /// Pixel scroll captured when zoom crossed the playhead-anchor threshold.
    pub(in crate::editor::timeline) pre_anchor_scroll: Option<f32>,
    pub(in crate::editor::timeline) drag: Option<DragState>,
    /// Highlighted landing row or insertion line while a move drag is over the lanes.
    pub(in crate::editor::timeline) drop_zone: Option<DropZone>,
    /// Command of the candidate staged by the group preview contract.
    pub(in crate::editor::timeline) preview: Option<engine::CommandKind>,
    /// The last staged command leaves every member at its entry position.
    pub(in crate::editor::timeline) preview_noop: bool,
    /// The gesture armed the preview contract and may stage a candidate.
    pub(in crate::editor::timeline) preview_armed: bool,
    /// Latest refusal from a staged move/trim, painted until the next gesture.
    pub(in crate::editor::timeline) refusal: Option<String>,
    pub(in crate::editor::timeline) bookmark_drag: Option<BookmarkDrag>,
    pub(in crate::editor::timeline) bookmark_popover: Option<BookmarkPopover>,
    /// The keyframe graph popover; its edits live in one engine gesture.
    pub(in crate::editor::timeline) graph_popover: Option<GraphPopover>,
    pub(in crate::editor::timeline) snap_indicator: Option<Tick>,
    /// Preview-axis pointer time; transient and never part of the project.
    pub(in crate::editor::timeline) hover: Option<Tick>,
    pub(in crate::editor::timeline) marquee: Option<(egui::Pos2, BTreeSet<ItemId>, bool)>,
    pub(in crate::editor::timeline) track_rename: Option<(TrackId, String)>,
    pub(in crate::editor::timeline) track_rename_focus: bool,
    pub(in crate::editor::timeline) track_drag: Option<TrackId>,
    /// Items whose keyframe lanes are expanded; persisted in preferences.
    pub(in crate::editor::timeline) expanded: BTreeSet<ItemId>,
    pub(in crate::editor::timeline) keyframe_drag: Option<KeyframeDrag>,
    pub(in crate::editor::timeline) keyframe_marquee: Option<KeyframeMarquee>,
    pub(in crate::editor::timeline) volume_drag: Option<VolumeDrag>,
    pub(in crate::editor::timeline) fade_drag: Option<FadeDrag>,
    pub(in crate::editor) scenes: Scenes,
    /// Screen rectangles of the visible clips, published for drops from other areas.
    pub(in crate::editor::timeline) item_rects: Vec<(ItemId, Rect)>,
    /// Screen rectangles of the visible keyframe diamonds, for lane marquees.
    pub(in crate::editor::timeline) keyframe_rects: Vec<(KeyframeSelection, Rect)>,
    pub(in crate::editor::timeline) outbox: Vec<Action>,
    pub(in crate::editor::timeline) widgets: Vec<Remembered>,
    /// Cache-only clip visuals and the generation jobs they request.
    media: Media,
}
impl Default for State {
    fn default() -> Self {
        Self::new(DEFAULT_ZOOM, 0.0)
    }
}
impl State {
    pub(in crate::editor) fn new(zoom: f32, scroll: f32) -> Self {
        Self {
            zoom,
            scroll,
            vertical_scroll: 0.0,
            viewport: 1000.0,
            pre_anchor_scroll: None,
            drag: None,
            drop_zone: None,
            preview: None,
            preview_noop: false,
            preview_armed: false,
            refusal: None,
            bookmark_drag: None,
            bookmark_popover: None,
            graph_popover: None,
            snap_indicator: None,
            hover: None,
            marquee: None,
            track_rename: None,
            track_rename_focus: false,
            track_drag: None,
            expanded: BTreeSet::new(),
            keyframe_drag: None,
            keyframe_marquee: None,
            volume_drag: None,
            fade_drag: None,
            scenes: Scenes::default(),
            item_rects: Vec::new(),
            keyframe_rects: Vec::new(),
            outbox: Vec::new(),
            widgets: Vec::new(),
            media: Media::default(),
        }
    }
    pub(in crate::editor) fn zoom(&self) -> f32 {
        self.zoom
    }
    pub(in crate::editor) fn scroll(&self) -> f32 {
        self.scroll
    }
    pub(in crate::editor) fn snap_indicator(&self) -> Option<Tick> {
        self.snap_indicator
    }
    /// The pointer time the preview axis currently shows, if any.
    pub(in crate::editor) fn hover_time(&self) -> Option<Tick> {
        self.hover
    }
    pub(in crate::editor) fn clear_hover(&mut self) {
        self.hover = None;
    }
    /// The graph popover is open.
    pub(in crate::editor) fn graph_popover_open(&self) -> bool {
        self.graph_popover.is_some()
    }
    /// Escape and Cancel: close the graph popover and discard its staged previews.
    pub(in crate::editor) fn escape_graph(&mut self) -> bool {
        let Some(open) = self.graph_popover.take() else {
            return false;
        };
        if open.staged {
            self.outbox.push(Action::DiscardGesture);
        }
        true
    }
    /// The bookmark whose editor is open, if any.
    pub(in crate::editor) fn bookmark_popover(&self) -> Option<BookmarkId> {
        self.bookmark_popover
            .as_ref()
            .map(|popover| popover.bookmark)
    }
    /// Close the editor, committing a typed note or duration as one update.
    pub(in crate::editor) fn close_bookmark_popover(&mut self) {
        self.commit_bookmark_drafts();
        self.bookmark_popover = None;
    }
    /// Commit the drafts still in the editor; Escape bypasses this to discard.
    fn commit_bookmark_drafts(&mut self) {
        let Some(open) = self.bookmark_popover.as_ref() else {
            return;
        };
        let note = open
            .note
            .as_ref()
            .filter(|note| **note != open.note_base)
            .map(|note| engine::NfcText::from(note.clone()));
        let duration = open
            .duration
            .as_deref()
            .and_then(duration_tick)
            .filter(|tick| *tick != open.duration_base);
        if note.is_none() && duration.is_none() {
            return;
        }
        self.outbox
            .push(Action::Command(Box::new(engine::Command::UpdateBookmark {
                bookmark: open.bookmark,
                note,
                color: None,
                duration,
            })));
    }
    /// Escape closes the colour window before the marker editor.
    pub(in crate::editor) fn escape_bookmark(&mut self) -> bool {
        let Some(popover) = self.bookmark_popover.as_mut() else {
            return false;
        };
        if popover.picker.take().is_some() {
            return true;
        }
        self.bookmark_popover = None;
        true
    }
    pub(in crate::editor) fn item_rects(&self) -> &[(ItemId, Rect)] {
        &self.item_rects
    }
    pub(in crate::editor) fn marquee_active(&self) -> bool {
        self.marquee.is_some()
    }
    pub(in crate::editor) fn renaming_track(&self) -> bool {
        self.track_rename.is_some()
    }
    /// Take the controls painted since the last call.
    pub(in crate::editor) fn take_widgets(&mut self) -> Vec<Remembered> {
        std::mem::take(&mut self.widgets)
    }
    /// Adopt a persisted or agent-issued view (zoom in px/s, offset in seconds).
    pub(in crate::editor) fn restore_view(&mut self, zoom: f32, scroll: f32) {
        self.pre_anchor_scroll = None;
        self.zoom = zoom;
        self.scroll = scroll;
    }
    /// Adopt the workspace's one media cache instance for clip visuals and
    /// their generation jobs.
    pub(in crate::editor) fn set_media(
        &mut self,
        access: std::sync::Arc<std::sync::Mutex<engine::control::MediaAccess>>,
    ) {
        self.media.set_access(access);
    }
    /// Submit one generation command the painted clips requested.
    pub(in crate::editor) fn generate(
        &mut self,
        session: &mut engine::Session,
        request: crate::editor::context::visuals::VisualRequest,
        budget: &engine::ByteBudget,
    ) -> crate::Result<()> {
        self.media.generate(session, request, budget)
    }
    /// Advance the generation jobs and the cache maintenance worker.
    pub(in crate::editor) fn tick_media(
        &mut self,
        session: &mut engine::Session,
    ) -> crate::Result<()> {
        self.media.tick(session)
    }
    pub(in crate::editor) fn set_proxies(&mut self, index: Option<engine::proxies::ProxyIndex>) {
        self.media.set_proxies(index);
    }
    pub(in crate::editor) fn request_proxy(
        &mut self,
        action: crate::editor::context::ProxyAction,
        assets: &[media::SourceAsset],
        budget: &engine::ByteBudget,
    ) {
        self.media.request_proxy(action, assets, budget);
    }
    pub(in crate::editor) fn request_cache(&mut self, action: crate::editor::context::CacheAction) {
        self.media.request_cache(action);
    }
    pub(in crate::editor) fn cache_ui(&self) -> crate::editor::context::CacheUi {
        self.media.cache_ui()
    }
    /// The display queries the context lends to every area.
    pub(in crate::editor) fn visuals(&self) -> &dyn crate::editor::context::VisualQuery {
        &self.media
    }
    /// Lend the media service out for one paint: the context borrows it while
    /// the painted state is borrowed mutably.
    pub(in crate::editor) fn take_media(&mut self) -> Media {
        std::mem::take(&mut self.media)
    }
    pub(in crate::editor) fn restore_media(&mut self, media: Media) {
        self.media = media;
    }
    /// Adopt the project's stored view: zoom as a multiple of the reference
    /// scale, scroll in pixels.
    pub(in crate::editor) fn adopt_stored_view(&mut self, zoom: f64, scroll: f64) {
        let zoom = (f64::from(DEFAULT_ZOOM) * zoom)
            .clamp(f64::from(MIN_ZOOM_FLOOR), f64::from(MAX_ZOOM)) as f32;
        self.restore_view(zoom, (scroll / f64::from(zoom)) as f32);
    }
    /// The `(zoom, scroll)` pair the project stores: zoom as a multiple of the
    /// reference scale, scroll in pixels.
    pub(in crate::editor) fn stored_view(&self) -> (f64, f64) {
        (
            f64::from(self.zoom) / f64::from(DEFAULT_ZOOM),
            (f64::from(self.scroll) * f64::from(self.zoom)).clamp(0.0, 1e9),
        )
    }
    /// Adopt the persisted set of items whose keyframe lanes stay open.
    pub(in crate::editor) fn restore_lanes(&mut self, lanes: impl IntoIterator<Item = ItemId>) {
        self.expanded = lanes.into_iter().collect();
    }
    pub(in crate::editor) fn lane_open(&self, item: ItemId) -> bool {
        self.expanded.contains(&item)
    }
    pub(in crate::editor) fn set_lane(&mut self, item: ItemId, open: bool) {
        if open {
            self.expanded.insert(item);
        } else {
            self.expanded.remove(&item);
        }
    }
    pub(in crate::editor) fn expanded_lanes(&self) -> Vec<ItemId> {
        self.expanded.iter().copied().collect()
    }
    pub(in crate::editor) fn min_zoom(&self, duration: Tick) -> f32 {
        fit_zoom(self.viewport, duration.seconds())
    }
    /// One toolbar or shortcut step: ×1.7 in, ÷1.7 out.
    pub(in crate::editor) fn zoom_step(&mut self, zoom_in: bool, duration: Tick, playhead: Tick) {
        let factor = if zoom_in { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
        self.zoom_by(factor, duration, playhead);
    }
    pub(in crate::editor::timeline) fn zoom_by(
        &mut self,
        factor: f32,
        duration: Tick,
        playhead: Tick,
    ) {
        let target = (self.zoom * factor).clamp(MIN_ZOOM_FLOOR, MAX_ZOOM);
        self.apply_zoom(target, duration, playhead);
    }
    pub(in crate::editor::timeline) fn apply_zoom(
        &mut self,
        target: f32,
        duration: Tick,
        playhead: Tick,
    ) {
        let min = self.min_zoom(duration);
        let target = target.clamp(min, MAX_ZOOM);
        if (target - self.zoom).abs() <= f32::EPSILON {
            self.zoom = target;
            return;
        }
        let (scroll, pre_anchor) = anchored_scroll(
            self.zoom,
            target,
            min,
            self.scroll,
            playhead.seconds() as f32,
            self.pre_anchor_scroll,
        );
        self.zoom = target;
        self.scroll = scroll;
        self.pre_anchor_scroll = pre_anchor;
    }
    pub(in crate::editor::timeline) fn tick_at(&self, x: f32, left: f32) -> Tick {
        Tick::from_seconds_f64(
            f64::from(self.scroll + (x - left) / self.zoom),
            Rounding::Nearest,
        )
        .unwrap_or(Tick::ZERO)
    }
    pub(in crate::editor::timeline) fn x_at(&self, at: Tick, left: f32) -> f32 {
        left + (at.seconds() as f32 - self.scroll) * self.zoom
    }
    /// Drop missing presentation targets; return whether a fade needs rollback.
    pub(in crate::editor) fn forget_missing(&mut self, session: &engine::Session) -> bool {
        let data = session.project().data();
        if self
            .track_rename
            .as_ref()
            .is_some_and(|(id, _)| !data.scene().timeline.iter().any(|track| track.id == *id))
        {
            self.track_rename = None;
            self.track_rename_focus = false;
        }
        if self
            .scenes
            .rename
            .as_ref()
            .is_some_and(|(scene, _)| data.scenes.get(*scene).is_none())
        {
            self.scenes.rename = None;
        }
        let existing: BTreeSet<ItemId> = data
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.expanded.retain(|id| existing.contains(id));
        let fade_data = session
            .preview_project()
            .unwrap_or(session.project())
            .data();
        let cancel_fade = self.fade_drag.is_some_and(|drag| {
            !fade_data.scene().timeline.iter().any(|track| {
                !track.locked
                    && track.items.iter().any(|item| {
                        item.id == drag.item
                            && matches!(item.content, project::ItemParams::Audio(_))
                    })
            })
        });
        if cancel_fade || !session.gesture_active() {
            self.fade_drag = None;
        }
        // Another interaction took the gesture: the popover's previews are gone.
        if self
            .graph_popover
            .as_ref()
            .is_some_and(|popover| popover.staged && !session.gesture_active())
        {
            self.graph_popover = None;
        }
        if self.bookmark_popover.as_ref().is_some_and(|popover| {
            !data
                .scene()
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.id == popover.bookmark)
        }) {
            self.bookmark_popover = None;
        }
        cancel_fade
    }
    /// A refused scene rename keeps the typed name open for another try.
    pub(in crate::editor) fn keep_scene_rename(&mut self, scene: project::SceneId, name: String) {
        self.scenes.rename = Some((scene, name));
    }
    /// Forget a gesture the frame could not open.
    pub(in crate::editor) fn abandon_gesture(&mut self) {
        self.drag = None;
        self.bookmark_drag = None;
        self.keyframe_drag = None;
        self.volume_drag = None;
        self.fade_drag = None;
        self.drop_zone = None;
        self.preview = None;
        self.preview_noop = false;
        self.preview_armed = false;
        self.snap_indicator = None;
    }
    /// Arm the group-preview contract before the first staged command.
    pub(in crate::editor) fn arm_preview(&mut self) {
        self.preview = None;
        self.preview_armed = true;
        self.refusal = None;
    }
    /// Whether a staged group command may be sent now.
    pub(in crate::editor) fn preview_armed(&self) -> bool {
        self.preview_armed
    }
    /// Record the command backing the live candidate.
    pub(in crate::editor) fn mark_preview(&mut self, command: engine::CommandKind, noop: bool) {
        self.preview = Some(command);
        self.preview_noop = noop;
        self.preview_armed = true;
        self.refusal = None;
    }
    /// The staged command would leave every member at its entry position.
    pub(in crate::editor) fn preview_noop(&self) -> bool {
        self.preview_noop
    }
    /// The command id behind the live candidate, if any.
    pub(in crate::editor) fn preview_command(&self) -> Option<&'static str> {
        self.preview.map(engine::CommandKind::id)
    }
    /// Publish or discard finished: stop accepting staged commands.
    pub(in crate::editor) fn clear_preview(&mut self) {
        self.preview = None;
        self.preview_noop = false;
        self.preview_armed = false;
    }
    /// Show why a staged command was refused; the candidate is already gone.
    pub(in crate::editor) fn refuse(&mut self, message: impl Into<String>) {
        self.refusal = Some(message.into());
    }
    pub(in crate::editor) fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }
    /// The last landing hint, as a stable automation string.
    pub(in crate::editor) fn drop_zone_label(&self) -> Option<String> {
        self.drop_zone.map(|zone| match zone {
            DropZone::Track(track) => format!("track:{}", track.0),
            DropZone::Insert { track, above } => {
                format!(
                    "insert_{}:{}",
                    if above { "above" } else { "below" },
                    track.0
                )
            }
        })
    }
    /// Escape on a keyframe retime; the frame cancels the session gesture.
    pub(in crate::editor) fn cancel_keyframe_drag(&mut self) -> bool {
        let had = self.keyframe_drag.take().is_some();
        if had {
            self.snap_indicator = None;
        }
        had
    }
    /// Escape on a volume-line drag; the frame cancels the session gesture.
    pub(in crate::editor) fn cancel_volume_drag(&mut self) -> bool {
        self.volume_drag.take().is_some()
    }
    /// Escape on a fade-handle drag; the frame cancels the session gesture.
    pub(in crate::editor) fn cancel_fade_drag(&mut self) -> bool {
        self.fade_drag.take().is_some()
    }
    /// Escape on the keyframe marquee: the selection it started from.
    pub(in crate::editor) fn cancel_keyframe_marquee(
        &mut self,
    ) -> Option<BTreeSet<KeyframeSelection>> {
        self.keyframe_marquee.take().map(|marquee| marquee.before)
    }
    /// Open the marker editor under the clicked bookmark; a previous draft commits.
    pub(in crate::editor::timeline) fn open_bookmark_popover(
        &mut self,
        bookmark: BookmarkId,
        anchor: egui::Pos2,
        note_base: String,
        duration_base: Tick,
    ) {
        self.commit_bookmark_drafts();
        self.bookmark_popover = Some(BookmarkPopover {
            bookmark,
            anchor,
            note: None,
            duration: None,
            note_base,
            duration_base,
            picker: None,
        });
    }
    /// Escape on a bookmark drag; the frame cancels the session gesture.
    pub(in crate::editor) fn cancel_bookmark_drag(&mut self) -> bool {
        let had = self.bookmark_drag.take().is_some();
        if had {
            self.snap_indicator = None;
        }
        had
    }
    /// Escape on a clip drag; the frame cancels the session gesture.
    pub(in crate::editor) fn cancel_drag(&mut self) -> bool {
        let had = self.drag.take().is_some();
        if had {
            self.drop_zone = None;
            self.snap_indicator = None;
        }
        had
    }
    /// Escape on a track header drag or rename.
    pub(in crate::editor) fn cancel_track_edit(&mut self) -> bool {
        let drag = self.track_drag.take().is_some();
        if self.track_rename.take().is_some() || drag {
            self.track_rename_focus = false;
            return true;
        }
        false
    }
    /// Escape on the rename field alone.
    pub(in crate::editor) fn cancel_track_rename(&mut self) {
        self.track_rename = None;
        self.track_rename_focus = false;
    }
    /// Escape on the marquee: the selection it started from, to be restored.
    pub(in crate::editor) fn cancel_marquee(&mut self) -> Option<BTreeSet<ItemId>> {
        self.marquee.take().map(|(_, before, _)| before)
    }
    pub(in crate::editor::timeline) fn push(&mut self, action: Action) {
        self.outbox.push(action);
    }
    pub(in crate::editor::timeline) fn fail(&mut self, error: &dyn std::fmt::Display) {
        self.outbox.push(Action::Fail(error.to_string()));
    }
    /// Record a painted control for the frame's automation registry.
    pub(in crate::editor::timeline) fn remember(
        &mut self,
        id: impl Into<String>,
        response: &egui::Response,
    ) {
        self.remember_with(id, None, None, response);
    }
    pub(in crate::editor::timeline) fn remember_icon(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        response: &egui::Response,
    ) {
        self.remember_with(id, icon, None, response);
    }
    pub(in crate::editor::timeline) fn remember_with(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        tooltip: Option<String>,
        response: &egui::Response,
    ) {
        self.widgets.push(Remembered {
            id: id.into(),
            icon,
            tooltip,
            response: response.clone(),
        });
    }
    pub(in crate::editor::timeline) fn playhead_x(&self, cx: &Context<'_>, left: f32) -> f32 {
        self.x_at(cx.playhead, left)
    }
}

/// Parse a duration field in seconds, clamped at zero for the engine.
pub(in crate::editor::timeline) fn duration_tick(text: &str) -> Option<Tick> {
    let seconds = text.trim().parse::<f64>().ok()?;
    Tick::from_seconds_f64(seconds.max(0.0), Rounding::Nearest).ok()
}
