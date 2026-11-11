//! Coordinates the editor through area interfaces.
use crate::i18n;
mod assets;
mod captions;
mod context;
pub(crate) use context::{Context, Desk, Request};
pub(crate) mod dialogs;
mod frame;
pub use frame::{AppCommand, PreviewZoom};
pub(crate) mod panels;
pub(crate) use panels::player::KeyPick;
pub use panels::player::crop::{Aspect as CropAspect, Inspection as CropInspection};
pub use panels::properties;
mod timeline;
mod widgets;

mod state;
pub use assets::sounds::{Client, Sounds, SoundsMode, SoundsProblem, SoundsView};
pub use panels::player::{PreviewAxis, PreviewGuide, TextCaret};
pub use state::{
    AssetInspection, AssetState, Editor, Inspection, ShortcutRow, ShortcutTab, ShortcutsInspection,
};
use state::{
    AssetRow, MediaMarquee, MediaReveal, PreferenceCommand, SHORTCUT_CONTROL_HEIGHT,
    SHORTCUT_ROW_HEIGHT, SHORTCUT_TEXT_SIZE, SHORTCUT_TITLE_SIZE, Toast,
};
pub use widgets::WidgetState;

mod timing;
use timing::{display_short_time, display_time};

use crate::selection::{KeyframeSelection, SelectionScope, SelectionScopes};
use crate::snapping::{SnapCandidate, SnapSource, snap_point};
use crate::{
    Error, Result,
    actions::{
        self, ActionTarget, BlockedReason, Dispatch, DispatchContext, EngineAction,
        KeybindingIssue, Keymap, UiAction,
    },
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    i18n::{Locale, Text},
    import::{
        ImportEvent, ImportFailure, ImportProbe, ImportQueue, ThumbnailOutcome, ThumbnailSource,
    },
    kit as theme,
    kit::{self as icons, Icon},
    media_panel::{self, MediaKind, MediaSort, MediaSortKey, MediaSortOrder, MediaView},
    panels::AssetsTab,
    preferences::{Preferences, drag_panel},
    services::playback::Playback,
};
use engine::{
    Command, CreatedId, ItemParamPatch, NewItem, Session,
    jobs::{Clock, SystemClock},
};
use media::{AssetId, ContentHash, SourceAsset, StreamKind, UnsupportedReason};
use project::{BookmarkId, ItemId, ItemParams, TrackId, TrackKind};
use properties::ColorTarget;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use time::{FrameRate, Rounding, SourceRange, TICKS_PER_SECOND, Tick};

const TOOLBAR_HEIGHT: f32 = theme::size::TOOLBAR;
const BUTTON_ICON: f32 = theme::icon::DENSE;
const ICON_BUTTON: f32 = theme::size::CONTROL;
const RULER_HEIGHT: f32 = theme::size::RULER;
const LABEL_WIDTH: f32 = theme::size::TRACK_LABEL;
const SCROLLBAR: f32 = theme::size::SCROLLBAR;
const TRACK_GAP: f32 = theme::space::SM;
const MEDIA_REVEAL: Duration = Duration::from_millis(1500);
/// Fixed reference scale for persisted view state: 1× is 40 logical px/s.
const DEFAULT_ZOOM: f32 = 40.0;
/// The timeline slider tops out at 100× the reference scale.
const MAX_ZOOM: f32 = DEFAULT_ZOOM * 100.0;
/// A positive floor for hostile/empty content; the fit minimum is dynamic.
const MIN_ZOOM_FLOOR: f32 = 0.000_001;
const ZOOM_STEP: f32 = 1.7;
const ZOOM_ANCHOR_THRESHOLD: f32 = 0.15;
const PADDING_MAX_RATIO: f32 = 0.75;
const PADDING_MIN_RATIO: f32 = 0.15;
const PADDING_MIN_AT_ZOOM_PERCENT: f32 = 0.2;
const EDGE_AUTO_SCROLL_ZONE: f32 = 100.0;
const EDGE_AUTO_SCROLL_MAX_STEP: f32 = 15.0;
const HORIZONTAL_WHEEL_STEP: f32 = 40.0;
const RULER_LABEL_SPACING: f32 = 160.0;
const RULER_TICK_SPACING: f32 = 18.0;
const LABEL_CHARS: usize = 120;

impl Editor {
    pub(crate) fn theme(ctx: &egui::Context, dark: bool) {
        crate::kit::install_theme(ctx, dark);
    }
    pub(crate) fn fonts(ctx: &egui::Context) {
        crate::kit::install_fonts(ctx);
    }
    pub(in crate::editor) fn tr(&self, key: Text) -> &'static str {
        i18n::text(self.preferences.locale, key)
    }
    pub(in crate::editor) fn accent(&self) -> Color32 {
        theme::accent(self.preferences.dark)
    }
    pub(in crate::editor) fn panel_fill(&self) -> Color32 {
        theme::panel(self.preferences.dark)
    }
    pub(in crate::editor) fn control_fill(&self) -> Color32 {
        theme::field(self.preferences.dark)
    }
    pub(in crate::editor) fn border(&self) -> Color32 {
        theme::border(self.preferences.dark)
    }
    pub(in crate::editor) fn secondary(&self) -> Color32 {
        theme::secondary(self.preferences.dark)
    }
    /// Inject the catalog command adapter. Production opens it lazily; tests
    /// and audits supply a recording fake.
    pub fn set_font_backend(&mut self, backend: Box<dyn properties::font_picker::Backend>) {
        self.properties.set_font_backend(backend);
    }
    /// Wire the My fonts tab once font import (80) supplies imported families.
    pub fn set_font_imports_available(&mut self, available: bool) {
        self.properties.set_font_imports_available(available);
    }
    /// Open the shared colour picker on a target the workspace owns.
    pub fn open_color_picker(&mut self, target: ColorTarget) -> Result<()> {
        let actions = self
            .properties
            .open_picker(self.session.project(), target, None)?;
        properties::apply(&mut self.desk(), actions);
        Ok(())
    }
    /// Inject the native capability boundary for an open picker.
    pub fn set_eyedropper(
        &mut self,
        sampler: crate::services::color_picker::NativeEyedropper,
    ) -> Result<()> {
        self.properties.set_eyedropper(sampler)
    }
    /// The inspector and automation use the same registered atomic field edit.
    pub fn set_shared_field(&mut self, field: &str, value: &str) -> Result<()> {
        properties::set_shared_field(&mut self.desk(), field, value)
    }
    /// One element's committed field edit, through the singular registered command.
    pub fn set_item_field(&mut self, item: ItemId, field: &str, value: &str) -> Result<()> {
        properties::set_item_field(&mut self.desk(), item, field, value)
    }
    /// Adopt one chroma-key eyedropper pick for the selected clip.
    pub(in crate::editor) fn pick_key_color(
        &mut self,
        item: ItemId,
        effect: Option<&str>,
        color: project::Color,
    ) -> Result<()> {
        let mut state = std::mem::take(&mut self.properties);
        let result = {
            let mut desk = self.desk();
            state.pick_key_color(&mut desk, item, effect, color)
        };
        self.properties = state;
        result
    }
    /// Attach one effect definition to a clip's stack as one undo step.
    pub(in crate::editor) fn add_effect_to_item(&mut self, item: ItemId, kind: &str) {
        properties::add_effect_to_item(&mut self.desk(), item, kind);
    }
    /// Poll the active font download; the frame ticks it once per frame.
    pub(in crate::editor) fn poll_fonts(&mut self) {
        self.properties.poll_fonts();
    }
    /// End the properties scrub gesture, if one is open.
    pub(in crate::editor) fn finish_scrub(&mut self, commit: bool) {
        let actions = self.properties.end_scrub(commit);
        properties::apply(&mut self.desk(), actions);
    }
    /// Adopt the workspace's one media cache instance for timeline display
    /// reads and generation. The same instance is lent to the MCP inbox.
    pub(crate) fn set_media(
        &mut self,
        media: std::sync::Arc<std::sync::Mutex<engine::control::MediaAccess>>,
    ) {
        self.properties
            .attach_luts(media.clone(), self.playback.budget().clone());
        self.media.set_access(media);
    }

    /// Complete the source picker through the captured properties workflow.
    pub fn lut_picked_file(&mut self, path: Option<std::path::PathBuf>) -> Result<()> {
        let mut state = std::mem::take(&mut self.properties);
        let result = state.picked_lut(&mut self.desk(), path);
        if result.is_err() {
            state.abort_lut();
        }
        self.properties = state;
        result
    }

    pub fn new(session: Session, playback: Playback, preferences: Preferences) -> Self {
        Self::with_session(
            crate::lifecycle::EditorSession::Memory(Box::new(session)),
            playback,
            preferences,
        )
    }
    /// A transient preview editor draws with the caller's appearance and
    /// defaults for every other preference; the shorts window owns its state.
    pub fn preview(session: Session, playback: Playback, look: crate::appearance::Look) -> Self {
        let preferences = Preferences {
            locale: look.locale,
            dark: look.dark,
            ..Preferences::default()
        };
        Self::new(session, playback, preferences)
    }
    fn with_session(
        session: crate::lifecycle::EditorSession,
        playback: Playback,
        preferences: Preferences,
    ) -> Self {
        let view = session.metadata().view;
        let zoom = (f64::from(DEFAULT_ZOOM) * view.zoom)
            .clamp(f64::from(MIN_ZOOM_FLOOR), f64::from(MAX_ZOOM)) as f32;
        let scroll = (view.scroll / f64::from(zoom)) as f32;
        let selection = session.selection().clone();
        let preferences_fit = preferences.preview_fit;
        let preferences_zoom = preferences.preview_zoom;
        let project = session.project().data().id;
        let media = timeline::Media::new(playback.budget().clone());
        let timeline_lanes: Vec<ItemId> = preferences
            .timeline_lanes
            .get(&project)
            .into_iter()
            .flatten()
            .copied()
            .map(ItemId)
            .collect();
        let mut player = panels::player::State::new(preferences_fit, preferences_zoom);
        player.last_view = view;
        let frame = frame::State::new(&preferences);
        Self {
            session,
            playback,
            preferences,
            overlay_open: false,
            selection,
            selection_scopes: SelectionScopes::default(),
            properties: properties::State::default(),
            effect_previews: crate::effect_previews::EffectPreviews::cpu().ok(),
            effect_drag: None,
            frame,
            player,
            timeline: {
                let mut state = timeline::State::new(zoom, scroll);
                state.restore_lanes(timeline_lanes);
                state
            },
            media,
            error: None,
            persistence_error: None,
            configured: false,
            lifecycle_action: None,
            imports: None,
            import_total: 0,
            import_completed: 0,
            import_imported: 0,
            import_cancelled: false,
            import_failures: Vec::new(),
            import_notice: None,
            import_error: None,
            clock: Arc::new(SystemClock::default()),
            toast: None,
            assets: assets::State::default(),
            media_selection: BTreeSet::new(),
            media_anchor: None,
            media_sizes: BTreeMap::new(),
            media_marquee: None,
            media_reveal: None,
            media_delete_confirm: None,
            dialogs: dialogs::State::default(),
            asset_states: BTreeMap::new(),
            thumbnails: BTreeMap::new(),
            pending_imports: BTreeMap::new(),
            restoring_asset: None,
            thumbnail_textures: std::cell::RefCell::default(),
            thumbnail_root: None,
            sound_library: None,
            sounds: None,
            sound_media_dir: None,
            subtitles: None,
            requests: Vec::new(),
        }
    }
    /// Panel rectangles from the last painted frame, for automation reads.
    pub fn panels(&self) -> &BTreeMap<&'static str, Rect> {
        &self.frame.panels
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
        let media_order = self.media_order();
        let media_highlight = self.media_reveal.map(|reveal| reveal.asset);
        let media_delete_pending = self.media_delete_confirm.clone().unwrap_or_default();
        Ok(Inspection {
            widgets: self.frame.widgets.values().cloned().collect(),
            view: "editor",
            playing: self.playback.playing(),
            position: self.playback.position()?,
            selection: self.selection.iter().copied().collect(),
            selection_scope: self.selection_scopes.active(&self.selection),
            expanded_lanes: self.timeline.expanded_lanes(),
            zoom: self.timeline.zoom(),
            scroll: self.timeline.scroll(),
            min_zoom: self.timeline.min_zoom(self.session.project().duration()),
            snapping: self.preferences.snapping,
            snap_indicator: self.timeline.snap_indicator(),
            preview_guides: self.player.transform.guides().to_vec(),
            bookmark_popover: self.timeline.bookmark_popover(),
            preview: self.timeline.preview_command(),
            timeline_refusal: self.timeline.refusal().map(str::to_owned),
            drop_zone: self.timeline.drop_zone_label(),
            pinned: self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .filter(|item| item.start == Tick::ZERO)
                .map(|item| item.id)
                .collect(),
            ripple: self.preferences.ripple,
            preview_zoom: self.player.view.zoom,
            preview_fit: self.player.view.fit,
            preview_offset: self.player.view.offset,
            preview_fullscreen: self.player.fullscreen,
            preview_axis: self.preferences.preview_axis,
            hover_time: self.timeline.hover_time(),
            preview_at: self.playback.hover(),
            timecode: self.current_timecode(),
            timecode_editing: self.player.timecode_edit.is_some(),
            snapshot_dialog_open: self.player.snapshot.is_open(),
            snapshot_saved: self.player.snapshot.saved.clone(),
            snapshot_error: self.player.snapshot.error,
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
                    i18n::format(
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
            assets: self
                .session
                .project()
                .data()
                .assets
                .iter()
                .filter(|asset| !asset.ephemeral())
                .map(|asset| {
                    let state = self
                        .asset_states
                        .get(&asset.id())
                        .copied()
                        .unwrap_or(AssetState::Probing);
                    AssetInspection {
                        id: asset.id(),
                        state: state.code(),
                        thumbnail: self.thumbnails.contains_key(&asset.id()),
                        detail: match state {
                            AssetState::Unsupported(reason) => Some(reason.label().to_owned()),
                            _ => None,
                        },
                    }
                })
                .chain(self.pending_imports.keys().map(|id| AssetInspection {
                    id: *id,
                    state: "probing",
                    thumbnail: false,
                    detail: None,
                }))
                .collect(),
            effect_previews: self
                .effect_previews
                .as_ref()
                .map_or_else(BTreeMap::new, |previews| previews.statuses()),
            media_view: self.preferences.media_view,
            media_sort: self.preferences.media_sort,
            media_order,
            media_highlight,
            media_delete_pending,
            sounds: self.sounds.as_ref().map(|sounds| sounds.view()),
            export_options: self.preferences.export,
            export: self.dialogs.export.inspect(),
            crop: self
                .player
                .crop
                .as_ref()
                .map(panels::player::crop::Dialog::inspect),
            subtitles: self.subtitles.as_ref().map(|s| s.inspect(&self.session)),
            models: self.dialogs.models.inspect(),
            keybinding_issues: self.frame.keybinding_issues.clone(),
            color_preview: self.properties.color_preview(),
            eyedropper: self.properties.eyedropper(),
            key_color_pick: self.player.key_color_pick.as_ref().map(|pick| pick.item),
            font_picker: self
                .properties
                .font_inspection(&self.preferences.font_picker),
            text_caret: self.player.transform.text_caret(),
            shortcuts: self.frame.shortcuts_open.then(|| {
                let platform = actions::Platform::current();
                let rows = actions::actions()
                    .iter()
                    .filter(|spec| spec.bindable())
                    .map(|spec| {
                        let chords = self.preferences.keybindings.effective_chords(spec);
                        ShortcutRow {
                            action: spec.id.to_owned(),
                            label: spec.label.text(self.preferences.locale).to_owned(),
                            category: spec.category.label(),
                            tab: ShortcutTab::for_category(spec.category),
                            keys: chords
                                .iter()
                                .filter_map(|text| actions::Chord::parse(text).ok())
                                .map(|chord| chord.display(platform))
                                .collect(),
                            customized: self
                                .preferences
                                .keybindings
                                .is_customized
                                .contains(spec.id),
                            unbound: chords.is_empty(),
                        }
                    })
                    .collect();
                ShortcutsInspection {
                    tab: self.frame.shortcut_tab,
                    recording: self.frame.shortcut_recording.map(str::to_owned),
                    error: self.frame.shortcut_error.clone(),
                    rows,
                }
            }),
        })
    }
    /// Last frame submitted by the frontend for presentation, independent of decode-ahead.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.player.texture.presented()
    }
    /// Replace the UI timer clock in tests; production uses the system clock.
    /// Set it before raising a timed state so its deadline uses this clock.
    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = clock;
    }
    /// Upload one decoded thumbnail on first use; later frames reuse the handle.
    fn thumbnail_texture(&self, ctx: &egui::Context, id: AssetId) -> Option<egui::TextureHandle> {
        context::Thumbnails {
            images: &self.thumbnails,
            textures: &self.thumbnail_textures,
        }
        .texture(ctx, id)
    }
    /// Start the export sheet with typed caller settings without mutating
    /// editable project content.
    pub(in crate::editor) fn run_export(
        &mut self,
        options: dialogs::export::ExportOptions,
    ) -> Result<()> {
        if self.session.transaction_active() {
            return Err(Error::Engine(engine::EditError::TransactionActive));
        }
        let project = self.session.project().clone();
        let budget = self.playback.budget().clone();
        let render = self.playback.render_state()?;
        self.dialogs.export.run(&project, options, budget, render)
    }
    /// Open or raise the export dialog without changing project content.
    pub(in crate::editor) fn open_export(&mut self) -> Result<()> {
        let budget = self.playback.budget().clone();
        let render = self.playback.render_state()?;
        let project = self.session.project().clone();
        self.dialogs.export.open(budget, render, &project)
    }
    /// Accept the native export save picker's result without opening OS UI in tests.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let protected = self
            .session
            .autosave()
            .map(|s| s.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        self.dialogs.export.save_to(path, protected)
    }
    /// Cancelling the native save dialog keeps the finished job for a later save.
    pub fn export_save_cancelled(&mut self) {
        self.dialogs.export.save_cancelled();
    }
    /// Default filename and last directory for the native save dialog.
    pub fn export_save_defaults(&self) -> (String, Option<PathBuf>) {
        self.dialogs.export.defaults(
            self.session.project(),
            self.preferences.export,
            self.preferences.export_directory.clone(),
        )
    }
    /// The completed attempt owns its container even if settings changed after failure.
    pub fn export_save_format(&self) -> dialogs::export::ExportFormat {
        self.dialogs
            .export
            .save_format(self.preferences.export.format)
    }
    /// The export dialog, for automation and test injection.
    pub fn export_dialog(&mut self) -> Option<&mut dialogs::export::ExportDialog> {
        self.dialogs.export.dialog_mut()
    }
    /// The export dialog state, for automation reads.
    pub fn export_state(&self) -> Option<&dialogs::export::ExportDialog> {
        self.dialogs.export.dialog()
    }
    /// Instantiate the one shared model cache on first use; it never fetches then.
    pub fn open_models(&mut self) -> Result<()> {
        let budget = self.playback.budget().clone();
        self.dialogs.models.open(budget)
    }
    /// Open the manager on a specific missing model, as `/09` does.
    pub fn open_model(&mut self, id: &str) -> Result<()> {
        let budget = self.playback.budget().clone();
        self.dialogs.models.open_model(budget, id)
    }
    /// Inject cache/transport/IO ownership for composition and tests.
    pub fn set_model_manager(&mut self, models: dialogs::models::model_manager::ModelManager) {
        self.dialogs.models.set(models);
    }
    pub(crate) fn take_model_manager(
        &mut self,
    ) -> Option<dialogs::models::model_manager::ModelManager> {
        self.dialogs.models.take()
    }
    pub(crate) fn set_models_visible(&mut self, visible: bool) {
        self.dialogs.models.set_visible(visible);
    }
    pub fn models(&self) -> Option<&dialogs::models::model_manager::ModelManager> {
        self.dialogs.models.manager()
    }
    /// Native picker destination applied through the consumed `preview.snapshot`
    /// command. Failure stays typed and never shows a success toast.
    pub fn snapshot_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let at = self.playback.position()?;
        let protected = self
            .session
            .autosave()
            .map(|autosave| autosave.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        let budget = self.playback.budget().clone();
        match self
            .player
            .snapshot
            .save(&mut self.session, budget, at, &path, protected)
        {
            Ok(()) => {
                self.preferences
                    .set("snapshot_directory", serde_json::to_value(path.parent())?)?;
                self.set_toast(
                    self.tr(Text::SnapshotSaved),
                    Some(path.to_string_lossy().into_owned()),
                );
                Ok(())
            }
            Err(error) => {
                self.desk().fail(error.to_string());
                Err(error)
            }
        }
    }
    pub fn snapshot_save_cancelled(&mut self) {
        self.player.snapshot.cancel();
    }
    /// Tests and non-window callers consume the same lifecycle request the
    /// workspace turns into the native picker.
    pub fn take_lifecycle_action(&mut self) -> Option<crate::lifecycle::EditorAction> {
        self.lifecycle_action.take()
    }
    /// Default filename and last directory for the native snapshot dialog.
    pub fn snapshot_save_defaults(&self) -> (String, Option<PathBuf>) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        (
            engine::snapshot::default_name(self.session.project(), at)
                .unwrap_or_else(|_| "snapshot.png".into()),
            self.preferences.snapshot_directory.clone(),
        )
    }
    /// Put the current frame on the OS clipboard through egui's clipboard owner.
    pub fn copy_snapshot(&mut self, ctx: &egui::Context) -> Result<()> {
        let at = self.playback.position()?;
        if let Err(error) = crate::services::preview::copy(&self.session, at, ctx) {
            self.player.snapshot.error = Some(match &error {
                Error::Engine(engine::EditError::EmptyProject) => {
                    engine::control::ErrorCode::EmptyProject
                }
                _ => engine::control::ErrorCode::Failed,
            });
            self.desk().fail(error.to_string());
            return Err(error);
        }
        self.player.snapshot.error = None;
        self.set_toast(self.tr(Text::SnapshotCopy), None);
        Ok(())
    }
    /// Shared transport and preview surface for dedicated review windows.
    pub fn preview_ui(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            Self::fonts(ui.ctx());
            crate::kit::install(ui.ctx());
            self.configured = true;
        }
        Self::theme(ui.ctx(), self.preferences.dark);
        crate::kit::install_editor_density(ui.ctx());
        self.show_player(ui);
    }
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
/// A library tile must read as the effect at a glance, so previews use a
/// visible setting instead of an insertion default that can be an identity.
fn effect_preview_instance(
    definition: &project::EffectDefinition,
) -> Option<project::EffectInstance> {
    let mut instance = definition.instance(definition.id);
    match definition.id {
        "blur" => {
            let (_, max) = definition.param("intensity")?.scalar_range()?;
            instance.set_scalar("intensity", max).ok()?;
        }
        "directional_blur" => {
            instance.set_scalar("length", 8.0).ok()?;
            instance.set_scalar("direction", 45.0).ok()?;
        }
        "gamma_correction" => instance.set_scalar("gamma", 25.0).ok()?,
        "levels" => {
            instance.set_scalar("in_black", 40.0).ok()?;
            instance.set_scalar("in_white", 200.0).ok()?;
        }
        // Swap red and blue so the identity matrix does not read as no effect.
        "channel_mix" => {
            for (id, value) in [("rr", 0.0), ("rb", 100.0), ("br", 100.0), ("bb", 0.0)] {
                instance.set_scalar(id, value).ok()?;
            }
        }
        "color_replace" => {
            instance = instance
                .with(
                    "target",
                    project::EffectValue::Color(project::Color([0, 0, 255, 255])),
                )
                .ok()?
                .with(
                    "replace",
                    project::EffectValue::Color(project::Color([255, 0, 0, 255])),
                )
                .ok()?;
        }
        "strobe" => {
            instance = instance
                .with(
                    "color",
                    project::EffectValue::Color(project::Color([255, 210, 0, 255])),
                )
                .ok()?;
        }
        _ => {}
    }
    Some(instance)
}
fn display_label(value: &str, limit: usize) -> String {
    let mut label: String = value.chars().take(limit.saturating_add(1)).collect();
    if label.chars().count() > limit {
        label.pop();
        label.push('…');
    }
    label
}
