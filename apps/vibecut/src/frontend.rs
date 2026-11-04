use crate::{
    Error, Result,
    catalog::{self, Locale, Preferences, Text},
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    import::{ImportEvent, ImportFailure, ImportProbe, ImportQueue},
    playback::Playback,
};
use engine::{Command, CreatedId, NewItem, Session};
use media::{AssetId, SourceAsset, StreamKind};
use project::{ItemId, ItemParams, TrackId, TrackKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

const TOOLBAR_HEIGHT: f32 = 40.0;
const RULER_HEIGHT: f32 = 22.0;
const LABEL_WIDTH: f32 = 112.0;
const SCROLLBAR: f32 = 12.0;
const TRACK_GAP: f32 = 6.0;
const MIN_ZOOM: f32 = 4.0;
const MAX_ZOOM: f32 = 400.0;
const DEFAULT_ZOOM: f32 = 40.0;
const SNAP_PIXELS: f32 = 10.0;
const ZOOM_STEP: f32 = 1.7;
const LABEL_CHARS: usize = 120;

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub id: String,
    pub enabled: bool,
    pub focused: bool,
    pub rect: Rect,
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub widgets: Vec<WidgetState>,
    pub view: &'static str,
    pub playing: bool,
    pub position: Tick,
    pub selection: Vec<ItemId>,
    pub zoom: f32,
    pub snapping: bool,
    pub ripple: bool,
    pub importing: bool,
    pub import_completed: usize,
    pub import_total: usize,
    pub import_imported: usize,
    pub import_unfinished: usize,
    pub import_cancelled: bool,
    pub import_failures: Vec<String>,
    pub import_notice: Option<String>,
    pub import_error: Option<Arc<engine::EditError>>,
    pub import_toast: Option<String>,
    pub media_selection: Vec<AssetId>,
}
#[derive(Clone, Debug)]
struct Toast {
    text: String,
    detail: Option<String>,
    until: Instant,
}
#[derive(Clone, Debug)]
struct AssetRow {
    id: AssetId,
    name: String,
    info: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}
#[derive(Clone, Copy, Debug)]
struct DragOrigin {
    item: ItemId,
    track: TrackId,
    start: Tick,
    end: Tick,
}
#[derive(Clone, Debug)]
struct DragState {
    item: ItemId,
    mode: DragMode,
    press_x: f32,
    origins: Vec<DragOrigin>,
    source_range: SourceRange,
    moved: bool,
}
#[derive(Clone, Debug)]
struct TrackRow {
    kind: TrackKind,
    name: String,
    top: f32,
    height: f32,
}
#[derive(Clone, Debug)]
struct ItemView {
    id: ItemId,
    kind: TrackKind,
    start: Tick,
    end: Tick,
    top: f32,
    height: f32,
    locked: bool,
    label: String,
    overlay: Option<[u8; 4]>,
}
struct TimelineLayout {
    toolbar: Rect,
    ruler: Rect,
    body: Rect,
    labels: Rect,
    corner: Rect,
    horizontal: Rect,
    vertical: Rect,
}
impl TimelineLayout {
    fn new(rect: Rect) -> Self {
        let toolbar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOOLBAR_HEIGHT));
        let top = toolbar.bottom();
        let right = (rect.right() - SCROLLBAR).max(rect.left() + LABEL_WIDTH);
        let bottom = (rect.bottom() - SCROLLBAR).max(top + RULER_HEIGHT);
        let ruler = Rect::from_min_max(
            egui::pos2(rect.left() + LABEL_WIDTH, top),
            egui::pos2(right, top + RULER_HEIGHT),
        );
        let body = Rect::from_min_max(
            egui::pos2(ruler.left(), ruler.bottom()),
            egui::pos2(right, bottom),
        );
        let labels = Rect::from_min_max(
            egui::pos2(rect.left(), ruler.bottom()),
            egui::pos2(ruler.left(), bottom),
        );
        let corner = Rect::from_min_max(
            egui::pos2(rect.left(), top),
            egui::pos2(ruler.left(), ruler.bottom()),
        );
        let horizontal = Rect::from_min_max(
            egui::pos2(ruler.left(), bottom),
            egui::pos2(right, rect.bottom()),
        );
        let vertical = Rect::from_min_max(
            egui::pos2(right, ruler.bottom()),
            egui::pos2(rect.right(), bottom),
        );
        Self {
            toolbar,
            ruler,
            body,
            labels,
            corner,
            horizontal,
            vertical,
        }
    }
}
/// App state only. The immutable engine project is read through Session, never copied
/// into a parallel frontend project model. Every edit dispatches its registry ID.
pub struct Editor {
    pub session: crate::lifecycle::EditorSession,
    pub playback: Playback,
    pub preferences: Preferences,
    pub selection: BTreeSet<ItemId>,
    pub snapping: bool,
    pub ripple: bool,
    zoom: f32,
    scroll: f32,
    vertical_scroll: f32,
    last_view: project::ViewState,
    widgets: BTreeMap<String, WidgetState>,
    pub panels: BTreeMap<&'static str, Rect>,
    drag: Option<DragState>,
    snap_indicator: Option<Tick>,
    texture: Option<egui::TextureHandle>,
    texture_key: Option<(u64, Tick)>,
    texture_owner: Option<std::sync::Arc<crate::playback::Frame>>,
    error: Option<String>,
    pub(crate) persistence_error: Option<std::sync::Arc<engine::EditError>>,
    configured: bool,
    details: bool,
    rename: Option<String>,
    rename_focus: bool,
    pub(crate) lifecycle_action: Option<crate::lifecycle::EditorAction>,
    imports: Option<ImportQueue>,
    import_total: usize,
    import_completed: usize,
    import_imported: usize,
    import_cancelled: bool,
    import_failures: Vec<ImportFailure>,
    import_notice: Option<String>,
    import_error: Option<Arc<engine::EditError>>,
    toast: Option<Toast>,
    media_selection: BTreeSet<AssetId>,
    media_anchor: Option<AssetId>,
}
impl Editor {
    pub fn new(session: Session, playback: Playback, preferences: Preferences) -> Self {
        Self::with_session(
            crate::lifecycle::EditorSession::Memory(Box::new(session)),
            playback,
            preferences,
        )
    }
    fn with_session(
        session: crate::lifecycle::EditorSession,
        playback: Playback,
        preferences: Preferences,
    ) -> Self {
        let view = session.metadata().view;
        let zoom = (f64::from(DEFAULT_ZOOM) * view.zoom)
            .clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)) as f32;
        let scroll = (view.scroll / f64::from(zoom)) as f32;
        Self {
            session,
            playback,
            preferences,
            selection: BTreeSet::new(),
            snapping: true,
            ripple: false,
            zoom,
            scroll,
            vertical_scroll: 0.0,
            last_view: view,
            widgets: BTreeMap::new(),
            panels: BTreeMap::new(),
            drag: None,
            snap_indicator: None,
            texture: None,
            texture_key: None,
            texture_owner: None,
            error: None,
            persistence_error: None,
            configured: false,
            details: false,
            rename: None,
            rename_focus: false,
            lifecycle_action: None,
            imports: None,
            import_total: 0,
            import_completed: 0,
            import_imported: 0,
            import_cancelled: false,
            import_failures: Vec::new(),
            import_notice: None,
            import_error: None,
            toast: None,
            media_selection: BTreeSet::new(),
            media_anchor: None,
        }
    }
    /// Attach the engine autosave owner without duplicating its Session.
    pub fn persisted(
        autosave: engine::recovery::Autosave,
        playback: Playback,
        preferences: Preferences,
    ) -> Self {
        Self::with_session(
            crate::lifecycle::EditorSession::Persistent(Box::new(autosave)),
            playback,
            preferences,
        )
    }
    pub fn inspect(&self) -> Result<Inspection> {
        Ok(Inspection {
            widgets: self.widgets.values().cloned().collect(),
            view: "editor",
            playing: self.playback.playing(),
            position: self.playback.position()?,
            selection: self.selection.iter().copied().collect(),
            zoom: self.zoom,
            snapping: self.snapping,
            ripple: self.ripple,
            importing: self.importing(),
            import_completed: self.import_completed,
            import_total: self.import_total,
            import_imported: self.import_imported,
            import_unfinished: self.import_total.saturating_sub(self.import_completed),
            import_cancelled: self.import_cancelled,
            import_failures: self
                .import_failures
                .iter()
                .map(|failure| {
                    format!(
                        "{}: {}",
                        failure.path.file_name().map_or_else(
                            || failure.path.to_string_lossy().into_owned(),
                            |name| name.to_string_lossy().into_owned()
                        ),
                        failure.reason
                    )
                })
                .collect(),
            import_notice: self.import_notice.clone(),
            import_error: self.import_error.clone(),
            import_toast: self.toast.as_ref().map(|toast| match &toast.detail {
                Some(detail) => format!("{} · {detail}", toast.text),
                None => toast.text.clone(),
            }),
            media_selection: self.media_selection.iter().copied().collect(),
        })
    }
    /// Last frame submitted by the frontend for presentation, independent of decode-ahead.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.texture_key.map(|(_, at)| at)
    }
    /// Replace the media boundary in tests; production keeps the media owner.
    pub fn set_import_probe(&mut self, probe: Arc<dyn ImportProbe>) -> Result<()> {
        self.import_queue()?.set_probe(probe);
        Ok(())
    }
    pub fn importing(&self) -> bool {
        self.imports.as_ref().is_some_and(ImportQueue::active)
    }
    /// Accept a native drag-drop batch through the same path as the picker.
    pub fn drop_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.start_import(paths)
    }
    pub fn start_import(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        if self.importing() {
            return Err(Error::Invalid("import already in progress"));
        }
        if paths.is_empty() {
            return Ok(());
        }
        let first = self.next_asset_id();
        self.import_total = paths.len();
        self.import_completed = 0;
        self.import_imported = 0;
        self.import_cancelled = false;
        self.import_failures.clear();
        self.import_notice = None;
        self.import_error = None;
        self.toast = None;
        let result = self.import_queue()?.start(paths, first);
        if result.is_err() {
            self.import_total = 0;
        }
        result
    }
    fn import_queue(&mut self) -> Result<&mut ImportQueue> {
        if self.imports.is_none() {
            self.imports = Some(ImportQueue::new(self.playback.budget().clone())?);
        }
        self.imports
            .as_mut()
            .ok_or(Error::Invalid("import queue unavailable"))
    }
    fn next_asset_id(&self) -> u64 {
        self.session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| asset.id().0)
            .max()
            .map_or(1, |id| id.saturating_add(1))
    }
    fn add_imported_asset(&mut self, asset: SourceAsset) -> Result<()> {
        let next = self.next_asset_id();
        let asset = if asset.id().0 == next {
            asset
        } else {
            let mut data = media::AssetData::from(asset);
            data.id = AssetId(next);
            SourceAsset::try_from(data)
                .map_err(|error| Error::Engine(engine::EditError::Media(error)))?
        };
        self.session.apply(Command::AddAsset { asset })?;
        Ok(())
    }
    fn apply_import_events(&mut self) {
        let polled = match self.imports.as_mut() {
            Some(queue) => queue.poll(),
            None => Ok(Vec::new()),
        };
        let events = match polled {
            Ok(events) => events,
            Err(error) => {
                self.import_notice = Some(error_detail(error.as_ref()));
                self.import_error = Some(error);
                if let Some(queue) = &self.imports {
                    queue.cancel();
                }
                self.finish_import();
                return;
            }
        };
        for event in events {
            match event {
                ImportEvent::Imported(asset) => {
                    let path = asset.path().to_owned();
                    self.import_completed += 1;
                    if let Err(error) = self.add_imported_asset(asset) {
                        self.import_failures
                            .push(ImportFailure::local(path, error_detail(&error)));
                    } else {
                        self.import_imported += 1;
                    }
                }
                ImportEvent::Failed(failure) => {
                    self.import_completed += 1;
                    self.import_failures.push(failure);
                }
                ImportEvent::Done { cancelled, error } => {
                    self.import_cancelled = cancelled;
                    if let Some(error) = error {
                        self.import_notice = Some(error_detail(error.as_ref()));
                        self.import_error = Some(error);
                    }
                    self.finish_import();
                }
            }
        }
        self.prune_media_selection();
    }
    fn finish_import(&mut self) {
        let failed = self.import_failures.len();
        let unfinished = self.import_total.saturating_sub(self.import_completed);
        let detail = format!(
            "{}: {} · {}: {failed} · {}: {unfinished}",
            self.tr(Text::Imported),
            self.import_imported,
            self.tr(Text::Failed),
            self.tr(Text::Unfinished)
        );
        let text = if self.import_error.is_some() {
            self.tr(Text::ImportFailed)
        } else if self.import_cancelled {
            self.tr(Text::ImportCancelled)
        } else if failed > 0 || unfinished > 0 {
            self.tr(Text::ImportPartial)
        } else {
            self.tr(Text::ImportComplete)
        };
        self.set_toast(text, Some(detail));
        self.import_total = self.import_total.max(self.import_completed);
    }
    fn set_toast(&mut self, text: &str, detail: Option<String>) {
        self.toast = Some(Toast {
            text: text.to_owned(),
            detail,
            until: Instant::now() + std::time::Duration::from_secs(5),
        });
    }
    fn truncate_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| Instant::now() >= toast.until)
        {
            self.toast = None;
        }
    }
    fn prune_media_selection(&mut self) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(SourceAsset::id)
            .collect();
        self.media_selection.retain(|id| existing.contains(id));
        if self
            .media_anchor
            .is_some_and(|anchor| !existing.contains(&anchor))
        {
            self.media_anchor = None;
        }
    }
    /// Replace the media selection; tests and agents use the same state as clicks.
    pub fn select_media(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        self.media_selection = ids.into_iter().collect();
        self.media_anchor = self.media_selection.iter().next().copied();
        self.session.end_merge();
    }
    fn select_asset(&mut self, id: AssetId, modifiers: egui::Modifiers) {
        let ids: Vec<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(SourceAsset::id)
            .collect();
        if modifiers.shift
            && let Some(anchor) = self.media_anchor
            && let (Some(from), Some(to)) = (
                ids.iter().position(|candidate| *candidate == anchor),
                ids.iter().position(|candidate| *candidate == id),
            )
        {
            let (from, to) = if from <= to { (from, to) } else { (to, from) };
            if let Some(range) = ids.get(from..=to) {
                self.media_selection = range.iter().copied().collect();
            }
            self.session.end_merge();
            return;
        }
        if modifiers.command {
            if !self.media_selection.remove(&id) {
                self.media_selection.insert(id);
            }
        } else {
            self.media_selection.clear();
            self.media_selection.insert(id);
        }
        self.media_anchor = Some(id);
        self.session.end_merge();
    }
    /// Add only selected assets, in library order, appending after existing
    /// compatible items so insertion can never overlap or reorder a track.
    fn insert_selected_assets(&mut self) {
        let selected: Vec<SourceAsset> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| self.media_selection.contains(&asset.id()))
            .cloned()
            .collect();
        if selected.is_empty() {
            return;
        }
        let at = match self.playback.position() {
            Ok(at) => at,
            Err(error) => {
                self.set_toast(self.tr(Text::InsertFailed), Some(error_detail(&error)));
                return;
            }
        };
        let result = self.session.transaction(|session| {
            let mut created = Vec::new();
            for asset in &selected {
                let kind = asset_kind(asset)?;
                let duration = asset
                    .probe()
                    .duration
                    .filter(|duration| *duration > Tick::ZERO)
                    .ok_or(engine::EditError::Unsupported(
                        "media duration is unavailable",
                    ))?;
                let existing = session
                    .project()
                    .data()
                    .timeline
                    .tracks
                    .iter()
                    .find(|track| track.kind == kind && !track.locked)
                    .map(|track| {
                        (
                            track.id,
                            track
                                .items
                                .iter()
                                .filter_map(|item| item.end().ok())
                                .max()
                                .unwrap_or(Tick::ZERO),
                        )
                    });
                let (track, start) = match existing {
                    Some((track, end)) => (track, at.max(end)),
                    None => {
                        let count = session
                            .project()
                            .data()
                            .timeline
                            .tracks
                            .iter()
                            .filter(|track| track.kind == kind)
                            .count();
                        let name = match kind {
                            TrackKind::Video => format!("Video {}", count + 1),
                            _ => format!("Audio {}", count + 1),
                        };
                        let outcome = session.apply(Command::AddTrack {
                            name: name.into(),
                            kind,
                        })?;
                        let track = match outcome.created.first().copied() {
                            Some(CreatedId::Track(track)) => track,
                            _ => {
                                return Err(engine::EditError::Unsupported("track allocation"));
                            }
                        };
                        (track, at)
                    }
                };
                let item = NewItem {
                    animations: Default::default(),
                    source_asset: Some(asset.id()),
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start,
                    content: match kind {
                        TrackKind::Video => ItemParams::Video(project::VideoParams {
                            source_audio_enabled: true,
                        }),
                        _ => ItemParams::Audio(project::AudioParams {
                            gain_db: 0.0,
                            muted: false,
                        }),
                    },
                };
                let outcome = session.apply(Command::InsertItem { track, item })?;
                created.extend(outcome.created.into_iter().filter_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                }));
            }
            Ok(created)
        });
        match result {
            Ok(created) => {
                self.selection = created.into_iter().collect();
                self.import_notice = None;
                self.session.end_merge();
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::InsertFailed), Some(detail));
            }
        }
    }
    fn tr(&self, key: Text) -> &'static str {
        catalog::text(self.preferences.locale, key)
    }
    fn remember(&mut self, id: impl Into<String>, response: &egui::Response) {
        let id = id.into();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(&id));
        self.widgets.insert(
            id.clone(),
            WidgetState {
                id,
                enabled: response.enabled(),
                focused: response.has_focus(),
                rect: response.rect,
            },
        );
    }
    fn button(&mut self, ui: &mut egui::Ui, id: &'static str, key: Text, enabled: bool) -> bool {
        let label = self.tr(key);
        let response = ui
            .push_id(id, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::new(label).min_size(Vec2::new(28.0, 26.0)),
                )
            })
            .inner;
        self.remember(id, &response);
        response.clicked()
    }
    fn toggle_button(&mut self, ui: &mut egui::Ui, id: &'static str, key: Text, on: bool) -> bool {
        let label = self.tr(key);
        let response = ui.push_id(id, |ui| ui.selectable_label(on, label)).inner;
        self.remember(id, &response);
        response.clicked()
    }
    /// All control inputs (including tests/agents) use the same action path.
    /// Project edits are engine registry calls; transport/preferences are app state.
    pub fn activate(&mut self, id: &str) -> Result<()> {
        self.remember_view()?;
        let at = self.playback.position()?;
        let duration = self.session.project().duration();
        let playing = self.playback.playing();
        let before = self.session.project().clone();
        match id {
            "project.save" => self.lifecycle_action = Some(crate::lifecycle::EditorAction::Save),
            "project.save_as" => {
                self.lifecycle_action = Some(crate::lifecycle::EditorAction::SaveAs)
            }
            "project.back" => self.lifecycle_action = Some(crate::lifecycle::EditorAction::Leave),
            "project.rename" => {
                self.rename = Some(self.session.project().data().name.as_str().to_owned());
                self.rename_focus = true;
            }
            "file.import" => {
                if self.importing() {
                    self.import_notice = Some("import already in progress".to_owned());
                } else {
                    self.lifecycle_action = Some(crate::lifecycle::EditorAction::ImportMedia);
                }
            }
            "media.insert" => self.insert_selected_assets(),
            "import.cancel" => {
                if let Some(queue) = &self.imports {
                    queue.cancel();
                }
            }
            "transport.play" => self.playback.seek(
                if at >= duration { Tick::ZERO } else { at },
                !playing,
                self.session.project().clone(),
            )?,
            "transport.pause" => self
                .playback
                .seek(at, false, self.session.project().clone())?,
            "transport.back" => self.seek((at - Tick(TICKS_PER_SECOND)).max(Tick::ZERO))?,
            "transport.forward" => self.seek((at + Tick(TICKS_PER_SECOND)).min(duration))?,
            "transport.frame_back" => self.step_frames(-1)?,
            "transport.frame_forward" => self.step_frames(1)?,
            "transport.start" => self.seek(Tick::ZERO)?,
            "transport.end" => self.seek(duration)?,
            "edit.undo" | "edit.redo" => {
                self.session.execute(
                    if id == "edit.undo" { "undo" } else { "redo" },
                    serde_json::json!({}),
                )?;
                self.prune_selection();
            }
            "timeline.split" => self.split_at_playhead()?,
            "timeline.delete" => self.delete_selected(self.ripple)?,
            "timeline.ripple_delete" => self.delete_selected(true)?,
            "timeline.select_all" => self.select_all(),
            "timeline.snap" => self.snapping = !self.snapping,
            "timeline.ripple" => self.ripple = !self.ripple,
            "timeline.zoom_in" => self.zoom_by(ZOOM_STEP),
            "timeline.zoom_out" => self.zoom_by(1.0 / ZOOM_STEP),
            "appearance.theme" => self.preferences.dark = !self.preferences.dark,
            "appearance.locale" => {
                self.preferences.locale = if self.preferences.locale == Locale::En {
                    Locale::Vi
                } else {
                    Locale::En
                }
            }
            "layout.reset" => self.preferences.panels = Preferences::default().panels,
            "preview.retry" => {
                self.playback.clear()?;
                self.error = None;
            }
            "inspector.project" => self.details = false,
            "inspector.details" => self.details = true,
            id if id.starts_with("timeline.item.") => {
                let raw = id.trim_start_matches("timeline.item.");
                let parsed = raw
                    .parse::<u64>()
                    .map_err(|_| Error::Unsupported("widget action"))?;
                self.select_single(ItemId(parsed));
            }
            _ => return Err(Error::Unsupported("widget action")),
        }
        self.prune_media_selection();
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
        self.remember_view()?;
        Ok(())
    }
    /// Restart preview/audio work against the current immutable project snapshot.
    /// Old generations are fenced by `Playback::seek`; the position is clamped when
    /// an edit shortens the timeline and the transport state is preserved when able.
    fn sync_preview(&mut self) {
        let project = self.session.project().clone();
        let duration = project.duration();
        let at = self
            .playback
            .position()
            .unwrap_or(Tick::ZERO)
            .clamp(Tick::ZERO, duration);
        let playing = self.playback.playing();
        if self.playback.seek(at, playing, project.clone()).is_err()
            && let Err(error) = self.playback.seek(at, false, project)
        {
            self.error = Some(error.to_string());
        }
    }
    fn seek(&mut self, at: Tick) -> Result<()> {
        self.playback
            .seek(at, self.playback.playing(), self.session.project().clone())?;
        self.remember_view()
    }
    fn remember_view(&mut self) -> Result<()> {
        if self.session.transaction_active() {
            return Ok(());
        }
        let stored = self.session.metadata().view;
        // Other adapters can issue the engine view command between frontend actions.
        if stored != self.last_view {
            self.zoom = (f64::from(DEFAULT_ZOOM) * stored.zoom)
                .clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)) as f32;
            self.scroll = (stored.scroll / f64::from(self.zoom)) as f32;
            self.playback.seek(
                stored
                    .playhead
                    .clamp(Tick::ZERO, self.session.project().duration()),
                self.playback.playing(),
                self.session.project().clone(),
            )?;
            self.last_view = stored;
        }
        let view = project::ViewState {
            zoom: f64::from(self.zoom) / f64::from(DEFAULT_ZOOM),
            scroll: (f64::from(self.scroll) * f64::from(self.zoom)).clamp(0.0, 1e9),
            playhead: self.playback.position()?,
        };
        if self.session.metadata().view != view {
            self.session.execute(
                "project.set_view_state",
                serde_json::json!({"zoom":view.zoom,"scroll":view.scroll,"playhead":view.playhead}),
            )?;
        }
        self.last_view = view;
        Ok(())
    }
    fn seek_to(&mut self, at: Tick) {
        let at = at.clamp(Tick::ZERO, self.session.project().duration());
        if let Err(error) = self.seek(at) {
            self.error = Some(error.to_string());
        }
    }
    fn step_frames(&mut self, frames: i64) -> Result<()> {
        let frame = self.frame_ticks();
        let at = self.playback.position()?;
        self.seek(
            (at + Tick(frame.0.saturating_mul(frames)))
                .clamp(Tick::ZERO, self.session.project().duration()),
        )
    }
    fn frame_ticks(&self) -> Tick {
        self.session
            .project()
            .data()
            .canvas
            .fps
            .frame_duration()
            .ok()
            .filter(|frame| *frame > Tick::ZERO)
            .unwrap_or(Tick(1))
    }
    fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    }
    fn action(&mut self, id: &str) {
        if let Err(error) = self.activate(id) {
            self.error = Some(error.to_string());
        }
    }
    fn select_single(&mut self, item: ItemId) {
        self.selection.clear();
        self.selection.insert(item);
        self.session.end_merge();
    }
    fn select_item(&mut self, item: ItemId, modifiers: egui::Modifiers) {
        if modifiers.command || modifiers.shift {
            if !self.selection.remove(&item) {
                self.selection.insert(item);
            }
        } else {
            self.selection.clear();
            self.selection.insert(item);
        }
        self.session.end_merge();
    }
    fn select_all(&mut self) {
        self.selection = self
            .session
            .project()
            .data()
            .timeline
            .tracks
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.session.end_merge();
    }
    fn prune_selection(&mut self) {
        let existing: BTreeSet<ItemId> = self
            .session
            .project()
            .data()
            .timeline
            .tracks
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection.retain(|id| existing.contains(id));
    }
    fn split_at_playhead(&mut self) -> Result<()> {
        let at = self.playback.position()?;
        let mut under = Vec::new();
        let mut selected_under = Vec::new();
        for track in &self.session.project().data().timeline.tracks {
            if track.locked {
                continue;
            }
            for item in &track.items {
                if item.start < at && item.end().is_ok_and(|end| at < end) {
                    under.push(item.id);
                    if self.selection.contains(&item.id) {
                        selected_under.push(item.id);
                    }
                }
            }
        }
        // A nonempty selection owns the edit even when no selected item crosses the
        // playhead: splitting then is a no-op, never a fallback to unrelated clips.
        let targets = if self.selection.is_empty() {
            under
        } else {
            selected_under
        };
        if targets.is_empty() {
            return Ok(());
        }
        let mut created = Vec::new();
        self.session.transaction(|session| {
            for item in targets {
                created.extend(session.apply(Command::SplitAt { item, at })?.created);
            }
            Ok(())
        })?;
        self.selection = created
            .into_iter()
            .filter_map(|id| match id {
                CreatedId::Item(item) => Some(item),
                _ => None,
            })
            .collect();
        self.session.end_merge();
        Ok(())
    }
    fn delete_selected(&mut self, ripple: bool) -> Result<()> {
        if self.selection.is_empty() {
            return Ok(());
        }
        if ripple {
            let order = self.selection_by_end(true);
            self.session.transaction(|session| {
                for item in &order {
                    session.apply(Command::RippleDelete { item: *item })?;
                }
                Ok(())
            })?;
        } else {
            let items: Vec<ItemId> = self.selection.iter().copied().collect();
            self.session.apply(Command::Delete { items })?;
        }
        self.selection.clear();
        self.session.end_merge();
        Ok(())
    }
    fn selection_by_end(&self, descending: bool) -> Vec<ItemId> {
        let mut items: Vec<(Tick, ItemId)> = self
            .session
            .project()
            .data()
            .timeline
            .tracks
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|item| self.selection.contains(&item.id))
            .map(|item| (item.start, item.id))
            .collect();
        items.sort_by(|a, b| {
            if descending {
                b.0.cmp(&a.0)
            } else {
                a.0.cmp(&b.0)
            }
        });
        items.into_iter().map(|(_, id)| id).collect()
    }
    fn begin_drag(&mut self, item: ItemId, mode: DragMode, ui: &egui::Ui) {
        if self.drag.is_some() {
            self.finish_drag();
        }
        if !self.selection.contains(&item) {
            self.select_single(item);
        }
        let Some(press_x) = ui
            .ctx()
            .input(|input| input.pointer.press_origin().map(|origin| origin.x))
        else {
            return;
        };
        self.session.end_merge();
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.error = Some(error.to_string());
            return;
        }
        if let Err(error) = self.session.begin_gesture() {
            self.error = Some(error.to_string());
            return;
        }
        let mut origins = Vec::new();
        let mut source_range = None;
        for track in &self.session.project().data().timeline.tracks {
            for candidate in &track.items {
                let wanted = if mode == DragMode::Move {
                    self.selection.contains(&candidate.id)
                } else {
                    candidate.id == item
                };
                if !wanted {
                    continue;
                }
                if let Ok(end) = candidate.end() {
                    origins.push(DragOrigin {
                        item: candidate.id,
                        track: track.id,
                        start: candidate.start,
                        end,
                    });
                    if candidate.id == item {
                        source_range = Some(candidate.source_range);
                    }
                }
            }
        }
        let Some(source_range) = source_range else {
            let _ = self.session.cancel_gesture();
            return;
        };
        self.drag = Some(DragState {
            item,
            mode,
            press_x,
            origins,
            source_range,
            moved: false,
        });
    }
    /// Drive the active gesture from global pointer state. Release and update do not
    /// depend on the item or handle still being visible or hit-testable this frame.
    fn advance_drag(&mut self, ui: &egui::Ui, body_left: f32) {
        if self.drag.is_none() {
            return;
        }
        let (pointer, bypass, down) = ui.ctx().input(|input| {
            (
                input.pointer.interact_pos(),
                input.modifiers.shift,
                input.pointer.primary_down(),
            )
        });
        if let Some(pointer) = pointer {
            self.update_drag(pointer, body_left, bypass);
        }
        if !down {
            self.finish_drag();
        }
    }
    fn update_drag(&mut self, pointer: egui::Pos2, body_left: f32, bypass: bool) {
        let Some(mut drag) = self.drag.clone() else {
            return;
        };
        let result = match drag.mode {
            DragMode::Move => self.apply_drag_move(&drag, pointer.x, bypass),
            DragMode::TrimStart => {
                self.apply_drag_trim(&drag, pointer.x, body_left, bypass, DragMode::TrimStart)
            }
            DragMode::TrimEnd => {
                self.apply_drag_trim(&drag, pointer.x, body_left, bypass, DragMode::TrimEnd)
            }
        };
        let changed = match result {
            Ok(changed) => changed,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        };
        drag.moved |= changed;
        self.drag = Some(drag);
        if changed {
            self.sync_preview();
        }
    }
    fn apply_drag_move(&mut self, drag: &DragState, pointer_x: f32, bypass: bool) -> Result<bool> {
        let raw = Tick::from_seconds_f64(
            f64::from(pointer_x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (snapped, indicator) = self.snap_delta(raw, &drag.origins, bypass);
        let (low, high) = self.move_bounds(&drag.origins);
        let delta = snapped.clamp(low, high);
        self.snap_indicator = indicator;
        // Always target the origin plus this delta. A return to the press point must
        // restore the gesture-entry positions rather than leave an earlier preview.
        let moves = drag
            .origins
            .iter()
            .map(|origin| engine::Move {
                item: origin.item,
                track: origin.track,
                start: origin.start + delta,
            })
            .collect();
        Ok(self.session.apply(Command::MoveItems { moves })?.changed)
    }
    fn apply_drag_trim(
        &mut self,
        drag: &DragState,
        pointer_x: f32,
        body_left: f32,
        bypass: bool,
        mode: DragMode,
    ) -> Result<bool> {
        let raw = Tick::from_seconds_f64(
            f64::from(self.scroll + (pointer_x - body_left) / self.zoom),
            Rounding::Nearest,
        )?;
        let exclude: BTreeSet<ItemId> = drag.origins.iter().map(|origin| origin.item).collect();
        let (snapped, indicator) = self.snap_tick(raw, &exclude, bypass);
        self.snap_indicator = indicator;
        let (low, high) = self.trim_bounds(drag, mode);
        let at = snapped.clamp(low, high);
        let command = match mode {
            DragMode::TrimStart => Command::TrimStart {
                item: drag.item,
                at,
            },
            DragMode::TrimEnd => Command::TrimEnd {
                item: drag.item,
                at,
            },
            DragMode::Move => return Ok(false),
        };
        Ok(self.session.apply(command)?.changed)
    }
    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let mut edited = false;
        if self.session.gesture_active() {
            let outcome = if drag.moved {
                edited = true;
                self.session.commit_gesture()
            } else {
                self.session.cancel_gesture()
            };
            if let Err(error) = outcome {
                self.error = Some(error.to_string());
            }
        }
        if edited {
            self.sync_preview();
        }
        self.snap_indicator = None;
        self.prune_selection();
    }
    fn cancel_interaction(&mut self) {
        if let Some(drag) = self.drag.take() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            if drag.moved {
                self.sync_preview();
            }
            self.snap_indicator = None;
        } else {
            self.selection.clear();
            self.session.end_merge();
        }
    }
    fn move_bounds(&self, origins: &[DragOrigin]) -> (Tick, Tick) {
        let tracks: BTreeSet<TrackId> = origins.iter().map(|origin| origin.track).collect();
        let selected: BTreeSet<ItemId> = origins.iter().map(|origin| origin.item).collect();
        let earliest = origins
            .iter()
            .map(|origin| origin.start)
            .min()
            .unwrap_or(Tick::ZERO);
        // One shared delta keeps the selected group rigid at the timeline zero edge.
        let mut low = Tick(earliest.0.saturating_neg());
        let mut high = Tick(i64::MAX / 4);
        for track in &self.session.project().data().timeline.tracks {
            if !tracks.contains(&track.id) {
                continue;
            }
            let track_origins: Vec<&DragOrigin> = origins
                .iter()
                .filter(|origin| origin.track == track.id)
                .collect();
            for item in &track.items {
                if selected.contains(&item.id) {
                    continue;
                }
                let Ok(end) = item.end() else {
                    continue;
                };
                let previous_end = track_origins
                    .iter()
                    .filter(|origin| origin.end <= item.start)
                    .map(|origin| origin.end)
                    .max();
                if let Some(previous_end) = previous_end {
                    high = high.min(item.start - previous_end);
                }
                let next_start = track_origins
                    .iter()
                    .filter(|origin| origin.start >= end)
                    .map(|origin| origin.start)
                    .min();
                if let Some(next_start) = next_start {
                    low = low.max(end - next_start);
                }
            }
        }
        (low, high)
    }
    fn trim_bounds(&self, drag: &DragState, mode: DragMode) -> (Tick, Tick) {
        let minimum = self.frame_ticks();
        let origin = drag
            .origins
            .iter()
            .find(|origin| origin.item == drag.item)
            .copied();
        let Some(origin) = origin else {
            return (Tick::ZERO, Tick::ZERO);
        };
        let mut low = match mode {
            DragMode::TrimStart => (origin.start - drag.source_range.start()).max(Tick::ZERO),
            DragMode::TrimEnd => origin.start + minimum,
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        let mut high = match mode {
            DragMode::TrimStart => origin.end - minimum,
            DragMode::TrimEnd => Tick(i64::MAX / 4),
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        for track in &self.session.project().data().timeline.tracks {
            if track.id != origin.track {
                continue;
            }
            for item in &track.items {
                if item.id == origin.item {
                    continue;
                }
                let Ok(end) = item.end() else {
                    continue;
                };
                match mode {
                    DragMode::TrimStart if end <= origin.start => low = low.max(end),
                    DragMode::TrimEnd if item.start >= origin.end => high = high.min(item.start),
                    _ => {}
                }
            }
        }
        if mode == DragMode::TrimEnd
            && let Some(asset) = self
                .session
                .project()
                .data()
                .assets
                .iter()
                .find(|asset| Some(asset.id()) == self.item_asset(drag.item))
            && let Some(duration) = asset.probe().duration
        {
            high = high.min(origin.start + (duration - drag.source_range.start()));
        }
        if high < low {
            high = low;
        }
        (low, high)
    }
    fn item_asset(&self, item: ItemId) -> Option<media::AssetId> {
        self.session
            .project()
            .data()
            .timeline
            .tracks
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item)
            .and_then(|candidate| candidate.source_asset)
    }
    fn snap_points(&self, exclude: &BTreeSet<ItemId>) -> Vec<Tick> {
        let project = self.session.project();
        let data = project.data();
        let mut points = vec![Tick::ZERO, self.playback.position().unwrap_or(Tick::ZERO)];
        points.push(project.duration());
        points.extend(
            data.bookmarks
                .iter()
                .flat_map(|bookmark| [bookmark.start, bookmark.start + bookmark.duration]),
        );
        points.extend(
            data.timeline
                .tracks
                .iter()
                .flat_map(|track| track.items.iter())
                .filter(|item| !exclude.contains(&item.id))
                .flat_map(|item| [item.start, item.end().unwrap_or(item.start)]),
        );
        points
    }
    fn snap_delta(&self, raw: Tick, origins: &[DragOrigin], bypass: bool) -> (Tick, Option<Tick>) {
        if !self.snapping || bypass {
            return (raw, None);
        }
        let exclude: BTreeSet<ItemId> = origins.iter().map(|origin| origin.item).collect();
        let points = self.snap_points(&exclude);
        let threshold = self.snap_threshold();
        let mut best: Option<(Tick, Tick)> = None;
        for origin in origins {
            for edge in [origin.start, origin.end] {
                let current = edge + raw;
                for point in &points {
                    let difference = *point - current;
                    if difference.abs() <= threshold
                        && best.is_none_or(|(distance, _)| difference.abs() < distance.abs())
                    {
                        best = Some((difference, *point));
                    }
                }
            }
        }
        match best {
            Some((difference, point)) => (raw + difference, Some(point)),
            None => (raw, None),
        }
    }
    fn snap_tick(
        &self,
        raw: Tick,
        exclude: &BTreeSet<ItemId>,
        bypass: bool,
    ) -> (Tick, Option<Tick>) {
        if !self.snapping || bypass {
            return (raw, None);
        }
        let threshold = self.snap_threshold();
        let mut best: Option<(Tick, Tick)> = None;
        for point in self.snap_points(exclude) {
            let difference = point - raw;
            if difference.abs() <= threshold
                && best.is_none_or(|(distance, _)| difference.abs() < distance.abs())
            {
                best = Some((difference, point));
            }
        }
        match best {
            Some((difference, point)) => (raw + difference, Some(point)),
            None => (raw, None),
        }
    }
    fn snap_threshold(&self) -> Tick {
        Tick::from_seconds_f64(
            f64::from(SNAP_PIXELS) / f64::from(self.zoom),
            Rounding::Nearest,
        )
        .unwrap_or(Tick::ZERO)
    }
    fn tick_at(&self, x: f32, left: f32) -> Tick {
        Tick::from_seconds_f64(
            f64::from(self.scroll + (x - left) / self.zoom),
            Rounding::Nearest,
        )
        .unwrap_or(Tick::ZERO)
    }
    fn x_at(&self, at: Tick, left: f32) -> f32 {
        left + (at.seconds() as f32 - self.scroll) * self.zoom
    }
    pub(crate) fn theme(ctx: &egui::Context, dark: bool) {
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let panel = if dark {
            Color32::from_rgb(15, 15, 15)
        } else {
            Color32::from_rgb(249, 250, 251)
        };
        visuals.panel_fill = panel;
        visuals.window_fill = panel;
        visuals.override_text_color = Some(if dark {
            Color32::from_rgb(229, 229, 229)
        } else {
            Color32::from_rgb(28, 28, 28)
        });
        visuals.selection.bg_fill = if dark {
            Color32::from_rgb(0, 193, 205)
        } else {
            Color32::from_rgb(0, 106, 116)
        };
        ctx.set_visuals(visuals);
    }
    pub(crate) fn fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert(
            "Inter".into(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                "../../../assets/fonts/Inter-Regular.ttf"
            ))),
        );
        fonts
            .families
            .insert(egui::FontFamily::Proportional, vec!["Inter".into()]);
        fonts
            .families
            .insert(egui::FontFamily::Monospace, vec!["Inter".into()]);
        ctx.set_fonts(fonts);
    }
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        if let Some(autosave) = self.session.autosave()
            && let Err(e) = autosave.tick()
        {
            self.persistence_error = Some(std::sync::Arc::new(e));
        }
        if let Err(e) = self.playback.poll() {
            self.error = Some(e.to_string());
        }
        self.apply_import_events();
        self.truncate_toast();
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            Self::fonts(ui.ctx());
            self.configured = true;
        }
        let (dropped, hovering) = ui.ctx().input(|input| {
            (
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_owned())
                    .collect::<Vec<_>>(),
                !input.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty()
            && let Err(error) = self.drop_files(dropped)
        {
            self.import_notice = Some(error_detail(&error));
        }
        Self::theme(ui.ctx(), self.preferences.dark);
        ui.style_mut().interaction.selectable_labels = false;
        self.widgets.clear();
        self.panels.clear();
        self.tick();
        self.keyboard(ui.ctx());
        let rect = ui.max_rect();
        let window_fill = if self.preferences.dark {
            Color32::from_rgb(15, 15, 15)
        } else {
            Color32::from_rgb(242, 243, 245)
        };
        ui.painter().rect_filled(rect, 0.0, window_fill);
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 52.0));
        let body = Rect::from_min_max(egui::pos2(rect.left(), top.bottom()), rect.max);
        let timeline_height = body.height() * self.preferences.panels[2];
        let upper = Rect::from_min_max(
            body.min,
            egui::pos2(body.right(), body.bottom() - timeline_height),
        );
        let timeline = Rect::from_min_max(egui::pos2(body.left(), upper.bottom()), body.max);
        let left_w = upper.width() * self.preferences.panels[0];
        let right_w = upper.width() * self.preferences.panels[1];
        let left = Rect::from_min_size(upper.min, Vec2::new(left_w, upper.height()));
        let right = Rect::from_min_max(egui::pos2(upper.right() - right_w, upper.top()), upper.max);
        let preview = Rect::from_min_max(
            egui::pos2(left.right(), upper.top()),
            egui::pos2(right.left(), upper.bottom()),
        );
        for (id, area) in [
            ("header", top),
            ("media", left),
            ("preview", preview),
            ("inspector", right),
            ("timeline", timeline),
        ] {
            self.panels.insert(id, area);
            let fill = if id == "header" {
                if self.preferences.dark {
                    Color32::from_rgb(19, 19, 19)
                } else {
                    Color32::WHITE
                }
            } else if self.preferences.dark {
                Color32::from_rgb(15, 15, 15)
            } else {
                Color32::from_rgb(249, 250, 251)
            };
            ui.painter().rect_filled(area, 0.0, fill);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(area.shrink(12.0)),
            );
            child.set_clip_rect(area);
            match id {
                "header" => self.header(&mut child),
                "media" => self.media(&mut child),
                "preview" => self.preview(&mut child),
                "inspector" => self.inspector(&mut child),
                _ => self.timeline(&mut child),
            }
        }
        let separator = if self.preferences.dark {
            Color32::from_rgb(31, 31, 31)
        } else {
            Color32::from_rgb(222, 222, 222)
        };
        for (id, area, axis) in [
            (
                "layout.media",
                Rect::from_center_size(
                    egui::pos2(left.right(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                0,
            ),
            (
                "layout.inspector",
                Rect::from_center_size(
                    egui::pos2(right.left(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                1,
            ),
            (
                "layout.timeline",
                Rect::from_center_size(
                    egui::pos2(timeline.center().x, timeline.top()),
                    Vec2::new(timeline.width(), 8.0),
                ),
                2,
            ),
        ] {
            let response = ui.interact(area, Id::unique(id), egui::Sense::drag());
            self.remember(id, &response);
            if response.dragged() {
                let delta = response.drag_delta();
                if let Some(v) = self.preferences.panels.get_mut(axis) {
                    *v = match axis {
                        0 => (*v + delta.x / upper.width()).clamp(0.15, 0.35),
                        1 => (*v - delta.x / upper.width()).clamp(0.15, 0.35),
                        _ => (*v - delta.y / body.height()).clamp(0.15, 0.7),
                    };
                }
            }
        }
        for (a, b) in [
            (top.left_bottom(), top.right_bottom()),
            (left.right_top(), left.right_bottom()),
            (right.left_top(), right.left_bottom()),
            (timeline.left_top(), timeline.right_top()),
        ] {
            ui.painter()
                .line_segment([a, b], egui::Stroke::new(1.0, separator));
        }
        self.import_overlay(ui.ctx());
        self.drop_overlay(ui.ctx(), hovering);
        self.toast(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if self.playback.playing() { 8 } else { 33 },
            ));
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.rename.is_some() {
            return;
        }
        // Most specific chords first: consume_key ignores extra Shift/Alt modifiers.
        let command_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        if consume(ctx, command_shift, egui::Key::Z)
            || consume(ctx, egui::Modifiers::COMMAND, egui::Key::Y)
        {
            self.action("edit.redo");
        }
        if consume(ctx, egui::Modifiers::COMMAND, egui::Key::Z) {
            self.action("edit.undo");
        }
        if consume(ctx, egui::Modifiers::COMMAND, egui::Key::B) {
            self.action("timeline.split");
        }
        if consume(ctx, egui::Modifiers::COMMAND, egui::Key::A) {
            self.action("timeline.select_all");
        }
        if consume(ctx, egui::Modifiers::SHIFT, egui::Key::Delete)
            || consume(ctx, egui::Modifiers::SHIFT, egui::Key::Backspace)
        {
            self.action("timeline.ripple_delete");
        } else if consume(ctx, egui::Modifiers::NONE, egui::Key::Delete)
            || consume(ctx, egui::Modifiers::NONE, egui::Key::Backspace)
        {
            self.action("timeline.delete");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::N) {
            self.action("timeline.snap");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
            self.cancel_interaction();
        }
        if consume(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowLeft) {
            self.action("transport.back");
        }
        if consume(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowRight) {
            self.action("transport.forward");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::ArrowLeft) {
            self.action("transport.frame_back");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::ArrowRight) {
            self.action("transport.frame_forward");
        }
        for (key, id) in [
            (egui::Key::Space, "transport.play"),
            (egui::Key::K, "transport.pause"),
            (egui::Key::J, "transport.back"),
            (egui::Key::L, "transport.forward"),
            (egui::Key::Home, "transport.start"),
            (egui::Key::End, "transport.end"),
        ] {
            if consume(ctx, egui::Modifiers::NONE, key) {
                self.action(id);
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("VibeCut").strong().size(18.0));
            ui.separator();
            if self.button(ui, "project.back", Text::Projects, true) {
                self.action("project.back");
            }
            if self.rename.is_some() {
                let mut name = self.rename.take().unwrap_or_default();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .id_salt("project.rename.input")
                        .desired_width(230.0),
                );
                self.remember("project.rename.input", &response);
                if self.rename_focus {
                    response.request_focus();
                    self.rename_focus = false;
                }
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if commit && !cancel {
                    if let Err(e) = self
                        .session
                        .execute("project.rename", serde_json::json!({"name": name}))
                    {
                        self.error = Some(e.to_string());
                        self.rename = Some(name);
                    }
                } else if !cancel {
                    self.rename = Some(name);
                }
            } else {
                let response = ui.add_sized(
                    [220.0, 26.0],
                    egui::Button::new(self.session.project().data().name.as_str()).truncate(),
                );
                self.remember("project.rename", &response);
                if response.clicked() {
                    self.action("project.rename");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.button(ui, "file.export", Text::Export, false);
                if self.button(ui, "project.save_as", Text::SaveAs, true) {
                    self.action("project.save_as");
                }
                if self.button(ui, "project.save", Text::Save, true) {
                    self.action("project.save");
                }
                if self.button(ui, "appearance.theme", Text::Theme, true) {
                    self.action("appearance.theme");
                }
                if self.button(ui, "appearance.locale", Text::Language, true) {
                    self.action("appearance.locale");
                }
                if self.button(ui, "layout.reset", Text::Reset, true) {
                    self.action("layout.reset");
                }
            });
        });
    }
    fn media(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Media))
                .size(16.0)
                .strong(),
        );
        ui.add_space(8.0);
        let importing = self.importing();
        ui.horizontal(|ui| {
            if self.button(ui, "file.import", Text::Import, !importing) {
                self.action("file.import");
            }
            if self.button(
                ui,
                "media.insert",
                Text::AddToTimeline,
                !importing && !self.media_selection.is_empty(),
            ) {
                self.action("media.insert");
            }
        });
        if !importing && !self.media_selection.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{}: {}",
                    self.tr(Text::Selected),
                    self.media_selection.len()
                ))
                .small()
                .color(self.secondary()),
            );
        }
        ui.add_space(12.0);
        let locale = self.preferences.locale;
        let rows: Vec<AssetRow> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| asset_row(asset, locale))
            .collect();
        if rows.is_empty() {
            ui.label(self.tr(Text::EmptyMedia));
        } else {
            let selected = self.media_selection.clone();
            egui::ScrollArea::vertical()
                .id_salt("media.list")
                .max_height((ui.available_height() - 120.0).max(80.0))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!importing, |ui| {
                        for row in &rows {
                            let is_selected = selected.contains(&row.id);
                            let response = ui
                                .push_id(("media.asset", row.id.0), |ui| {
                                    ui.add_sized(
                                        [ui.available_width(), 46.0],
                                        egui::Button::new(egui::RichText::new(format!(
                                            "{}\n{}",
                                            row.name, row.info
                                        )))
                                        .selected(is_selected)
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                    )
                                })
                                .inner;
                            self.remember(format!("media.asset.{}", row.id.0), &response);
                            if response.clicked() {
                                let modifiers = ui.ctx().input(|input| input.modifiers);
                                self.select_asset(row.id, modifiers);
                            }
                        }
                    });
                });
            if self.media_selection.is_empty() && !importing {
                ui.label(
                    egui::RichText::new(self.tr(Text::SelectAssets))
                        .small()
                        .color(self.secondary()),
                );
            }
        }
        if !self.import_failures.is_empty() {
            ui.separator();
            ui.label(
                egui::RichText::new(self.tr(Text::ImportIssues))
                    .size(14.0)
                    .strong(),
            );
            for failure in &self.import_failures {
                ui.label(
                    egui::RichText::new(format!(
                        "{}: {}",
                        file_name(&failure.path),
                        failure.reason
                    ))
                    .small()
                    .color(self.secondary()),
                );
            }
        }
        if let Some(notice) = &self.import_notice {
            ui.separator();
            ui.label(egui::RichText::new(notice).small().color(self.secondary()));
        }
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.button(ui, "inspector.project", Text::Project, true) {
                self.action("inspector.project");
            }
            if self.button(ui, "inspector.details", Text::Details, true) {
                self.action("inspector.details");
            }
        });
        ui.separator();
        ui.add_space(32.0);
        if self.details {
            let data = self.session.project().data();
            ui.label(format!("{} × {}", data.canvas.width, data.canvas.height));
            ui.label(format!(
                "{} / {} fps",
                data.canvas.fps.num(),
                data.canvas.fps.den()
            ));
            ui.label(format!("{} Hz", data.settings.audio_sample_rate));
        } else {
            ui.label(self.tr(Text::EmptyInspector));
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Preview))
                .size(16.0)
                .strong(),
        );
        ui.add_space(12.0);
        let mut footer_height = 86.0;
        if !self.playback.device_available() {
            footer_height += 24.0;
        }
        if self.error.is_some() || self.playback.error().is_some() {
            footer_height += 80.0;
        }
        let area = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(
                ui.available_width(),
                (ui.available_height() - footer_height).max(0.0),
            ),
        );
        ui.painter().rect_filled(area, 0.0, Color32::BLACK);
        if let Some(frame) = self
            .playback
            .frame()
            .filter(|_| self.session.project().duration() > Tick::ZERO)
        {
            let key = (frame.epoch, frame.at);
            if self.texture_key != Some(key) {
                if let Some(texture) = &mut self.texture {
                    texture.set(frame.image.clone(), egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ui.ctx().load_texture(
                        "preview.frame",
                        frame.image.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.texture_key = Some(key);
                self.texture_owner = Some(frame.clone());
            }
            if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let size = size * (area.width() / size.x).min(area.height() / size.y);
                ui.painter().image(
                    texture.id(),
                    Rect::from_center_size(area.center(), size),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            self.texture = None;
            self.texture_key = None;
            self.texture_owner = None;
            let label = if self.playback.error().is_some() {
                Text::Unavailable
            } else if self.session.project().duration() == Tick::ZERO {
                Text::EmptyPreview
            } else {
                Text::Loading
            };
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::centered_and_justified(
                        egui::Direction::TopDown,
                    )),
                |ui| {
                    ui.label(
                        egui::RichText::new(self.tr(label))
                            .size(14.0)
                            .color(Color32::from_rgb(229, 229, 229)),
                    );
                },
            );
        }
        ui.advance_cursor_after_rect(area);
        let ready = self.session.project().duration() > Tick::ZERO && self.playback.can_play();
        ui.horizontal(|ui| {
            if self.button(ui, "transport.back", Text::Back, true) {
                self.action("transport.back");
            }
            if self.button(
                ui,
                "transport.play",
                if self.playback.playing() {
                    Text::Pause
                } else {
                    Text::Play
                },
                ready,
            ) {
                self.action("transport.play");
            }
            if self.button(ui, "transport.forward", Text::Forward, true) {
                self.action("transport.forward");
            }
        });
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        ui.label(format!(
            "{} / {}",
            display_time(at),
            display_time(self.session.project().duration())
        ));
        if !self.playback.device_available() {
            ui.label(self.tr(Text::Offline));
        }
        if self.error.is_some() || self.playback.error().is_some() {
            ui.label(self.tr(Text::Unavailable));
            if self.button(ui, "preview.retry", Text::Retry, true) {
                self.error = None;
                self.action("preview.retry");
            }
        }
    }
    /// Non-modal import overlay over the media panel. Import controls are
    /// already disabled while the batch runs; cancel remains available.
    fn import_overlay(&mut self, ctx: &egui::Context) {
        if !self.importing() {
            return;
        }
        let rect = self
            .panels
            .get("media")
            .copied()
            .unwrap_or_else(|| Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 200.0)));
        let percent = self
            .import_completed
            .saturating_mul(100)
            .checked_div(self.import_total)
            .unwrap_or(0)
            .min(100);
        egui::Area::new(Id::unique("import.overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min + Vec2::new(12.0, 12.0))
            .show(ctx, |ui| {
                ui.set_max_width((rect.width() - 24.0).clamp(220.0, 360.0));
                egui::Frame::new()
                    .fill(self.panel_fill())
                    .stroke(egui::Stroke::new(1.0, self.border()))
                    .corner_radius(12)
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::Importing))
                                .size(16.0)
                                .strong(),
                        );
                        ui.add_space(10.0);
                        let (bar, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 2.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(bar, 0.0, self.control_fill());
                        let filled = Rect::from_min_size(
                            bar.min,
                            Vec2::new(bar.width() * percent as f32 / 100.0, bar.height()),
                        );
                        ui.painter().rect_filled(filled, 0.0, self.accent());
                        ui.label(format!("{} {percent}%", self.tr(Text::Processing)));
                        ui.label(
                            egui::RichText::new(format!(
                                "{}: {} / {}",
                                self.tr(Text::Imported),
                                self.import_completed,
                                self.import_total
                            ))
                            .small()
                            .color(self.secondary()),
                        );
                        if self.button(ui, "import.cancel", Text::Cancel, true) {
                            self.action("import.cancel");
                        }
                    });
            });
    }
    fn drop_overlay(&self, ctx: &egui::Context, hovering: bool) {
        if !hovering || self.importing() {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(Id::unique("import.drop"))
            .order(egui::Order::Tooltip)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_width(screen.width());
                ui.set_height(screen.height());
                ui.painter().rect_filled(
                    screen,
                    0.0,
                    Color32::from_rgba_unmultiplied(0, 0, 0, 170),
                );
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(screen.shrink(48.0)).layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    ),
                    |ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::DropMedia))
                                .size(18.0)
                                .color(Color32::WHITE),
                        );
                    },
                );
            });
    }
    fn toast(&self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        let text = toast.text.clone();
        let detail = toast.detail.clone();
        egui::Area::new(Id::unique("import.toast"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.panel_fill())
                    .stroke(egui::Stroke::new(1.0, self.border()))
                    .corner_radius(12)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_max_width(320.0);
                        let response = ui.label(egui::RichText::new(text).size(14.0).strong());
                        response.ctx.accesskit_node_builder(response.id, |node| {
                            node.set_author_id("import.toast");
                        });
                        if let Some(detail) = detail {
                            ui.label(egui::RichText::new(detail).small().color(self.secondary()));
                        }
                    });
            });
    }
    fn timeline(&mut self, ui: &mut egui::Ui) {
        let layout = TimelineLayout::new(ui.max_rect());
        self.timeline_toolbar(ui, &layout);
        self.timeline_scroll_input(ui, &layout);
        let (rows, items, duration, bookmarks) = self.timeline_views(&layout);
        self.paint_ruler(ui, &layout, bookmarks.as_slice());
        self.paint_tracks(ui, &layout, &rows, &items);
        self.advance_drag(ui, layout.body.left());
        self.paint_playhead(ui, &layout);
        self.timeline_playhead_interaction(ui, &layout);
        self.timeline_scrollbars(ui, &layout, duration, &rows);
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
        if items.is_empty() && !rows.is_empty() {
            let empty = self.tr(Text::EmptyTimeline);
            ui.painter().text(
                layout.body.center(),
                Align2::CENTER_CENTER,
                empty,
                FontId::proportional(14.0),
                self.secondary(),
            );
        }
    }
    fn timeline_toolbar(&mut self, ui: &mut egui::Ui, layout: &TimelineLayout) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("timeline.toolbar")
                .max_rect(layout.toolbar.shrink2(Vec2::new(4.0, 7.0))),
        );
        child.set_clip_rect(layout.toolbar);
        child.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::Timeline))
                    .size(16.0)
                    .strong(),
            );
            let can_undo = self.session.can_undo();
            let can_redo = self.session.can_redo();
            if self.button(ui, "edit.undo", Text::Undo, can_undo) {
                self.action("edit.undo");
            }
            if self.button(ui, "edit.redo", Text::Redo, can_redo) {
                self.action("edit.redo");
            }
            let has_items = self
                .session
                .project()
                .data()
                .timeline
                .tracks
                .iter()
                .any(|track| !track.items.is_empty());
            let has_selection = !self.selection.is_empty();
            if self.button(ui, "timeline.split", Text::Split, has_items) {
                self.action("timeline.split");
            }
            if self.button(ui, "timeline.delete", Text::Delete, has_selection) {
                self.action("timeline.delete");
            }
            if self.toggle_button(ui, "timeline.snap", Text::Snap, self.snapping) {
                self.action("timeline.snap");
            }
            if self.toggle_button(ui, "timeline.ripple", Text::Ripple, self.ripple) {
                self.action("timeline.ripple");
            }
            if self.button(ui, "timeline.zoom_out", Text::ZoomOut, true) {
                self.action("timeline.zoom_out");
            }
            if self.button(ui, "timeline.zoom_in", Text::ZoomIn, true) {
                self.action("timeline.zoom_in");
            }
        });
    }
    fn timeline_scroll_input(&mut self, ui: &egui::Ui, layout: &TimelineLayout) {
        if self.drag.is_some() {
            return;
        }
        let hovered = ui.rect_contains_pointer(layout.body)
            || ui.rect_contains_pointer(layout.ruler)
            || ui.rect_contains_pointer(layout.labels);
        if !hovered {
            return;
        }
        let (scroll, zoom, modifiers) = ui.ctx().input(|input| {
            (
                input.smooth_scroll_delta(),
                input.zoom_delta(),
                input.modifiers,
            )
        });
        if (zoom - 1.0).abs() > 0.01 {
            let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
            let before = self.zoom;
            let anchor =
                pointer.map(|pointer| self.scroll + (pointer.x - layout.body.left()) / before);
            self.zoom_by(zoom);
            if let (Some(pointer), Some(anchor)) = (pointer, anchor) {
                self.scroll = (anchor - (pointer.x - layout.body.left()) / self.zoom).max(0.0);
            }
        } else if modifiers.shift || scroll.x.abs() > scroll.y.abs() {
            let delta = if modifiers.shift { scroll.y } else { scroll.x };
            self.scroll = (self.scroll - delta / self.zoom).max(0.0);
        } else {
            self.vertical_scroll = (self.vertical_scroll - scroll.y).max(0.0);
        }
    }
    fn timeline_views(
        &mut self,
        layout: &TimelineLayout,
    ) -> (Vec<TrackRow>, Vec<ItemView>, Tick, Vec<Tick>) {
        let (rows, raw_items, duration, bookmarks, content_height) = {
            let data = self.session.project().data();
            let mut rows = Vec::new();
            let mut raw_items = Vec::new();
            let mut offset = 0.0;
            for (row_index, track) in data.timeline.tracks.iter().enumerate() {
                let height = track_height(track.kind);
                rows.push(TrackRow {
                    kind: track.kind,
                    name: display_label(track.name.as_str(), 48),
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
                    ));
                }
                offset += height + TRACK_GAP;
            }
            (
                rows,
                raw_items,
                self.session.project().duration(),
                data.bookmarks
                    .iter()
                    .map(|bookmark| bookmark.start)
                    .collect(),
                (offset - TRACK_GAP).max(0.0),
            )
        };
        self.vertical_scroll = self
            .vertical_scroll
            .clamp(0.0, (content_height - layout.body.height()).max(0.0));
        let mut items = Vec::new();
        for (row_index, id, kind, start, end, locked, label, overlay) in raw_items {
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
            });
        }
        let view_seconds = (layout.body.width() / self.zoom).max(0.001);
        let max_scroll = (duration.seconds() as f32 + 1.0 - view_seconds).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        (rows, items, duration, bookmarks)
    }
    fn paint_ruler(&mut self, ui: &egui::Ui, layout: &TimelineLayout, bookmarks: &[Tick]) {
        let painter = ui.painter().with_clip_rect(layout.ruler);
        painter.rect_filled(layout.ruler, 0.0, self.control_fill());
        painter.rect_filled(layout.corner, 0.0, self.control_fill());
        let step = ruler_step(self.zoom);
        let start = self.scroll;
        let end = self.scroll + layout.ruler.width() / self.zoom;
        let first = (start / step).floor() * step;
        let mut second = first;
        let mut guard = 0;
        while second <= end && guard < 512 {
            let x = layout.ruler.left() + (second - self.scroll) * self.zoom;
            painter.line_segment(
                [
                    egui::pos2(x, layout.ruler.bottom() - 6.0),
                    egui::pos2(x, layout.ruler.bottom()),
                ],
                egui::Stroke::new(1.0, self.border()),
            );
            if let Ok(at) = Tick::from_seconds_f64(f64::from(second), Rounding::Nearest) {
                painter.text(
                    egui::pos2(x + 4.0, layout.ruler.top() + 2.0),
                    Align2::LEFT_TOP,
                    display_time(at),
                    FontId::proportional(10.0),
                    self.secondary(),
                );
            }
            second += step;
            guard += 1;
        }
        for bookmark in bookmarks {
            let x = self.x_at(*bookmark, layout.ruler.left());
            if x >= layout.ruler.left() && x <= layout.ruler.right() {
                painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(x - 2.0, layout.ruler.bottom() - 5.0),
                        Vec2::new(4.0, 5.0),
                    ),
                    1.0,
                    self.accent(),
                );
            }
        }
        let response = ui.interact(
            layout.ruler,
            Id::unique("timeline.ruler"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.ruler", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = self.frame_snap(self.tick_at(pointer.x, layout.ruler.left()));
            self.seek_to(at);
        }
    }
    fn paint_tracks(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        items: &[ItemView],
    ) {
        let body_painter = ui.painter().with_clip_rect(layout.body);
        let label_painter = ui.painter().with_clip_rect(layout.labels);
        label_painter.rect_filled(layout.labels, 0.0, self.panel_fill());
        body_painter.rect_filled(layout.body, 0.0, self.panel_fill());
        for (index, row) in rows.iter().enumerate() {
            let top = layout.body.top() + row.top - self.vertical_scroll;
            let label_rect = Rect::from_min_size(
                egui::pos2(layout.labels.left(), top),
                Vec2::new(layout.labels.width(), row.height),
            );
            let row_rect = Rect::from_min_size(
                egui::pos2(layout.body.left(), top),
                Vec2::new(layout.body.width(), row.height),
            );
            let shade = if index % 2 == 0 {
                self.panel_fill()
            } else {
                self.control_fill()
            };
            label_painter.rect_filled(label_rect, 0.0, shade);
            body_painter.rect_filled(row_rect, 0.0, shade);
            label_painter.text(
                egui::pos2(label_rect.left() + 8.0, label_rect.center().y),
                Align2::LEFT_CENTER,
                &row.name,
                FontId::proportional(12.0),
                self.text_color(),
            );
            label_painter.text(
                egui::pos2(label_rect.right() - 8.0, label_rect.center().y),
                Align2::RIGHT_CENTER,
                self.tr(kind_label(row.kind)),
                FontId::proportional(10.0),
                self.secondary(),
            );
        }
        let modifiers = ui.ctx().input(|input| input.modifiers);
        for item in items {
            self.paint_item(ui, layout, item, modifiers);
        }
        if let Some(at) = self.snap_indicator {
            let x = self.x_at(at, layout.body.left());
            if x >= layout.body.left() && x <= layout.body.right() {
                ui.painter().with_clip_rect(layout.body).line_segment(
                    [
                        egui::pos2(x, layout.body.top()),
                        egui::pos2(x, layout.body.bottom()),
                    ],
                    egui::Stroke::new(1.0, self.accent()),
                );
            }
        }
    }
    fn paint_item(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        modifiers: egui::Modifiers,
    ) {
        let x0 = self.x_at(item.start, layout.body.left());
        let x1 = self.x_at(item.end, layout.body.left());
        let full = Rect::from_min_max(
            egui::pos2(x0, item.top + 2.0),
            egui::pos2(x1, item.top + item.height - 2.0),
        );
        let visible = full.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let selected = self.selection.contains(&item.id);
        let response = ui.interact(
            visible,
            Id::unique(("timeline.item", item.id.0)),
            if item.locked {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        self.remember(format!("timeline.item.{}", item.id.0), &response);
        let (fill, border, text) =
            item_palette(item.kind, self.preferences.dark, selected, item.overlay);
        let painter = ui.painter().with_clip_rect(visible);
        painter.rect_filled(visible, 0.0, fill);
        painter.rect_stroke(
            visible,
            0.0,
            egui::Stroke::new(if selected { 2.0 } else { 1.0 }, border),
            egui::StrokeKind::Inside,
        );
        match item.kind {
            TrackKind::Video => {
                let mut x = visible.left() + 4.0;
                let mut index = 0;
                while x + 10.0 < visible.right() && index < 256 {
                    let shade = if index % 2 == 0 { 12 } else { 6 };
                    painter.rect_filled(
                        Rect::from_min_size(
                            egui::pos2(x, visible.top() + 3.0),
                            Vec2::new(9.0, visible.height() - 6.0),
                        ),
                        0.0,
                        Color32::from_rgba_unmultiplied(0, 193, 205, shade),
                    );
                    x += 13.0;
                    index += 1;
                }
            }
            TrackKind::Audio => {
                let center = visible.center().y;
                let bars = ((visible.width() / 4.0).floor() as usize).min(256);
                for index in 0..bars {
                    let t = index as f32 / bars.max(1) as f32;
                    let scale = ((t * 47.0 + item.id.0 as f32).sin() * 0.5 + 0.5) * 0.8 + 0.1;
                    let half = (visible.height() * 0.35 * scale).max(1.0);
                    let x = visible.left() + index as f32 * 4.0 + 2.0;
                    painter.line_segment(
                        [egui::pos2(x, center - half), egui::pos2(x, center + half)],
                        egui::Stroke::new(1.0, text),
                    );
                }
            }
            _ => {}
        }
        painter.text(
            egui::pos2(visible.left() + 6.0, visible.center().y),
            Align2::LEFT_CENTER,
            &item.label,
            FontId::proportional(11.0),
            text,
        );
        if !item.locked && visible.width() > 16.0 {
            let width = 8.0_f32.min(visible.width() / 3.0);
            let left = Rect::from_min_max(
                visible.left_top(),
                egui::pos2(visible.left() + width, visible.bottom()),
            );
            let right = Rect::from_min_max(
                egui::pos2(visible.right() - width, visible.top()),
                visible.right_bottom(),
            );
            for (id, rect, mode) in [
                ("trim_start", left, DragMode::TrimStart),
                ("trim_end", right, DragMode::TrimEnd),
            ] {
                let response = ui.interact(
                    rect,
                    Id::unique(("timeline.item.handle", item.id.0, id)),
                    egui::Sense::click_and_drag(),
                );
                self.remember(format!("timeline.item.{}.{id}", item.id.0), &response);
                ui.painter().with_clip_rect(rect).line_segment(
                    [rect.center_top(), rect.center_bottom()],
                    egui::Stroke::new(
                        2.0,
                        if selected || response.hovered() {
                            self.accent()
                        } else {
                            border
                        },
                    ),
                );
                if response.clicked() {
                    self.select_item(item.id, modifiers);
                }
                if response.drag_started() && self.drag.is_none() {
                    self.begin_drag(item.id, mode, ui);
                }
            }
        }
        if response.clicked() {
            self.select_item(item.id, modifiers);
        }
        if !item.locked && self.drag.is_none() && response.drag_started() {
            self.begin_drag(item.id, DragMode::Move, ui);
        }
    }
    fn paint_playhead(&self, ui: &egui::Ui, layout: &TimelineLayout) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 2.0 || x > layout.body.right() + 2.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(layout.ruler.left(), layout.ruler.top()),
            egui::pos2(layout.ruler.right(), layout.body.bottom()),
        ));
        painter.line_segment(
            [
                egui::pos2(x, layout.ruler.bottom()),
                egui::pos2(x, layout.body.bottom()),
            ],
            egui::Stroke::new(2.0, self.accent()),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - 5.0, layout.ruler.bottom() - 8.0),
                egui::pos2(x + 5.0, layout.ruler.bottom() - 8.0),
                egui::pos2(x, layout.ruler.bottom()),
            ],
            self.accent(),
            egui::Stroke::NONE,
        ));
    }
    fn timeline_playhead_interaction(&mut self, ui: &egui::Ui, layout: &TimelineLayout) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 6.0 || x > layout.body.right() + 6.0 {
            return;
        }
        // Keep the playhead draggable from the ruler so item automation IDs stay
        // clickable when the line crosses a clip.
        let rect = Rect::from_min_max(
            egui::pos2(x - 6.0, layout.ruler.top()),
            egui::pos2(x + 6.0, layout.ruler.bottom()),
        );
        let response = ui.interact(
            rect,
            Id::unique("timeline.playhead"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.playhead", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = self.frame_snap(self.tick_at(pointer.x, layout.body.left()));
            self.seek_to(at);
        }
    }
    fn timeline_scrollbars(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        duration: Tick,
        rows: &[TrackRow],
    ) {
        let view = (layout.body.width() / self.zoom).max(0.001);
        let content = (duration.seconds() as f32).max(view) + 1.0;
        let max = (content - view).max(0.0);
        let track = layout.horizontal.shrink2(Vec2::new(0.0, 3.0));
        let thumb_width = ((view / content) * track.width()).clamp(12.0, track.width());
        let fraction = if max > 0.0 { self.scroll / max } else { 0.0 };
        let thumb_x = track.left() + fraction * (track.width() - thumb_width);
        let thumb = Rect::from_min_size(
            egui::pos2(thumb_x, track.top()),
            Vec2::new(thumb_width, track.height()),
        );
        ui.painter()
            .rect_filled(layout.horizontal, 0.0, self.panel_fill());
        ui.painter().rect_filled(track, 6.0, self.control_fill());
        ui.painter().rect_filled(thumb, 6.0, self.border());
        let response = ui.interact(
            layout.horizontal,
            Id::unique("timeline.scroll_h"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.scroll_h", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && max > 0.0
        {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            self.scroll = fraction * max;
        }
        let content_height: f32 = rows.iter().map(|row| row.height + TRACK_GAP).sum::<f32>();
        let max_vertical = (content_height - TRACK_GAP - layout.body.height()).max(0.0);
        let track = layout.vertical.shrink2(Vec2::new(3.0, 0.0));
        let thumb_height = if content_height > 0.0 {
            ((layout.body.height() / content_height) * track.height()).clamp(12.0, track.height())
        } else {
            track.height()
        };
        let fraction = if max_vertical > 0.0 {
            self.vertical_scroll / max_vertical
        } else {
            0.0
        };
        let thumb_y = track.top() + fraction * (track.height() - thumb_height);
        ui.painter()
            .rect_filled(layout.vertical, 0.0, self.panel_fill());
        ui.painter().rect_filled(track, 6.0, self.control_fill());
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(track.left(), thumb_y),
                Vec2::new(track.width(), thumb_height),
            ),
            6.0,
            self.border(),
        );
        let response = ui.interact(
            layout.vertical,
            Id::unique("timeline.scroll_v"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.scroll_v", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && max_vertical > 0.0
        {
            let fraction = ((pointer.y - track.top()) / track.height()).clamp(0.0, 1.0);
            self.vertical_scroll = fraction * max_vertical;
        }
    }
    fn frame_snap(&self, at: Tick) -> Tick {
        self.session
            .project()
            .data()
            .canvas
            .fps
            .snap_nearest(at)
            .unwrap_or(at)
    }
    fn accent(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(0, 193, 205)
        } else {
            Color32::from_rgb(0, 106, 116)
        }
    }
    fn panel_fill(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(15, 15, 15)
        } else {
            Color32::from_rgb(249, 250, 251)
        }
    }
    fn control_fill(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(38, 38, 38)
        } else {
            Color32::from_rgb(238, 239, 240)
        }
    }
    fn border(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(56, 56, 56)
        } else {
            Color32::from_rgb(133, 139, 145)
        }
    }
    fn text_color(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(229, 229, 229)
        } else {
            Color32::from_rgb(28, 28, 28)
        }
    }
    fn secondary(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(154, 154, 154)
        } else {
            Color32::from_rgb(92, 98, 104)
        }
    }
}
fn asset_kind(asset: &SourceAsset) -> std::result::Result<TrackKind, engine::EditError> {
    if asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Video)
    {
        Ok(TrackKind::Video)
    } else if asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Audio)
    {
        Ok(TrackKind::Audio)
    } else {
        Err(engine::EditError::Unsupported(
            "media has no video or audio stream",
        ))
    }
}
fn asset_row(asset: &SourceAsset, locale: Locale) -> AssetRow {
    let mut info = Vec::new();
    let video = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref());
    let audio = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.audio.as_ref());
    let kind = if video.is_some() {
        Text::Video
    } else if audio.is_some() {
        Text::Audio
    } else {
        Text::Media
    };
    info.push(catalog::text(locale, kind).to_owned());
    info.push(asset.probe().duration.map_or_else(
        || catalog::text(locale, Text::DurationUnknown).to_owned(),
        display_time,
    ));
    if let Some(video) = video {
        info.push(format!("{}×{}", video.width, video.height));
    }
    if let Some(rate) = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
    {
        info.push(if rate.den() == 1 {
            format!("{} fps", rate.num())
        } else {
            format!("{:.3} fps", rate.num() as f64 / rate.den() as f64)
        });
    }
    if let Some(audio) = audio {
        info.push(format!("{} Hz · {} ch", audio.sample_rate, audio.channels));
    }
    AssetRow {
        id: asset.id(),
        name: file_name(asset.path()),
        info: info.join(" · "),
    }
}
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}
fn error_detail(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    for _ in 0..8 {
        let Some(cause) = source else {
            break;
        };
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}
fn consume(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| input.consume_key(modifiers, key))
}
fn track_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => 65.0,
        TrackKind::Audio => 50.0,
        TrackKind::Text | TrackKind::Caption | TrackKind::Overlay => 25.0,
    }
}
fn kind_label(kind: TrackKind) -> Text {
    match kind {
        TrackKind::Video => Text::Video,
        TrackKind::Audio => Text::Audio,
        TrackKind::Text => Text::TextItem,
        TrackKind::Caption => Text::Caption,
        TrackKind::Overlay => Text::Overlay,
    }
}
fn item_label(content: &ItemParams, locale: Locale, dark: bool) -> (String, Option<[u8; 4]>) {
    match content {
        ItemParams::Video(_) => (catalog::text(locale, Text::Video).to_owned(), None),
        ItemParams::Audio(_) => (catalog::text(locale, Text::Audio).to_owned(), None),
        ItemParams::Text(params) => {
            let value = params.content.as_str();
            if value.is_empty() {
                (catalog::text(locale, Text::TextItem).to_owned(), None)
            } else {
                (display_label(value, LABEL_CHARS), None)
            }
        }
        ItemParams::Caption(_) => (catalog::text(locale, Text::Caption).to_owned(), None),
        ItemParams::Overlay(params) => {
            let [r, g, b, _] = params.color.0;
            let color = if dark { [r, g, b, 90] } else { [r, g, b, 60] };
            (catalog::text(locale, Text::Overlay).to_owned(), Some(color))
        }
    }
}
fn display_label(value: &str, limit: usize) -> String {
    let mut label: String = value.chars().take(limit.saturating_add(1)).collect();
    if label.chars().count() > limit {
        label.pop();
        label.push('…');
    }
    label
}
fn item_palette(
    kind: TrackKind,
    dark: bool,
    selected: bool,
    overlay: Option<[u8; 4]>,
) -> (Color32, Color32, Color32) {
    let (fill, border, text) = if dark {
        match kind {
            TrackKind::Video => (
                Color32::from_rgb(10, 42, 36),
                Color32::from_rgb(11, 107, 91),
                Color32::from_rgb(229, 229, 229),
            ),
            TrackKind::Audio => (
                Color32::from_rgb(26, 32, 38),
                Color32::from_rgb(56, 56, 56),
                Color32::from_rgb(229, 229, 229),
            ),
            _ => (
                Color32::from_rgb(38, 38, 42),
                Color32::from_rgb(56, 56, 56),
                Color32::from_rgb(229, 229, 229),
            ),
        }
    } else {
        match kind {
            TrackKind::Video => (
                Color32::from_rgb(227, 247, 242),
                Color32::from_rgb(11, 107, 91),
                Color32::from_rgb(28, 28, 28),
            ),
            TrackKind::Audio => (
                Color32::from_rgb(238, 239, 240),
                Color32::from_rgb(133, 139, 145),
                Color32::from_rgb(28, 28, 28),
            ),
            _ => (
                Color32::from_rgb(238, 239, 240),
                Color32::from_rgb(133, 139, 145),
                Color32::from_rgb(28, 28, 28),
            ),
        }
    };
    let fill = overlay.map_or(fill, |[r, g, b, a]| {
        Color32::from_rgba_unmultiplied(r, g, b, a)
    });
    if selected {
        let accent = if dark {
            Color32::from_rgb(0, 193, 205)
        } else {
            Color32::from_rgb(0, 106, 116)
        };
        (fill, accent, text)
    } else {
        (fill, border, text)
    }
}
fn ruler_step(zoom: f32) -> f32 {
    for step in [
        0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0,
    ] {
        if step * zoom >= 70.0 {
            return step;
        }
    }
    3600.0
}
fn display_time(at: Tick) -> String {
    let s = at.seconds().max(0.0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (s / 3600.0) as u64,
        (s / 60.0) as u64 % 60,
        s as u64 % 60,
        ((s.fract() * 1000.0).floor()) as u64
    )
}
