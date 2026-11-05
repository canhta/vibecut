use crate::selection::{SelectionScope, SelectionScopes, shared_fields};
use crate::{
    Error, Result,
    catalog::{self, Locale, Preferences, Text, drag_panel},
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    icons::{self, Icon},
    import::{ImportEvent, ImportFailure, ImportProbe, ImportQueue},
    playback::Playback,
    theme,
};
use engine::{Command, CreatedId, ItemParamPatch, NewItem, Session};
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
const TOOLBAR_SPACING: f32 = 4.0;
const BUTTON_ICON: f32 = 16.0;
const ICON_BUTTON: f32 = 28.0;
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

/// App action id for every preference edit; engine history is untouched.
pub const PREFERENCE_COMMAND: &str = "ui.prefs.set";

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub id: String,
    pub enabled: bool,
    pub focused: bool,
    pub rect: Rect,
    /// Embedded Phosphor name for a button that carries an icon.
    pub icon: Option<&'static str>,
    /// Action and shortcut shown for icon-only controls.
    pub tooltip: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub widgets: Vec<WidgetState>,
    pub view: &'static str,
    pub playing: bool,
    pub position: Tick,
    pub selection: Vec<ItemId>,
    pub selection_scope: SelectionScope,
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
    pub export: Option<crate::export::ExportInspection>,
    pub export_options: crate::export::ExportOptions,
}
#[derive(Clone, Debug)]
struct PreferenceCommand {
    key: String,
    before: serde_json::Value,
    after: serde_json::Value,
}
impl PreferenceCommand {
    fn new(key: &str, before: serde_json::Value, after: serde_json::Value) -> Self {
        Self {
            key: key.to_owned(),
            before,
            after,
        }
    }
}
/// App-owned preference history, deliberately separate from the Project's
/// command history: `ui.prefs.set` edits never mutate Project JSON.
#[derive(Default)]
struct PreferenceHistory {
    undo: Vec<PreferenceCommand>,
    redo: Vec<PreferenceCommand>,
}
impl PreferenceHistory {
    fn record(&mut self, command: PreferenceCommand) {
        self.undo.push(command);
        self.redo.clear();
    }
    fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    fn undo(&mut self, preferences: &mut Preferences) -> bool {
        let Some(command) = self.undo.pop() else {
            return false;
        };
        if preferences
            .set(&command.key, command.before.clone())
            .is_err()
        {
            self.undo.push(command);
            return false;
        }
        self.redo.push(command);
        true
    }
    fn redo(&mut self, preferences: &mut Preferences) -> bool {
        let Some(command) = self.redo.pop() else {
            return false;
        };
        if preferences
            .set(&command.key, command.after.clone())
            .is_err()
        {
            self.redo.push(command);
            return false;
        }
        self.undo.push(command);
        true
    }
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
    icon: Icon,
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
    id: TrackId,
    main: bool,
    muted: bool,
    hidden: bool,
    locked: bool,
    empty: bool,
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
        let toolbar = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), TOOLBAR_HEIGHT.min(rect.height())),
        );
        let top = toolbar.bottom();
        let label_right = (rect.left() + LABEL_WIDTH).min(rect.right());
        let ruler_bottom = (top + RULER_HEIGHT).min(rect.bottom());
        let right = (rect.right() - SCROLLBAR).max(label_right);
        let bottom = (rect.bottom() - SCROLLBAR).max(ruler_bottom);
        let ruler = Rect::from_min_max(
            egui::pos2(label_right, top),
            egui::pos2(right, ruler_bottom),
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
    preference_history: PreferenceHistory,
    pub selection: BTreeSet<ItemId>,
    pub selection_scopes: SelectionScopes,
    marquee: Option<(egui::Pos2, BTreeSet<ItemId>, bool)>,
    property_drafts: BTreeMap<&'static str, String>,
    property_selection: BTreeSet<ItemId>,
    pub snapping: bool,
    pub ripple: bool,
    zoom: f32,
    scroll: f32,
    vertical_scroll: f32,
    last_view: project::ViewState,
    widgets: BTreeMap<String, WidgetState>,
    pub panels: BTreeMap<&'static str, Rect>,
    layout_drag: Option<[f32; 3]>,
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
    scenes_open: bool,
    scene_select: bool,
    scene_selected: BTreeSet<project::SceneId>,
    scene_confirm: bool,
    pub export: Option<crate::export::ExportDialog>,
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
        let selection = session.selection().clone();
        Self {
            session,
            playback,
            preferences,
            preference_history: PreferenceHistory::default(),
            selection,
            selection_scopes: SelectionScopes::default(),
            marquee: None,
            property_drafts: BTreeMap::new(),
            property_selection: BTreeSet::new(),
            snapping: true,
            ripple: false,
            zoom,
            scroll,
            vertical_scroll: 0.0,
            last_view: view,
            widgets: BTreeMap::new(),
            panels: BTreeMap::new(),
            layout_drag: None,
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
            scenes_open: false,
            scene_select: false,
            scene_selected: BTreeSet::new(),
            scene_confirm: false,
            export: None,
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
            selection_scope: self.selection_scopes.active(&self.selection),
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
                    let name = failure.path.file_name().map_or_else(
                        || failure.path.to_string_lossy().into_owned(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    catalog::format(
                        self.preferences.locale,
                        Text::FailureDetail,
                        &[("name", &name), ("reason", &failure.reason)],
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
            export_options: self.preferences.export,
            export: self
                .export
                .as_ref()
                .map(crate::export::ExportDialog::inspect),
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
        let failed_count = self.import_failures.len();
        let unfinished_count = self.import_total.saturating_sub(self.import_completed);
        let locale = self.preferences.locale;
        let imported = catalog::number(locale, self.import_imported as u64);
        let failed = catalog::number(locale, failed_count as u64);
        let unfinished = catalog::number(locale, unfinished_count as u64);
        let detail = catalog::format(
            locale,
            Text::ImportSummary,
            &[
                ("imported", &imported),
                ("failed", &failed),
                ("unfinished", &unfinished),
            ],
        );
        let text = if self.import_error.is_some() {
            self.tr(Text::ImportFailed)
        } else if self.import_cancelled {
            self.tr(Text::ImportCancelled)
        } else if failed_count > 0 || unfinished_count > 0 {
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
        crate::selection::click(
            &mut self.media_selection,
            &mut self.media_anchor,
            id,
            modifiers.command || modifiers.ctrl,
            modifiers.shift.then_some(ids.as_slice()),
        );
        self.session.end_merge();
    }
    /// Insert selected assets in library order; engine owns track and overlap policy.
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
            let mut video_at = at;
            let mut audio_at = at;
            for asset in &selected {
                let kind = asset_kind(asset)?;
                let duration = asset
                    .probe()
                    .duration
                    .filter(|duration| *duration > Tick::ZERO)
                    .ok_or(engine::EditError::Unsupported(
                        "media duration is unavailable",
                    ))?;
                let item = NewItem {
                    animations: Default::default(),
                    source_asset: Some(asset.id()),
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start: if kind == TrackKind::Video {
                        video_at
                    } else {
                        audio_at
                    },
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
                let outcome = session.apply(Command::InsertItem {
                    placement: engine::Placement::Auto,
                    item,
                })?;
                let placed = outcome
                    .placements
                    .first()
                    .ok_or(engine::EditError::Invalid("item placement"))?;
                let end = session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .find(|track| track.id == placed.track)
                    .and_then(|track| track.items.iter().find(|item| item.id == placed.item))
                    .ok_or(engine::EditError::NoItem(placed.item))?
                    .end()?;
                if kind == TrackKind::Video {
                    video_at = end;
                } else {
                    audio_at = end;
                }
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
        self.remember_with_icon(id, None, response);
    }
    fn remember_with_icon(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        response: &egui::Response,
    ) {
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
                icon,
                tooltip: None,
            },
        );
    }
    fn button(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        icon: Icon,
        key: Text,
        enabled: bool,
    ) -> bool {
        let color = if enabled {
            self.text_color()
        } else {
            theme::disabled(self.preferences.dark)
        };
        let image = icons::tinted(icon, BUTTON_ICON, color);
        let label = egui::RichText::new(self.tr(key)).color(color);
        let response = ui
            .push_id(id, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::image_and_text(image, label).min_size(Vec2::new(28.0, 26.0)),
                )
            })
            .inner;
        self.remember_with_icon(id, Some(icon.name()), &response);
        response.clicked()
    }
    /// Text-only is reserved for the properties tabs by the iconography contract.
    fn text_button(&mut self, ui: &mut egui::Ui, id: &'static str, key: Text) -> bool {
        let response = ui.push_id(id, |ui| ui.button(self.tr(key))).inner;
        self.remember(id, &response);
        response.clicked()
    }
    /// A property section whose open state lives in the preference store, with
    /// the same stable automation id across locale and layout changes.
    fn section<R>(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: &'static str,
        label: Text,
        default_open: bool,
        body: impl FnOnce(&mut Self, &mut egui::Ui) -> R,
    ) -> Option<R> {
        let state_id = ui.make_persistent_id(("preferences.section", key));
        let stored = self.preferences.section_open(key, default_open);
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            state_id,
            default_open,
        );
        if state.is_open() != stored {
            state.set_open(stored);
            state.store(ui.ctx());
        }
        let label = self.tr(label);
        let mut clicked = false;
        let mut header = state.show_header(ui, |ui| {
            let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
            clicked = response.clicked();
        });
        if clicked {
            header.toggle();
        }
        let open = header.is_open();
        let (toggle, _, body_response) = header.body(|ui| body(self, ui));
        if open != stored
            && let Err(error) = self.set_section_open(key, open)
        {
            self.error = Some(error.to_string());
        }
        toggle
            .ctx
            .accesskit_node_builder(toggle.id, |node| node.set_author_id(id));
        self.remember(id, &toggle);
        body_response.map(|response| response.inner)
    }
    #[allow(clippy::too_many_arguments)]
    fn icon_only(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        icon: Icon,
        label: &str,
        shortcut: Option<&str>,
        selected: Option<bool>,
        enabled: bool,
    ) -> bool {
        let locale = self.preferences.locale;
        let tooltip = shortcut.map_or_else(
            || label.to_owned(),
            |key| {
                catalog::format(
                    locale,
                    Text::ActionShortcut,
                    &[("action", label), ("shortcut", key)],
                )
            },
        );
        let is_selected = selected.unwrap_or(false);
        let (rect, response) = ui
            .push_id(id, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.allocate_exact_size(Vec2::splat(ICON_BUTTON), egui::Sense::click())
                })
                .inner
            })
            .inner;
        let color = if !enabled {
            theme::disabled(self.preferences.dark)
        } else if is_selected {
            self.accent()
        } else {
            self.text_color()
        };
        if enabled && (response.hovered() || response.has_focus()) {
            ui.painter()
                .rect_filled(rect, 4.0, theme::field(self.preferences.dark));
        }
        icons::tinted(icon, BUTTON_ICON, color).paint_at(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
        );
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &tooltip)
            } else {
                egui::WidgetInfo::labeled(egui::Role::Button, enabled, &tooltip)
            }
        });
        let response = response.on_hover_text(&tooltip);
        self.remember_with_icon(id, Some(icon.name()), &response);
        if let Some(state) = self.widgets.get_mut(id) {
            state.tooltip = Some(tooltip);
        }
        enabled && response.clicked()
    }
    /// All control inputs (including tests/agents) use the same action path.
    /// Project edits are engine registry calls; transport/preferences are app state.
    pub fn activate(&mut self, id: &str) -> Result<()> {
        self.session.set_selection(self.selection.clone())?;
        self.remember_view()?;
        let at = self.playback.position()?;
        let duration = self.session.project().duration();
        let playing = self.playback.playing();
        let before = self.session.project().clone();
        match id {
            "scene.open" => self.scenes_open = true,
            "scene.close" => {
                self.scenes_open = false;
                self.scene_confirm = false;
            }
            "scene.select" => {
                self.scene_select = !self.scene_select;
                self.scene_selected.clear();
            }
            "scene.delete" => self.scene_confirm = !self.scene_selected.is_empty(),
            "scene.cancel_delete" => self.scene_confirm = false,
            "scene.confirm_delete" => {
                self.session.execute(
                    "scene.delete",
                    serde_json::json!({"scenes":self.scene_selected}),
                )?;
                self.scene_selected.clear();
                self.scene_select = false;
                self.scene_confirm = false;
                self.selection.clear();
                self.selection_scopes = SelectionScopes::default();
            }
            row if row.starts_with("scene.row.") => {
                let scene = project::SceneId(
                    row.trim_start_matches("scene.row.")
                        .parse()
                        .map_err(|_| Error::Unsupported("scene identity"))?,
                );
                let target = self
                    .session
                    .project()
                    .data()
                    .scenes
                    .get(scene)
                    .ok_or(Error::Unsupported("scene not found"))?;
                if self.scene_select {
                    if !target.is_main && !self.scene_selected.remove(&scene) {
                        self.scene_selected.insert(scene);
                    }
                } else {
                    self.cancel_interaction();
                    self.session
                        .execute("scene.switch", serde_json::json!({"scene":scene}))?;
                    self.selection.clear();
                    self.selection_scopes = SelectionScopes::default();
                }
            }
            "file.export" => {
                if self.export.is_none() {
                    self.export = Some(crate::export::ExportDialog::new(
                        self.playback.budget().clone(),
                        self.playback.render_state()?,
                    )?);
                }
                if let Some(dialog) = &mut self.export {
                    dialog.show(self.session.project());
                }
            }
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
                    self.import_notice = Some(self.tr(Text::ImportBusy).to_owned());
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
                let preference_changed = if id == "edit.undo" {
                    self.preference_history.undo(&mut self.preferences)
                } else {
                    self.preference_history.redo(&mut self.preferences)
                };
                if !preference_changed {
                    self.session.execute(
                        if id == "edit.undo" { "undo" } else { "redo" },
                        serde_json::json!({}),
                    )?;
                    self.selection = self.session.selection().clone();
                    self.selection_scopes = SelectionScopes::default();
                    self.property_drafts.clear();
                }
            }
            "timeline.split" => self.split_at_playhead()?,
            "timeline.delete" => self.delete_selected(self.ripple)?,
            "timeline.ripple_delete" => self.delete_selected(true)?,
            "timeline.select_all" => self.select_all(),
            "timeline.deselect_all" => {
                self.selection.clear();
                self.selection_scopes = SelectionScopes::default();
            }
            "selection.escape" => self.cancel_interaction(),
            "timeline.snap" => self.snapping = !self.snapping,
            "timeline.ripple" => self.ripple = !self.ripple,
            "timeline.zoom_in" => self.zoom_by(ZOOM_STEP),
            "timeline.zoom_out" => self.zoom_by(1.0 / ZOOM_STEP),
            "appearance.theme" => {
                let after = !self.preferences.dark;
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "dark", "value": after}),
                )?;
            }
            "appearance.locale" => {
                let after = if self.preferences.locale == Locale::En {
                    Locale::Vi
                } else {
                    Locale::En
                };
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "locale", "value": after}),
                )?;
            }
            "layout.reset" => {
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({
                        "key": "panels",
                        "value": Preferences::default().panels,
                    }),
                )?;
            }
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
                let item = ItemId(parsed);
                if !self
                    .session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .any(|t| t.items.iter().any(|i| i.id == item))
                {
                    return Err(Error::Engine(engine::EditError::NoItem(item)));
                }
                self.select_single(item);
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
    /// App action dispatcher shared by UI and agents. Preference edits use
    /// `ui.prefs.set` with `{"key": ..., "value": ...}` typed params.
    pub fn dispatch(&mut self, id: &str, params: serde_json::Value) -> Result<()> {
        if id == PREFERENCE_COMMAND {
            let key = params
                .get("key")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid("preference key"))?;
            let value = params
                .get("value")
                .cloned()
                .ok_or(Error::Invalid("preference value"))?;
            return self.set_preference(key, value);
        }
        self.activate(id)
    }
    /// One command (`ui.prefs.set`) for every user-visible preference edit.
    /// App history is separate from Project history and never changes Project JSON.
    pub fn set_preference(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let before = self.preferences.get(key)?;
        self.preferences.set(key, value)?;
        let after = self.preferences.get(key)?;
        if before != after {
            self.preference_history
                .record(PreferenceCommand::new(key, before, after));
        }
        Ok(())
    }
    /// Section keys are dynamic; the whole open/closed map is one history entry.
    pub fn set_section_open(&mut self, key: &str, open: bool) -> Result<()> {
        let mut sections = self.preferences.sections.clone();
        sections.insert(key.to_owned(), open);
        self.dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "sections", "value": sections}),
        )
    }
    fn record_layout_change(&mut self, before: [f32; 3]) {
        let after = self.preferences.panels;
        self.preferences.panels = before;
        if let Err(error) = self.dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "panels", "value": after}),
        ) {
            self.error = Some(error.to_string());
        }
    }
    fn select_single(&mut self, item: ItemId) {
        self.selection.clear();
        self.selection.insert(item);
        self.selection_changed();
    }
    fn select_item(&mut self, item: ItemId, modifiers: egui::Modifiers) {
        crate::selection::click(
            &mut self.selection,
            &mut None,
            item,
            modifiers.command || modifiers.ctrl || modifiers.shift,
            None,
        );
        self.selection_changed();
    }
    fn select_all(&mut self) {
        self.selection = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection_changed();
    }
    fn selection_changed(&mut self) {
        self.selection_scopes = SelectionScopes::default();
        self.property_drafts.clear();
        if let Err(e) = self.session.set_selection(self.selection.clone()) {
            self.error = Some(e.to_string());
        }
    }
    fn prune_selection(&mut self) {
        let existing: BTreeSet<ItemId> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection.retain(|id| existing.contains(id));
        self.selection_scopes
            .mask_points
            .retain(|target| self.selection.contains(&target.item));
        self.selection_scopes
            .keyframes
            .retain(|target| self.selection.contains(&target.item));
    }
    fn split_at_playhead(&mut self) -> Result<()> {
        let at = self.playback.position()?;
        let mut under = Vec::new();
        let mut selected_under = Vec::new();
        for track in self.session.project().data().scene().timeline.iter() {
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
            .scene()
            .timeline
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
        if let Err(error) = self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.begin_gesture())
        {
            self.error = Some(error.to_string());
            return;
        }
        let mut origins = Vec::new();
        let mut source_range = None;
        for track in self.session.project().data().scene().timeline.iter() {
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
    fn advance_drag(&mut self, ui: &egui::Ui, layout: &TimelineLayout, rows: &[TrackRow]) {
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
            self.update_drag(pointer, layout.body.left(), bypass);
        }
        if !down {
            if let Some(pointer) = pointer
                && let Err(error) = self.drop_track(pointer, layout, rows, bypass)
            {
                self.error = Some(error.to_string());
                self.cancel_interaction();
                return;
            }
            self.finish_drag();
        }
    }
    fn drop_track(
        &mut self,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        bypass: bool,
    ) -> Result<()> {
        let Some(drag) = self.drag.clone() else {
            return Ok(());
        };
        if drag.mode != DragMode::Move || drag.origins.len() != 1 || !layout.body.contains(pointer)
        {
            return Ok(());
        }
        let Some(origin) = drag.origins.first() else {
            return Ok(());
        };
        let mut destination = None;
        for row in rows {
            let top = layout.body.top() + row.top - self.vertical_scroll;
            let bottom = top + row.height;
            if pointer.y < top + 3.0 {
                destination = Some((
                    row.id,
                    Some(engine::TrackPlacement::Above { track: row.id }),
                ));
                break;
            }
            if pointer.y <= bottom - 3.0 {
                destination = Some((row.id, None));
                break;
            }
            destination = Some((
                row.id,
                Some(engine::TrackPlacement::Below { track: row.id }),
            ));
            if pointer.y <= bottom + TRACK_GAP / 2.0 {
                break;
            }
        }
        let Some((target, placement)) = destination else {
            return Ok(());
        };
        if placement.is_none() && target == origin.track {
            return Ok(());
        }
        let raw = Tick::from_seconds_f64(
            f64::from(pointer.x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (delta, _) = self.snap_delta(raw, &drag.origins, bypass);
        let start = origin
            .start
            .checked_add(delta)
            .ok_or(engine::EditError::Invalid("time overflow"))?
            .max(Tick::ZERO);
        let target = if let Some(placement) = placement {
            let kind = self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .find(|t| t.id == origin.track)
                .ok_or(engine::EditError::NoTrack(origin.track))?
                .kind;
            let outcome = self.session.apply(Command::AddTrack {
                name: format!("{kind:?}").into(),
                kind,
                placement,
            })?;
            match outcome.created.first() {
                Some(CreatedId::Track(id)) => *id,
                _ => return Err(engine::EditError::Invalid("track allocation").into()),
            }
        } else {
            target
        };
        self.session.apply(Command::MoveItem {
            item: drag.item,
            track: target,
            start,
        })?;
        if let Some(drag) = &mut self.drag {
            drag.moved = true;
        }
        Ok(())
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
        } else if let Some((_, before, _)) = self.marquee.take() {
            self.selection = before;
            self.selection_changed();
        } else {
            self.selection_scopes.clear_specific(&mut self.selection);
            if let Err(e) = self.session.set_selection(self.selection.clone()) {
                self.error = Some(e.to_string());
            }
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
        for track in self.session.project().data().scene().timeline.iter() {
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
        for track in self.session.project().data().scene().timeline.iter() {
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
            .scene()
            .timeline
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
            data.scene()
                .bookmarks
                .iter()
                .flat_map(|bookmark| [bookmark.start, bookmark.start + bookmark.duration]),
        );
        points.extend(
            data.scene()
                .timeline
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
        let text = theme::text(dark);
        let border = theme::border(dark);
        let field = theme::field(dark);
        let hover = theme::hover(dark);
        let separator = theme::separator(dark);
        let stroke = |color| egui::Stroke::new(1.0, color);
        visuals.panel_fill = theme::panel(dark);
        visuals.window_fill = theme::modal(dark);
        visuals.window_stroke = stroke(border);
        visuals.extreme_bg_color = field;
        visuals.faint_bg_color = separator;
        visuals.code_bg_color = field;
        visuals.warn_fg_color = theme::warning(dark);
        visuals.error_fg_color = theme::error_text(dark);
        visuals.hyperlink_color = theme::accent(dark);
        visuals.override_text_color = Some(text);
        visuals.selection.bg_fill = theme::accent(dark);
        visuals.selection.stroke = stroke(theme::on_accent(dark));
        visuals.text_cursor.stroke = stroke(theme::accent(dark));
        visuals.widgets.noninteractive.bg_fill = theme::panel(dark);
        visuals.widgets.noninteractive.weak_bg_fill = theme::panel(dark);
        visuals.widgets.noninteractive.bg_stroke = stroke(separator);
        visuals.widgets.noninteractive.fg_stroke = stroke(theme::secondary(dark));
        visuals.widgets.inactive.bg_fill = field;
        visuals.widgets.inactive.weak_bg_fill = field;
        visuals.widgets.inactive.bg_stroke = stroke(border);
        visuals.widgets.inactive.fg_stroke = stroke(text);
        visuals.widgets.hovered.bg_fill = hover;
        visuals.widgets.hovered.weak_bg_fill = hover;
        visuals.widgets.hovered.bg_stroke = stroke(border);
        visuals.widgets.hovered.fg_stroke = stroke(text);
        visuals.widgets.active.bg_fill = hover;
        visuals.widgets.active.weak_bg_fill = hover;
        visuals.widgets.active.bg_stroke = stroke(theme::accent(dark));
        visuals.widgets.active.fg_stroke = stroke(text);
        visuals.widgets.open.bg_fill = hover;
        visuals.widgets.open.weak_bg_fill = hover;
        visuals.widgets.open.bg_stroke = stroke(border);
        visuals.widgets.open.fg_stroke = stroke(text);
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
        if let Some(dialog) = &mut self.export
            && let Err(error) = dialog.tick(self.session.project(), self.preferences.export)
        {
            dialog.failed(error);
        }
        self.apply_import_events();
        self.truncate_toast();
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.session.transaction_active()
            && let Err(e) = self.session.set_selection(self.selection.clone())
        {
            self.error = Some(e.to_string());
        }
        if !self.configured {
            Self::fonts(ui.ctx());
            icons::install(ui.ctx());
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
        if !self.export.as_ref().is_some_and(|d| d.open) {
            self.keyboard(ui.ctx());
        }
        let rect = ui.max_rect();
        let rect = Rect::from_min_size(rect.min, rect.size().max(Vec2::ZERO));
        ui.painter()
            .rect_filled(rect, 0.0, theme::window(self.preferences.dark));
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), rect.height().min(52.0)));
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
                theme::window(self.preferences.dark)
            } else {
                theme::panel(self.preferences.dark)
            };
            ui.painter().rect_filled(area, 0.0, fill);
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(
                area.shrink2(Vec2::new(
                    12.0_f32.min(area.width() / 2.0),
                    12.0_f32.min(area.height() / 2.0),
                )),
            ));
            child.set_clip_rect(area);
            if area.width() <= 0.0 || area.height() <= 0.0 {
                continue;
            }
            match id {
                "header" => self.header(&mut child),
                "media" => self.media(&mut child),
                "preview" => self.preview(&mut child),
                "inspector" => self.inspector(&mut child),
                _ => self.timeline(&mut child),
            }
        }
        let separator = theme::separator(self.preferences.dark);
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
            if response.drag_started() {
                self.layout_drag = Some(self.preferences.panels);
            }
            if response.dragged() {
                let delta = response.drag_delta();
                let fraction = match axis {
                    0 | 1 => delta.x / upper.width(),
                    _ => delta.y / body.height(),
                };
                self.preferences.panels = drag_panel(self.preferences.panels, axis, fraction);
            }
            if response.drag_stopped()
                && let Some(before) = self.layout_drag.take()
            {
                self.record_layout_change(before);
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
        self.scenes_sheet(ui.ctx());
        self.import_overlay(ui.ctx());
        self.drop_overlay(ui.ctx(), hovering);
        self.toast(ui.ctx());
        self.export_dialog(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if self.playback.playing() { 8 } else { 33 },
            ));
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.scenes_open {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                if self.scene_confirm {
                    self.scene_confirm = false;
                } else {
                    self.scenes_open = false;
                }
            }
            return;
        }
        if self.marquee.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.cancel_interaction();
            }
            return;
        }
        if self.rename.is_some()
            || ctx.memory(|m| m.focused()).is_some_and(|id| {
                [
                    "inspector.audio_gain",
                    "inspector.opacity",
                    "inspector.text_size",
                    "inspector.text_color",
                ]
                .iter()
                .any(|field| id == Id::unique(field))
            })
        {
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
    /// Accept the native export picker's result without opening OS UI in tests.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        self.export
            .as_mut()
            .ok_or(Error::Invalid("export dialog closed"))?
            .set_destination(path)
    }
    fn export_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.export else {
            return;
        };
        let mut options = self.preferences.export;
        let output = dialog.ui(
            ctx,
            self.preferences.locale,
            self.preferences.dark,
            &mut options,
            self.session.project(),
            self.texture.as_ref(),
        );
        match output.action {
            Some(crate::export::Action::Pick) => {
                self.lifecycle_action = Some(crate::lifecycle::EditorAction::ExportDestination)
            }
            Some(crate::export::Action::Start) => {
                let result = if self.session.transaction_active() {
                    Err(Error::Engine(engine::EditError::TransactionActive))
                } else {
                    dialog.start(self.session.project().clone(), options)
                };
                if let Err(error) = result {
                    dialog.failed(error);
                }
            }
            None => {}
        }
        if options != self.preferences.export
            && let Err(error) = self.dispatch(
                PREFERENCE_COMMAND,
                serde_json::json!({"key": "export", "value": options}),
            )
        {
            self.error = Some(error.to_string());
        }
        for (id, response) in output.widgets {
            self.remember(id, &response);
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::AppName))
                    .strong()
                    .size(18.0),
            );
            ui.separator();
            if self.button(ui, "project.back", Icon::ArrowLeft, Text::Projects, true) {
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
                let image = icons::tinted(Icon::PencilSimple, BUTTON_ICON, self.text_color());
                let response = ui.add_sized(
                    [220.0, 26.0],
                    egui::Button::image_and_text(
                        image,
                        egui::RichText::new(self.session.project().data().name.as_str()),
                    )
                    .truncate(),
                );
                self.remember_with_icon(
                    "project.rename",
                    Some(Icon::PencilSimple.name()),
                    &response,
                );
                if response.clicked() {
                    self.action("project.rename");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.button(ui, "file.export", Icon::Export, Text::Export, true) {
                    self.action("file.export");
                }
                if self.button(
                    ui,
                    "project.save_as",
                    Icon::FloppyDiskBack,
                    Text::SaveAs,
                    true,
                ) {
                    self.action("project.save_as");
                }
                if self.button(ui, "project.save", Icon::FloppyDisk, Text::Save, true) {
                    self.action("project.save");
                }
                if self.button(ui, "appearance.theme", Icon::CircleHalf, Text::Theme, true) {
                    self.action("appearance.theme");
                }
                if self.button(
                    ui,
                    "appearance.locale",
                    Icon::Translate,
                    Text::Language,
                    true,
                ) {
                    self.action("appearance.locale");
                }
                if self.button(ui, "layout.reset", Icon::CornersOut, Text::Reset, true) {
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
            if self.button(
                ui,
                "file.import",
                Icon::UploadSimple,
                Text::Import,
                !importing,
            ) {
                self.action("file.import");
            }
            if self.button(
                ui,
                "media.insert",
                Icon::Plus,
                Text::AddToTimeline,
                !importing && !self.media_selection.is_empty(),
            ) {
                self.action("media.insert");
            }
        });
        if !importing && !self.media_selection.is_empty() {
            ui.label(
                egui::RichText::new(catalog::format_count(
                    self.preferences.locale,
                    self.media_selection.len() as u64,
                    Text::SelectedCountOne,
                    Text::SelectedCountOther,
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
                            let color = if is_selected {
                                theme::on_accent(self.preferences.dark)
                            } else {
                                self.text_color()
                            };
                            let image = icons::tinted(row.icon, BUTTON_ICON, color);
                            let mut button = egui::Button::image_and_text(
                                image,
                                egui::RichText::new(format!("{}\n{}", row.name, row.info))
                                    .color(color),
                            )
                            .selected(is_selected)
                            .wrap_mode(egui::TextWrapMode::Wrap);
                            if is_selected {
                                button = button.fill(self.accent());
                            }
                            let response = ui
                                .push_id(("media.asset", row.id.0), |ui| {
                                    ui.add_sized([ui.available_width(), 46.0], button)
                                })
                                .inner;
                            self.remember_with_icon(
                                format!("media.asset.{}", row.id.0),
                                Some(row.icon.name()),
                                &response,
                            );
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
                let name = file_name(&failure.path);
                ui.label(
                    egui::RichText::new(catalog::format(
                        self.preferences.locale,
                        Text::FailureDetail,
                        &[("name", &name), ("reason", &failure.reason)],
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
            if self.text_button(ui, "inspector.project", Text::Project) {
                self.action("inspector.project");
            }
            if self.text_button(ui, "inspector.details", Text::Details) {
                self.action("inspector.details");
            }
        });
        ui.separator();
        ui.add_space(8.0);
        if self.details {
            self.section(
                ui,
                "inspector.section.canvas",
                "inspector.canvas",
                Text::Canvas,
                true,
                |editor, ui| {
                    let locale = editor.preferences.locale;
                    let data = editor.session.project().data();
                    let width = catalog::number(locale, u64::from(data.canvas.width));
                    let height = catalog::number(locale, u64::from(data.canvas.height));
                    ui.label(catalog::format(
                        locale,
                        Text::CanvasSize,
                        &[("width", &width), ("height", &height)],
                    ));
                    let numerator = catalog::signed(locale, data.canvas.fps.num());
                    let denominator = catalog::signed(locale, data.canvas.fps.den());
                    ui.label(catalog::format(
                        locale,
                        Text::FrameRate,
                        &[("num", &numerator), ("den", &denominator)],
                    ));
                    let rate = catalog::number(locale, u64::from(data.settings.audio_sample_rate));
                    ui.label(catalog::format(
                        locale,
                        Text::SampleRate,
                        &[("rate", &rate)],
                    ));
                },
            );
            self.section(
                ui,
                "inspector.section.metadata",
                "inspector.metadata",
                Text::Metadata,
                false,
                |editor, ui| {
                    let locale = editor.preferences.locale;
                    let created = catalog::date(locale, editor.session.metadata().created_unix_ms);
                    let modified =
                        catalog::date(locale, editor.session.metadata().modified_unix_ms);
                    ui.label(
                        egui::RichText::new(catalog::format(
                            locale,
                            Text::CreatedDate,
                            &[("date", &created)],
                        ))
                        .small()
                        .color(editor.secondary()),
                    );
                    ui.label(
                        egui::RichText::new(catalog::format(
                            locale,
                            Text::ModifiedDate,
                            &[("date", &modified)],
                        ))
                        .small()
                        .color(editor.secondary()),
                    );
                },
            );
        } else if self.selection.is_empty() {
            ui.label(self.tr(Text::EmptyInspector));
        } else {
            self.section(
                ui,
                "inspector.section.selection",
                "inspector.selection",
                Text::Selection,
                true,
                |editor, ui| editor.shared_inspector(ui),
            );
        }
    }
    fn shared_inspector(&mut self, ui: &mut egui::Ui) {
        if self.property_selection != self.selection {
            self.property_drafts.clear();
            self.property_selection = self.selection.clone();
        }
        ui.label(catalog::format_count(
            self.preferences.locale,
            self.selection.len() as u64,
            Text::SelectedCountOne,
            Text::SelectedCountOther,
        ));
        match self.selection_scopes.active(&self.selection) {
            SelectionScope::MaskPoints => {
                ui.label(self.tr(Text::MaskPoints));
            }
            SelectionScope::Keyframes => {
                ui.label(self.tr(Text::Keyframes));
            }
            _ => {}
        }
        let fields = shared_fields(self.session.project(), &self.selection);
        if fields.is_empty() {
            ui.label(self.tr(Text::NoSharedFields));
        }
        for field in fields {
            if field.id == "blendMode" {
                ui.add_space(12.0);
                let caption = ui.label(self.tr(Text::BlendMode));
                let current = field.value.as_ref().and_then(|v| {
                    serde_json::from_value::<project::BlendMode>(serde_json::json!(v)).ok()
                });
                let label = current
                    .map(|m| self.tr(catalog::blend_label(m)))
                    .unwrap_or_else(|| self.tr(Text::EmptyValue));
                let response = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = 26.0;
                        egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                            .selected_text(label)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for mode in project::BlendMode::ALL {
                                    let response = ui.selectable_label(
                                        current == Some(mode),
                                        self.tr(catalog::blend_label(mode)),
                                    );
                                    self.remember(
                                        format!("inspector.blendMode.{}", mode.id()),
                                        &response,
                                    );
                                    if response.clicked() {
                                        if let Err(error) =
                                            self.set_shared_field("blendMode", mode.id())
                                        {
                                            self.error = Some(error.to_string());
                                        }
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .labelled_by(caption.id)
                    })
                    .inner;
                self.remember("inspector.blendMode", &response);
                continue;
            }
            let (label, id) = match field.id {
                "opacity" => (Text::Opacity, "inspector.opacity"),
                "audio_gain" => (Text::AudioGain, "inspector.audio_gain"),
                "text_size" => (Text::TextSize, "inspector.text_size"),
                _ => (Text::TextColor, "inspector.text_color"),
            };
            ui.add_space(12.0);
            let caption = ui.label(self.tr(label));
            let mut value = self
                .property_drafts
                .get(field.id)
                .cloned()
                .unwrap_or_else(|| field.value.clone().unwrap_or_default());
            let response = ui
                .add_sized(
                    [
                        ui.available_width(),
                        if field.id == "opacity" { 26.0 } else { 36.0 },
                    ],
                    egui::TextEdit::singleline(&mut value)
                        .char_limit(64)
                        .background_color(self.control_fill())
                        .id(Id::unique(id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
                .labelled_by(caption.id);
            self.remember(id, &response);
            if response.changed() {
                self.property_drafts.insert(field.id, value.clone());
            }
            let compatible = catalog::number(self.preferences.locale, field.compatible as u64);
            let total = catalog::number(self.preferences.locale, self.selection.len() as u64);
            ui.label(
                egui::RichText::new(catalog::format(
                    self.preferences.locale,
                    Text::CompatibleCount,
                    &[("compatible", &compatible), ("total", &total)],
                ))
                .small()
                .color(self.secondary()),
            );
            if (response.lost_focus()
                || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                && self.property_drafts.contains_key(field.id)
            {
                let result = self.set_shared_field(field.id, &value);
                self.property_drafts.remove(field.id);
                if let Err(e) = result {
                    self.error = Some(e.to_string());
                }
            }
        }
    }
    /// The inspector and automation use the same registered atomic field edit.
    pub fn set_shared_field(&mut self, field: &str, value: &str) -> Result<()> {
        let patch = match field {
            "blendMode" => ItemParamPatch::BlendMode(
                serde_json::from_value(serde_json::json!(value))
                    .map_err(|_| Error::Invalid("blend mode"))?,
            ),
            "opacity" => ItemParamPatch::Opacity(
                value
                    .trim()
                    .parse()
                    .map_err(|_| Error::Invalid("opacity"))?,
            ),
            "audio_gain" => ItemParamPatch::AudioGain(
                value
                    .trim()
                    .parse()
                    .map_err(|_| Error::Invalid("audio gain"))?,
            ),
            "text_size" => ItemParamPatch::TextSize(
                value
                    .trim()
                    .parse()
                    .map_err(|_| Error::Invalid("text size"))?,
            ),
            "text_color" => {
                let value = value.trim().trim_start_matches('#');
                if !matches!(value.len(), 6 | 8) || !value.is_ascii() {
                    return Err(Error::Invalid("RGBA colour"));
                }
                let mut rgba = [255u8; 4];
                for (i, byte) in rgba.iter_mut().enumerate().take(value.len() / 2) {
                    *byte = u8::from_str_radix(
                        value
                            .get(i * 2..i * 2 + 2)
                            .ok_or(Error::Invalid("RGBA colour"))?,
                        16,
                    )
                    .map_err(|_| Error::Invalid("RGBA colour"))?;
                }
                ItemParamPatch::TextColor(project::Color(rgba))
            }
            _ => return Err(Error::Invalid("shared field")),
        };
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "timeline.set_item_param_many",
            serde_json::json!({"items":self.selection,"patch":patch}),
        )?;
        self.sync_preview();
        Ok(())
    }
    /// Shared transport and preview surface for dedicated review windows.
    pub fn preview_ui(&mut self, ui: &mut egui::Ui) {
        self.preview(ui);
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
        ui.painter()
            .rect_filled(area, 0.0, theme::preview_backdrop());
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
                    theme::image_tint(),
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
                            .color(theme::on_scrim()),
                    );
                },
            );
        }
        ui.advance_cursor_after_rect(area);
        let ready = self.session.project().duration() > Tick::ZERO && self.playback.can_play();
        ui.horizontal(|ui| {
            if self.icon_only(
                ui,
                "transport.back",
                Icon::SkipBack,
                self.tr(Text::Back),
                Some(self.tr(Text::ShortcutBack)),
                None,
                true,
            ) {
                self.action("transport.back");
            }
            let (play_icon, play_text, play_key) = if self.playback.playing() {
                (Icon::Pause, Text::Pause, Text::ShortcutPause)
            } else {
                (Icon::Play, Text::Play, Text::ShortcutPlay)
            };
            if self.icon_only(
                ui,
                "transport.play",
                play_icon,
                self.tr(play_text),
                Some(self.tr(play_key)),
                None,
                ready,
            ) {
                self.action("transport.play");
            }
            if self.icon_only(
                ui,
                "transport.forward",
                Icon::SkipForward,
                self.tr(Text::Forward),
                Some(self.tr(Text::ShortcutForward)),
                None,
                true,
            ) {
                self.action("transport.forward");
            }
        });
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        ui.label(catalog::format(
            self.preferences.locale,
            Text::TimePosition,
            &[
                ("current", &display_time(at)),
                ("duration", &display_time(self.session.project().duration())),
            ],
        ));
        if !self.playback.device_available() {
            ui.label(self.tr(Text::Offline));
        }
        if self.error.is_some() || self.playback.error().is_some() {
            ui.label(self.tr(Text::Unavailable));
            if self.icon_only(
                ui,
                "preview.retry",
                Icon::ArrowClockwise,
                self.tr(Text::Retry),
                None,
                None,
                true,
            ) {
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
                        ui.label(catalog::format(
                            self.preferences.locale,
                            Text::ProcessingPercent,
                            &[
                                ("status", self.tr(Text::Processing)),
                                (
                                    "percent",
                                    &catalog::number(self.preferences.locale, percent as u64),
                                ),
                            ],
                        ));
                        ui.label(
                            egui::RichText::new(catalog::format(
                                self.preferences.locale,
                                Text::ImportProgress,
                                &[
                                    (
                                        "done",
                                        &catalog::number(
                                            self.preferences.locale,
                                            self.import_completed as u64,
                                        ),
                                    ),
                                    (
                                        "total",
                                        &catalog::number(
                                            self.preferences.locale,
                                            self.import_total as u64,
                                        ),
                                    ),
                                ],
                            ))
                            .small()
                            .color(self.secondary()),
                        );
                        if self.button(ui, "import.cancel", Icon::X, Text::Cancel, true) {
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
                ui.painter().rect_filled(screen, 0.0, theme::scrim());
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(screen.shrink(48.0)).layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    ),
                    |ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::DropMedia))
                                .size(18.0)
                                .color(theme::on_scrim()),
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
        self.advance_drag(ui, &layout, &rows);
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
    fn scenes_sheet(&mut self, ctx: &egui::Context) {
        if !self.scenes_open {
            return;
        }
        let scenes: Vec<_> = self
            .session
            .project()
            .data()
            .scenes
            .iter()
            .map(|s| (s.id, s.name.as_str().to_owned(), s.is_main))
            .collect();
        let active = self.session.project().data().current_scene_id;
        let sheet_id = Id::unique("scenes.sheet");
        let sheet = egui::Modal::new(sheet_id)
            .area(egui::Modal::default_area(sheet_id).anchor(egui::Align2::RIGHT_TOP, Vec2::ZERO))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.set_min_height((ctx.content_rect().height() - 16.0).max(200.0));
                ui.heading(self.tr(if self.scene_select {
                    Text::SelectScenes
                } else {
                    Text::Scenes
                }));
                ui.label(self.tr(if self.scene_select {
                    Text::SelectScenesHint
                } else {
                    Text::SwitchScenesHint
                }));
                ui.horizontal(|ui| {
                    let response = ui.button(self.tr(if self.scene_select {
                        Text::CancelSelection
                    } else {
                        Text::Select
                    }));
                    self.remember("scene.select", &response);
                    if response.clicked() {
                        self.action("scene.select");
                    }
                    let response = ui.button(self.tr(Text::CloseScenes));
                    self.remember("scene.close", &response);
                    if response.clicked() {
                        self.action("scene.close");
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (id, name, main) in scenes {
                            let selected = if self.scene_select {
                                self.scene_selected.contains(&id)
                            } else {
                                active == id
                            };
                            let label = if main {
                                format!("{name} ({})", self.tr(Text::MainScene))
                            } else {
                                name
                            };
                            let response = ui.add_enabled(
                                !self.scene_select || !main,
                                egui::Button::new(label)
                                    .truncate()
                                    .selected(selected)
                                    .min_size(Vec2::new(296.0, 40.0)),
                            );
                            let row = format!("scene.row.{}", id.0);
                            self.remember(&row, &response);
                            if response.clicked() {
                                self.action(&row);
                            }
                        }
                    });
                if self.scene_select {
                    let response = ui.add_enabled(
                        !self.scene_selected.is_empty(),
                        egui::Button::image_and_text(
                            icons::tinted(Icon::Trash, BUTTON_ICON, self.text_color()),
                            format!("{} ({})", self.tr(Text::Delete), self.scene_selected.len()),
                        ),
                    );
                    self.remember("scene.delete", &response);
                    if response.clicked() {
                        self.action("scene.delete");
                    }
                }
            });
        if sheet.backdrop_response.clicked() && !self.scene_confirm {
            self.action("scene.close");
        }
        if self.scene_confirm {
            egui::Modal::new(Id::unique("scene.delete_dialog")).show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.heading(format!(
                    "{} {}",
                    self.tr(Text::Delete),
                    self.tr(Text::Scenes)
                ));
                ui.label(self.tr(Text::DeleteScenesPrompt));
                ui.horizontal(|ui| {
                    let response =
                        ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Cancel)));
                    self.remember("scene.cancel_delete", &response);
                    if response.clicked() {
                        self.action("scene.cancel_delete");
                    }
                    let response =
                        ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Delete)));
                    self.remember("scene.confirm_delete", &response);
                    if response.clicked() {
                        self.action("scene.confirm_delete");
                    }
                });
            });
        }
    }
    fn timeline_toolbar(&mut self, ui: &mut egui::Ui, layout: &TimelineLayout) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("timeline.toolbar")
                .max_rect(layout.toolbar.shrink2(Vec2::new(4.0, 6.0))),
        );
        child.set_clip_rect(layout.toolbar);
        child.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::Timeline))
                    .size(16.0)
                    .strong(),
            );
            ui.spacing_mut().item_spacing.x = TOOLBAR_SPACING;
            let can_undo = self.session.can_undo() || self.preference_history.can_undo();
            let can_redo = self.session.can_redo() || self.preference_history.can_redo();
            if self.icon_only(
                ui,
                "edit.undo",
                Icon::ArrowCounterClockwise,
                self.tr(Text::Undo),
                Some(self.tr(Text::ShortcutUndo)),
                None,
                can_undo,
            ) {
                self.action("edit.undo");
            }
            if self.icon_only(
                ui,
                "edit.redo",
                Icon::ArrowClockwise,
                self.tr(Text::Redo),
                Some(self.tr(Text::ShortcutRedo)),
                None,
                can_redo,
            ) {
                self.action("edit.redo");
            }
            let has_items = self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|track| !track.items.is_empty());
            let has_selection = !self.selection.is_empty();
            if self.icon_only(
                ui,
                "timeline.split",
                Icon::Scissors,
                self.tr(Text::Split),
                Some(self.tr(Text::ShortcutSplit)),
                None,
                has_items,
            ) {
                self.action("timeline.split");
            }
            if self.icon_only(
                ui,
                "timeline.delete",
                Icon::Trash,
                self.tr(Text::Delete),
                Some(self.tr(Text::ShortcutDelete)),
                None,
                has_selection,
            ) {
                self.action("timeline.delete");
            }
            if self.icon_only(
                ui,
                "timeline.snap",
                Icon::ArrowsInLineHorizontal,
                self.tr(Text::Snap),
                Some(self.tr(Text::ShortcutSnap)),
                Some(self.snapping),
                true,
            ) {
                self.action("timeline.snap");
            }
            let magnet = self.session.project().data().scene().timeline.magnet;
            if self.icon_only(
                ui,
                "timeline.magnet",
                Icon::Magnet,
                self.tr(Text::Magnet),
                None,
                Some(magnet),
                true,
            ) {
                self.track_command("track.set_magnet", serde_json::json!({"enabled":!magnet}));
            }
            if self.icon_only(
                ui,
                "timeline.ripple",
                Icon::Waveform,
                self.tr(Text::Ripple),
                Some(self.tr(Text::ShortcutRippleDelete)),
                Some(self.ripple),
                true,
            ) {
                self.action("timeline.ripple");
            }
            if self.icon_only(
                ui,
                "timeline.zoom_out",
                Icon::MagnifyingGlassMinus,
                self.tr(Text::ZoomOut),
                None,
                None,
                true,
            ) {
                self.action("timeline.zoom_out");
            }
            if self.icon_only(
                ui,
                "timeline.zoom_in",
                Icon::MagnifyingGlassPlus,
                self.tr(Text::ZoomIn),
                None,
                None,
                true,
            ) {
                self.action("timeline.zoom_in");
            }
        });
        let bounds = Rect::from_center_size(layout.toolbar.center(), Vec2::new(178.0, 28.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let name = self
                    .session
                    .project()
                    .data()
                    .scene()
                    .name
                    .as_str()
                    .to_owned();
                let response = ui
                    .add_sized([140.0, 26.0], egui::Button::new(name).truncate())
                    .on_hover_text(self.tr(Text::Scenes));
                self.remember("scene.open", &response);
                if response.clicked() {
                    self.action("scene.open");
                }
                let response = ui
                    .add(egui::Button::image(icons::tinted(
                        Icon::FilmStrip,
                        BUTTON_ICON,
                        self.text_color(),
                    )))
                    .on_hover_text(self.tr(Text::Scenes));
                self.remember("scene.open_list", &response);
                if response.clicked() {
                    self.action("scene.open");
                }
            });
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
            for (row_index, track) in data.scene().timeline.iter().rev().enumerate() {
                let height = track_height(track.kind);
                rows.push(TrackRow {
                    id: track.id,
                    main: track.id == data.scene().timeline.main.id,
                    muted: track.muted,
                    hidden: track.hidden,
                    locked: track.locked,
                    empty: track.items.is_empty(),
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
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        items: &[ItemView],
    ) {
        let mut header_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("track.headers")
                .max_rect(layout.labels),
        );
        header_ui.set_clip_rect(layout.labels);
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
            if !row_rect.intersects(layout.body) {
                continue;
            }
            let shade = if index % 2 == 0 {
                self.panel_fill()
            } else {
                self.control_fill()
            };
            label_painter.rect_filled(label_rect, 0.0, shade);
            body_painter.rect_filled(row_rect, 0.0, shade);
            let header = header_ui.interact(
                label_rect.intersect(layout.labels),
                Id::unique(("track.header", row.id.0)),
                egui::Sense::click(),
            );
            header.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name));
            let type_icon = track_icon(row.kind);
            self.remember_with_icon(
                format!("track.{}.header", row.id.0),
                Some(type_icon.name()),
                &header,
            );
            let icon_rect = Rect::from_center_size(
                egui::pos2(label_rect.right() - 14.0, label_rect.center().y),
                Vec2::splat(16.0),
            );
            icons::tinted(type_icon, 16.0, self.secondary()).paint_at(&header_ui, icon_rect);
            let mut x = label_rect.right() - 54.0;
            if row.kind.can_hide() {
                if self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    row,
                    x,
                    "hidden",
                    Text::HideTrack,
                    row.hidden,
                    if row.hidden {
                        Icon::EyeSlash
                    } else {
                        Icon::Eye
                    },
                ) {
                    self.track_command(
                        "track.set_hidden",
                        serde_json::json!({"track":row.id,"hidden":!row.hidden}),
                    );
                }
                x -= 28.0;
            }
            if row.kind.can_mute()
                && self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    row,
                    x,
                    "muted",
                    Text::MuteTrack,
                    row.muted,
                    if row.muted {
                        Icon::SpeakerSlash
                    } else {
                        Icon::SpeakerHigh
                    },
                )
            {
                self.track_command(
                    "track.set_muted",
                    serde_json::json!({"track":row.id,"muted":!row.muted}),
                );
            }
            if row.main {
                let enabled = self.session.project().data().scene().timeline.magnet;
                // A separate control fits below the main header's output toggles.
                let mut magnet_row = row.clone();
                magnet_row.top += 18.0;
                if self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    &magnet_row,
                    label_rect.left() + 4.0,
                    "magnet",
                    Text::Magnet,
                    enabled,
                    Icon::Magnet,
                ) {
                    self.track_command("track.set_magnet", serde_json::json!({"enabled":!enabled}));
                }
            }
            header.context_menu(|ui| {
                if !row.main {
                    let response = ui.add_enabled(
                        !row.locked,
                        egui::Button::image_and_text(
                            icons::tinted(Icon::Trash, 16.0, self.text_color()),
                            self.tr(Text::DeleteTrack),
                        ),
                    );
                    let response = response.on_disabled_hover_text(self.tr(Text::TrackLocked));
                    self.remember_with_icon(
                        format!("track.{}.delete", row.id.0),
                        Some(Icon::Trash.name()),
                        &response,
                    );
                    if response.clicked() {
                        self.track_command("track.remove", serde_json::json!({"track":row.id}));
                        ui.close();
                    }
                }
            });
            if row.empty {
                let r = row_rect.shrink(3.0);
                let points = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ];
                body_painter.extend(egui::Shape::dashed_line(
                    &points,
                    egui::Stroke::new(1.0, self.border()),
                    5.0,
                    4.0,
                ));
            }
        }
        let background = ui.interact(
            layout.body,
            Id::unique("timeline.marquee"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.marquee", &background);
        let modifiers = ui.ctx().input(|input| input.modifiers);
        for item in items {
            self.paint_item(ui, layout, item, modifiers);
        }
        let item_rects: Vec<_> = items
            .iter()
            .map(|item| {
                (
                    item.id,
                    Rect::from_min_max(
                        egui::pos2(self.x_at(item.start, layout.body.left()), item.top + 2.0),
                        egui::pos2(
                            self.x_at(item.end, layout.body.left()),
                            item.top + item.height - 2.0,
                        ),
                    )
                    .intersect(layout.body),
                )
            })
            .filter(|(_, rect)| rect.is_positive())
            .collect();
        if background.drag_started()
            && self.drag.is_none()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && !item_rects.iter().any(|(_, r)| r.contains(origin))
        {
            self.marquee = Some((
                origin,
                self.selection.clone(),
                modifiers.command || modifiers.ctrl || modifiers.shift,
            ));
        }
        if let Some((origin, before, additive)) = &self.marquee
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(*origin, pointer).intersect(layout.body);
            let mut selected =
                crate::selection::intersections(rect.min, rect.max, item_rects.iter().copied());
            if *additive {
                selected.extend(before);
            }
            self.selection = selected;
            ui.painter().with_clip_rect(layout.body).rect_filled(
                rect,
                0.0,
                self.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(layout.body).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, self.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped() && self.marquee.take().is_some() {
            self.selection_changed();
        }
        if background.clicked() {
            self.selection.clear();
            self.selection_changed();
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
    fn track_command(&mut self, command: &str, params: serde_json::Value) {
        let before = self.session.project().clone();
        match self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.execute(command, params))
        {
            Ok(_) => {
                self.prune_selection();
                if !Arc::ptr_eq(&before, self.session.project()) {
                    self.sync_preview();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn track_toggle(
        &mut self,
        ui: &mut egui::Ui,
        clip: Rect,
        row: &TrackRow,
        x: f32,
        suffix: &str,
        label: Text,
        selected: bool,
        icon: Icon,
    ) -> bool {
        let rect = Rect::from_min_size(
            egui::pos2(
                x,
                clip.top() + row.top - self.vertical_scroll + (row.height - 24.0) / 2.0,
            ),
            Vec2::splat(ICON_BUTTON),
        )
        .intersect(clip);
        if !rect.is_positive() {
            return false;
        }
        let response = ui.interact(
            rect,
            Id::unique(("track", row.id.0, suffix)),
            egui::Sense::click(),
        );
        let tooltip = catalog::format(
            self.preferences.locale,
            Text::TrackToggle,
            &[("action", self.tr(label)), ("track", &row.name)],
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, tooltip.clone())
        });
        self.remember_with_icon(
            format!("track.{}.{suffix}", row.id.0),
            Some(icon.name()),
            &response,
        );
        if let Some(state) = self
            .widgets
            .get_mut(&format!("track.{}.{suffix}", row.id.0))
        {
            state.tooltip = Some(tooltip.clone());
        }
        let color = if selected {
            self.accent()
        } else {
            self.secondary()
        };
        if response.hovered() || response.has_focus() {
            ui.painter()
                .with_clip_rect(clip)
                .rect_filled(rect, 4.0, self.control_fill());
        }
        icons::tinted(icon, 16.0, color)
            .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)));
        response.clone().on_hover_text(tooltip);
        response.clicked()
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
            TrackKind::Video if item.overlay.is_none() => {
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
                        Color32::from_rgba_unmultiplied(text.r(), text.g(), text.b(), shade),
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
        let thumb_width =
            ((view / content) * track.width()).clamp(12.0_f32.min(track.width()), track.width());
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
            && track.width() > 0.0
        {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            self.scroll = fraction * max;
        }
        let content_height: f32 = rows.iter().map(|row| row.height + TRACK_GAP).sum::<f32>();
        let max_vertical = (content_height - TRACK_GAP - layout.body.height()).max(0.0);
        let track = layout.vertical.shrink2(Vec2::new(3.0, 0.0));
        let thumb_height = if content_height > 0.0 {
            ((layout.body.height() / content_height) * track.height())
                .clamp(12.0_f32.min(track.height()), track.height())
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
            && track.height() > 0.0
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
        theme::accent(self.preferences.dark)
    }
    fn panel_fill(&self) -> Color32 {
        theme::panel(self.preferences.dark)
    }
    fn control_fill(&self) -> Color32 {
        theme::field(self.preferences.dark)
    }
    fn border(&self) -> Color32 {
        theme::border(self.preferences.dark)
    }
    fn text_color(&self) -> Color32 {
        theme::text(self.preferences.dark)
    }
    fn secondary(&self) -> Color32 {
        theme::secondary(self.preferences.dark)
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
    let icon = if video.is_some() {
        Icon::FilmStrip
    } else if audio.is_some() {
        Icon::MusicNotes
    } else {
        Icon::File
    };
    info.push(catalog::text(locale, kind).to_owned());
    info.push(asset.probe().duration.map_or_else(
        || catalog::text(locale, Text::DurationUnknown).to_owned(),
        display_time,
    ));
    if let Some(video) = video {
        let width = catalog::number(locale, u64::from(video.width));
        let height = catalog::number(locale, u64::from(video.height));
        info.push(catalog::format(
            locale,
            Text::AssetDimensions,
            &[("width", &width), ("height", &height)],
        ));
    }
    if let Some(rate) = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
    {
        let value = if rate.den() == 1 {
            catalog::signed(locale, rate.num())
        } else {
            catalog::decimal(locale, rate.num() as f64 / rate.den() as f64, 3)
        };
        info.push(catalog::format(locale, Text::AssetFps, &[("rate", &value)]));
    }
    if let Some(audio) = audio {
        let rate = catalog::number(locale, u64::from(audio.sample_rate));
        let channels = catalog::number(locale, u64::from(audio.channels));
        info.push(catalog::format(
            locale,
            Text::AssetAudio,
            &[("rate", &rate), ("channels", &channels)],
        ));
    }
    AssetRow {
        id: asset.id(),
        name: file_name(asset.path()),
        info: info.join(" · "),
        icon,
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
        TrackKind::Text
        | TrackKind::Sticker
        | TrackKind::Effect
        | TrackKind::Filter
        | TrackKind::Adjustment => 25.0,
    }
}
fn track_icon(kind: TrackKind) -> Icon {
    match kind {
        TrackKind::Video => Icon::FilmStrip,
        TrackKind::Audio => Icon::MusicNotes,
        TrackKind::Text => Icon::TextT,
        TrackKind::Sticker => Icon::Sticker,
        TrackKind::Effect => Icon::Sparkle,
        TrackKind::Filter => Icon::Funnel,
        TrackKind::Adjustment => Icon::SlidersHorizontal,
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
    let text = theme::text(dark);
    let (fill, border) = match kind {
        TrackKind::Video => (theme::video_fill(dark), theme::video_border(dark)),
        TrackKind::Audio => (theme::field(dark), theme::border(dark)),
        _ => (theme::hover(dark), theme::border(dark)),
    };
    let fill = overlay.map_or(fill, |[r, g, b, a]| {
        Color32::from_rgba_unmultiplied(r, g, b, a)
    });
    if selected {
        (fill, theme::accent(dark), text)
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
