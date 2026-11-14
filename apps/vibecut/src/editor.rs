//! Coordinates the editor through area interfaces.
use crate::i18n;
mod context;
pub(crate) use context::{Context, Request};
pub(in crate::editor) use context::{Note, Port, Read};
pub(crate) mod dialogs;
mod frame;
pub use frame::{AppCommand, PreviewZoom};
pub(crate) mod panels;
pub(crate) use panels::assets;
pub(crate) use panels::assets::media_panel;
pub use panels::player::crop::{CropAspect, CropInspection};
pub(crate) use panels::player::{KeyPick, MatteBrush};
pub use panels::properties;
mod timeline;
mod widgets;

mod state;
pub use assets::captions::{Inspection as SubtitlesInspection, Subtitles};
pub use assets::effects::{
    EffectPreviewStatus, EffectPreviews, PREVIEW_SCALE, PREVIEW_SIZE, SETTLE_BUDGET,
};
pub use assets::sounds::music::{Music, MusicMode, MusicProblem};
pub use assets::sounds::{Client, LibraryProblem, LibraryStore, Sounds, SoundsMode, SoundsProblem};
pub use assets::{
    AssetState, AssetsTab, ImportEvent, ImportProbe, ImportQueue, MAX_IMPORT_FILES, MediaSort,
    MediaSortKey, MediaSortOrder, MediaView, ThumbnailSource,
};
pub use panels::player::{
    Guide, OverlayVisibility, PREVIEW_SNAP_PIXELS, PreviewAxis, PreviewGuide, TextCaret,
};
pub use state::{
    AssetInspection, Editor, Inspection, ShortcutRow, ShortcutTab, ShortcutsInspection,
};
use state::{
    AssetRow, PreferenceCommand, SHORTCUT_CONTROL_HEIGHT, SHORTCUT_ROW_HEIGHT, SHORTCUT_TEXT_SIZE,
    SHORTCUT_TITLE_SIZE, Toast,
};
pub use widgets::WidgetState;

mod selection;
pub use selection::{KeyframeSelection, MaskPointSelection, SelectionScope, SelectionScopes};

use crate::{
    Error, Result,
    actions::{
        self, ActionTarget, BlockedReason, Dispatch, DispatchContext, EngineAction,
        KeybindingIssue, Keymap, UiAction,
    },
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    i18n::{Locale, Text},
    kit as theme,
    kit::{self as icons, Icon},
    preferences::{Preferences, drag_panel},
    services::{lifecycle::CaptionOutput, playback::Playback},
};
use engine::{
    Command, CreatedId, ItemParamPatch, NewItem, Session,
    jobs::{Clock, JobRunner, JobState, RunnerConfig, SystemClock},
};
use media::{AssetId, ContentHash, SourceAsset, StreamKind, UnsupportedReason};
use project::{BookmarkId, ItemId, ItemParams, TrackId, TrackKind};

/// Default hold for Freeze frame: three seconds. Unverified against the CapCut reference.
const FREEZE_HOLD_TICKS: i64 = 3 * time::TICKS_PER_SECOND;
use properties::ColorTarget;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use time::{FrameRate, Rounding, SourceRange, TICKS_PER_SECOND, Tick};

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
    /// Inject the catalog command adapter. Production opens it lazily; tests
    /// and audits supply a recording fake.
    pub fn set_font_backend(&mut self, backend: Box<dyn properties::font_picker::Backend>) {
        self.properties.set_font_backend(backend);
    }
    /// Open the shared colour picker on a target the workspace owns.
    pub fn open_color_picker(&mut self, target: ColorTarget) -> Result<()> {
        self.with_properties(|state, port| state.open_color_picker(port, target))
    }
    /// The inspector and automation use the same registered atomic field edit.
    pub fn set_shared_field(&mut self, field: &str, value: &str) -> Result<()> {
        self.with_properties(|_, port| properties::set_shared_field(port, field, value))
    }
    /// One element's committed field edit, through the singular registered command.
    pub fn set_item_field(&mut self, item: ItemId, field: &str, value: &str) -> Result<()> {
        self.with_properties(|_, port| properties::set_item_field(port, item, field, value))
    }
    /// Adopt one chroma-key eyedropper pick for the selected clip.
    pub(in crate::editor) fn pick_key_color(
        &mut self,
        item: ItemId,
        effect: Option<&str>,
        color: project::Color,
    ) -> Result<()> {
        self.with_properties(|state, port| state.pick_key_color(port, item, effect, color))
    }
    /// Publish one finished refinement stroke of the matte brush as one undo step.
    pub(in crate::editor) fn paint_matte_stroke(
        &mut self,
        item: ItemId,
        stroke: project::MatteStroke,
    ) -> Result<()> {
        self.with_properties(|_, port| properties::paint_matte_stroke(port, item, stroke))
    }
    /// Poll the active font download; the frame ticks it once per frame.
    pub(in crate::editor) fn poll_fonts(&mut self) {
        self.with_properties(|state, port| state.poll_fonts(port));
    }
    /// End the properties scrub gesture, if one is open.
    pub(in crate::editor) fn finish_scrub(&mut self, commit: bool) {
        let actions = self.properties.end_scrub(commit);
        self.apply_properties(actions);
    }
    /// Adopt the workspace's one media cache instance for timeline display
    /// reads and generation. The same instance is lent to the MCP inbox.
    pub fn set_media(
        &mut self,
        media: std::sync::Arc<std::sync::Mutex<engine::control::MediaAccess>>,
    ) {
        self.properties
            .attach_luts(media.clone(), self.playback.budget().clone());
        self.properties
            .attach_footage(media.clone(), self.playback.budget().clone());
        self.playback.set_footage(media.clone());
        self.dialogs.scene_split.set_media(media.clone());
        self.timeline.set_media(media);
        self.timeline.set_proxies(self.playback.proxies().cloned());
    }

    /// Attach the native effect adapter to playback and thumbnail jobs, sharing
    /// playback's byte budget. Failure leaves the thumbnail composition unchanged.
    pub fn set_effects(
        &mut self,
        effects: Arc<dyn render::effects::EffectCompositor>,
    ) -> Result<()> {
        let previews = assets::State::effect_previews_for(self.playback.budget(), effects.clone())?;
        self.playback.set_effects(effects)?;
        self.assets.effect_previews = Some(previews);
        Ok(())
    }

    /// Attach the app-shared saved library. One library handle is shared by every
    /// project window; each editor owns only its search/preview presentation.
    pub fn set_sound_library(
        &mut self,
        library: Arc<libraries::sounds::SoundLibrary>,
    ) -> Result<()> {
        self.with_assets(|assets, port| assets.set_sound_library(port, library))
    }

    /// Read the Stickers catalogue and its downloads from `store` instead of the per-user store.
    /// Tests and audit scenes use it with a store rooted in a temporary directory.
    pub fn set_sticker_store(&mut self, store: Arc<libraries::stickers::StickerStore>) {
        self.assets.sticker_store = Some(store);
    }

    /// Record why the app-shared Sounds library did not open; its section shows
    /// the reason with a Retry row.
    pub fn set_sound_open_error(&mut self, error: &engine::SoundError) {
        self.assets.sounds_open = Some(LibraryProblem::from_sound(error));
    }

    /// Record why the app-shared Music library did not open.
    pub fn set_music_open_error(&mut self, error: &engine::MusicError) {
        self.assets.music_open = Some(LibraryProblem::from_music(error));
    }

    /// Why the Sounds library did not open, when it did not.
    pub fn sound_open_problem(&self) -> Option<LibraryProblem> {
        self.assets.sounds_open
    }

    /// Why the Music library did not open, when it did not.
    pub fn music_open_problem(&self) -> Option<LibraryProblem> {
        self.assets.music_open
    }

    /// Stand-in for a section's Retry in tests: ask the lending owner to open this
    /// store again. The request is drained by the next frame's tick.
    #[cfg(test)]
    pub(crate) fn request_library_reopen(&mut self, store: LibraryStore) {
        self.assets.request_library_reopen(store);
    }

    /// Take the pending re-open requests; only the lending owner calls this.
    pub fn take_library_reopens(&mut self) -> Vec<LibraryStore> {
        std::mem::take(&mut self.assets.library_reopens)
    }

    /// Injection boundary for tests and audit scenes.
    pub fn set_sounds(&mut self, sounds: Sounds) {
        self.assets.set_sounds(sounds);
    }

    /// Attach the process-shared music session. One library and credential are
    /// shared by every project window; each editor owns only its presentation.
    pub fn set_music_session(
        &mut self,
        session: Arc<crate::services::assets::music::MusicSession>,
    ) -> Result<()> {
        self.with_assets(|assets, port| assets.set_music_session(port, session))
    }

    /// Injection boundary for tests and audit scenes.
    pub fn set_music(&mut self, music: Music) {
        self.assets.set_music(music);
    }

    /// Project-owned media directory used when a track is added.
    pub fn set_music_media_dir(&mut self, directory: PathBuf) {
        self.assets.set_music_media_dir(directory);
    }

    /// Build the captions panel without any download or provider construction.
    pub fn prepare_subtitles(&mut self) -> Result<()> {
        let budget = self.playback.budget().clone();
        self.assets.prepare_captions(&budget)
    }

    /// Injection boundary for tests: replace or clear the captions panel.
    pub fn set_subtitles(&mut self, panel: Option<Subtitles>) {
        self.assets.captions = panel;
    }

    /// The captions panel, once the Captions tab or a caption action built it.
    pub fn subtitles_mut(&mut self) -> Option<&mut Subtitles> {
        self.assets.captions.as_mut()
    }

    /// Run `run` on the captions panel with the session and the persisted options.
    pub fn with_subtitles<T>(
        &mut self,
        run: impl FnOnce(
            &mut Subtitles,
            &mut engine::Session,
            &crate::preferences::SubtitleOptions,
        ) -> T,
    ) -> Option<T> {
        let panel = self.assets.captions.as_mut()?;
        Some(run(panel, &mut self.session, &self.preferences.subtitles))
    }

    /// Project-owned media directory used when a library sound is added.
    pub fn set_sound_media_dir(&mut self, directory: PathBuf) {
        self.assets.set_sound_media_dir(directory);
    }

    /// Complete the source picker through the captured properties workflow.
    pub fn lut_picked_file(&mut self, path: Option<std::path::PathBuf>) -> Result<()> {
        self.with_properties(|state, port| state.lut_picked_file(port, path))
    }

    pub fn new(session: Session, playback: Playback, preferences: Preferences) -> Self {
        Self::with_session(
            crate::services::lifecycle::EditorSession::Memory(Box::new(session)),
            playback,
            preferences,
        )
    }
    /// A transient preview editor draws with the caller's appearance and
    /// defaults for every other preference; the shorts window owns its state.
    pub fn preview(session: Session, playback: Playback, look: crate::kit::Look) -> Self {
        let preferences = Preferences {
            locale: look.locale,
            dark: look.dark,
            ..Preferences::default()
        };
        Self::new(session, playback, preferences)
    }
    fn with_session(
        mut session: crate::services::lifecycle::EditorSession,
        playback: Playback,
        preferences: Preferences,
    ) -> Self {
        session.set_asset_export_sources(playback.export_sources());
        // Opens the sticker cache and lends it to the renderer for this session's preview and export.
        libraries::stickers::native_store();
        let font_error = libraries::fonts::FontStore::native()
            .and_then(|store| store.load_project(session.project()))
            .err()
            .map(|_| i18n::text(preferences.locale, Text::FontsLoadFailed).to_owned());
        let view = session.metadata().view;
        let preferences_fit = preferences.preview_fit;
        let preferences_zoom = preferences.preview_zoom;
        let project = session.project().data().id;
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
        let mut asset_state = assets::State::default();
        asset_state.effect_previews = EffectPreviews::cpu().ok();
        Self {
            session,
            playback,
            preferences,
            overlay_open: false,
            selection_scopes: SelectionScopes::default(),
            properties: properties::State::default(),
            frame,
            player,
            timeline: {
                let mut state = timeline::State::default();
                state.adopt_stored_view(view.zoom, view.scroll);
                state.restore_lanes(timeline_lanes);
                state
            },
            error: font_error,
            autosaved: None,
            persistence_error: None,
            configured: false,
            lifecycle_action: None,
            clock: Arc::new(SystemClock::default()),
            toast: None,
            assets: asset_state,
            dialogs: dialogs::State::default(),
            copy_snapshot_requested: false,
            requests: Vec::new(),
            clip_downloads: None,
            clip_job: None,
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
            crate::services::lifecycle::EditorSession::Persistent(Box::new(autosave)),
            playback,
            preferences,
        )
    }
    pub fn inspect(&self) -> Result<Inspection> {
        let media_order = self.assets.media_order(&self.session, &self.preferences);
        let media_highlight = self.assets.media_highlight();
        let media_delete_pending = self.assets.media_delete_pending();
        Ok(Inspection {
            widgets: self.frame.widgets.values().cloned().collect(),
            view: "editor",
            playing: self.playback.playing(),
            position: self.playback.position()?,
            selection: self.session.selection().iter().copied().collect(),
            selection_scope: self.selection_scopes.active(self.session.selection()),
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
            importing: self.assets.importing(),
            import_completed: self.assets.import_completed,
            import_total: self.assets.import_total,
            import_imported: self.assets.import_imported,
            import_unfinished: self
                .assets
                .import_total
                .saturating_sub(self.assets.import_completed),
            import_cancelled: self.assets.import_cancelled,
            import_failures: self
                .assets
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
            import_notice: self.assets.import_notice.clone(),
            import_error: self.assets.import_error.clone(),
            import_toast: self.toast.as_ref().map(|toast| match &toast.detail {
                Some(detail) => format!("{} · {detail}", toast.text),
                None => toast.text.clone(),
            }),
            media_selection: self.assets.media_selection().iter().copied().collect(),
            assets: self
                .session
                .project()
                .data()
                .assets
                .iter()
                .filter(|asset| !asset.ephemeral())
                .map(|asset| {
                    let state = self
                        .assets
                        .asset_states
                        .get(&asset.id())
                        .copied()
                        .unwrap_or(AssetState::Probing);
                    AssetInspection {
                        id: asset.id(),
                        state: state.code(),
                        thumbnail: self.assets.thumbnails.contains_key(&asset.id()),
                        detail: match state {
                            AssetState::Unsupported(reason) => Some(reason.label().to_owned()),
                            _ => None,
                        },
                    }
                })
                .chain(
                    self.assets
                        .pending_imports
                        .keys()
                        .map(|id| AssetInspection {
                            id: *id,
                            state: "probing",
                            thumbnail: false,
                            detail: None,
                        }),
                )
                .collect(),
            effect_previews: self
                .assets
                .effect_previews
                .as_ref()
                .map_or_else(BTreeMap::new, |previews| previews.statuses()),
            media_view: self.preferences.media_view,
            media_sort: self.preferences.media_sort,
            media_order,
            media_highlight,
            replace_request: self.timeline.replace_request(),
            media_delete_pending,
            sounds: self.assets.sounds.as_ref().map(|sounds| sounds.view()),
            music: self.assets.music.as_ref().map(|music| music.view()),
            export_options: self.preferences.export,
            export: self.dialogs.export.inspect(),
            crop: self
                .player
                .crop
                .as_ref()
                .map(panels::player::crop::Dialog::inspect),
            subtitles: self
                .assets
                .captions
                .as_ref()
                .map(|s| s.inspect(&self.session)),
            models: self.dialogs.models.inspect(),
            voiceover: self.dialogs.voiceover.view(),
            keybinding_issues: self.frame.keybinding_issues.clone(),
            color_preview: self.properties.color_preview(),
            key_color_pick: self.player.key_color_pick.as_ref().map(|pick| pick.item),
            matte_brush: self
                .player
                .matte_brush
                .map(|brush| (brush.item, brush.mode)),
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
    /// Show a creator-facing toast for five seconds.
    pub(in crate::editor) fn set_toast(&mut self, text: &str, detail: Option<String>) {
        self.toast = Some(Toast {
            text: text.to_owned(),
            detail,
            until: self.clock.now() + Duration::from_secs(5),
        });
    }
    pub(in crate::editor) fn truncate_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| self.clock.now() >= toast.until)
        {
            self.toast = None;
        }
    }
    /// Start the export sheet with typed caller settings without mutating
    /// editable project content.
    pub(in crate::editor) fn run_export(
        &mut self,
        options: dialogs::export::ExportOptions,
    ) -> Result<()> {
        if self.session.transaction_active() {
            return Err(Error::Engine(engine::control::Error::Session(
                engine::SessionError::TransactionActive,
            )));
        }
        let project = self.session.project().clone();
        let budget = self.playback.budget().clone();
        let render = self.playback.render_state()?;
        self.dialogs.export.run(
            &project,
            options,
            budget,
            render,
            self.preferences.hardware_acceleration.encode_preference(),
        )
    }
    /// Apply a shortcut file chosen in the native picker, merging or replacing it.
    pub fn keybindings_import_picked(
        &mut self,
        path: std::path::PathBuf,
        replace: bool,
    ) -> Result<()> {
        self.with_dialogs(|state, port| {
            dialogs::import_shortcuts(&mut state.preferences, port, &path, replace)
        })
    }
    /// Write the canonical shortcut profile to the picked destination.
    pub fn keybindings_export_picked(&mut self, path: std::path::PathBuf) -> Result<()> {
        self.with_dialogs(|state, port| {
            dialogs::export_shortcuts(&mut state.preferences, port, &path)
        })
    }
    /// Open or raise the export dialog without changing project content.
    pub(in crate::editor) fn open_export(&mut self) -> Result<()> {
        let budget = self.playback.budget().clone();
        let render = self.playback.render_state()?;
        let project = self.session.project().clone();
        self.dialogs.export.open(budget, render, &project)
    }
    /// Accept the native export save picker's result without opening OS UI in tests.
    /// The folder picker chose `directory`: start `export.clips` for the queued assets.
    pub fn export_clips_picked(&mut self, directory: PathBuf) -> Result<()> {
        if let Err(error) = self.assets.start_clip_export(&mut self.session, directory) {
            let message = self.tr(crate::i18n::Text::ClipExportRefused);
            self.set_toast(message, Some(error.to_string()));
        }
        Ok(())
    }
    /// Start `export.clip` for the destination the save dialog picked. The job runs on
    /// the editor's own runner; `poll_clip_download` announces its end.
    pub fn clip_download_picked(
        &mut self,
        item: ItemId,
        mode: engine::ClipExportMode,
        path: PathBuf,
    ) -> Result<()> {
        if let Err(error) = self.start_clip_download(item, mode, path) {
            let message = self.tr(Text::ClipDownloadFailed);
            self.set_toast(message, Some(error.to_string()));
        }
        Ok(())
    }
    fn start_clip_download(
        &mut self,
        item: ItemId,
        mode: engine::ClipExportMode,
        path: PathBuf,
    ) -> Result<()> {
        let budget = self.playback.budget().clone();
        let runner = match &mut self.clip_downloads {
            Some(runner) => runner,
            slot @ None => slot.insert(JobRunner::new(
                RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 1,
                    retained: 1,
                    network_attempts: 1,
                    backoff: Duration::from_secs(1),
                    progress_interval: Duration::from_millis(20),
                },
                budget,
                Arc::new(SystemClock::default()),
            )?),
        };
        // The native dialog already asked about replacing an existing file.
        let result = crate::services::ai_access::run(
            &mut self.session,
            runner,
            None,
            None,
            "export.clip",
            serde_json::json!({"clip_id": item, "mode": mode, "path": path, "overwrite": true}),
        )?;
        let (job, _) = crate::services::ai_access::accepted(result)?;
        self.clip_job = Some(job);
        Ok(())
    }
    /// Tick the download runner and announce the running `export.clip` job once it ends.
    pub(in crate::editor) fn poll_clip_download(&mut self) {
        let (Some(runner), Some(job)) = (self.clip_downloads.as_mut(), self.clip_job) else {
            return;
        };
        let state = match runner.tick() {
            Ok(()) => runner
                .snapshot(job)
                .map_or(JobState::Failed, |snapshot| snapshot.state),
            Err(_) => JobState::Failed,
        };
        let key = match state {
            JobState::Completed => Text::ClipDownloaded,
            JobState::Failed | JobState::Cancelled => Text::ClipDownloadFailed,
            _ => return,
        };
        self.clip_job = None;
        let message = self.tr(key);
        self.set_toast(message, None);
    }
    /// The folder picker was dismissed: the queued assets are not exported.
    pub fn export_clips_cancelled(&mut self) {
        self.assets.cancel_clip_pick();
    }
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let protected = self
            .session
            .autosave()
            .map(|s| s.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        self.dialogs.export.save_to(path, protected)
    }
    /// Save the caption output the export dialog asked for; the captions panel runs it.
    pub fn caption_output_picked(&mut self, path: PathBuf, output: CaptionOutput) -> Result<()> {
        self.with_assets(|assets, port| assets.export_caption_output(port, path, output));
        Ok(())
    }
    /// Parse the SRT/VTT file the Add captions picker chose, at the playhead it was asked from.
    pub fn subtitles_import_picked(&mut self, path: PathBuf, at: Tick) -> Result<()> {
        self.with_assets(|assets, port| assets.import_subtitle_picked(port, path, at));
        Ok(())
    }
    /// Complete the font picker's file; `None` drops the pending import.
    pub fn font_import_picked(&mut self, path: Option<PathBuf>) -> Result<()> {
        self.with_properties(|state, port| state.font_import_picked(port, path))
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
    /// Injection boundary for tests: the input host and the project-owned directory
    /// the voice-over dialog records into. Without it the dialog uses the operating
    /// system's default input and the project's media folder.
    pub fn set_voiceover(
        &mut self,
        host: Arc<dyn crate::services::assets::voiceover::InputHost>,
        directory: PathBuf,
    ) {
        self.dialogs.voiceover.set(host, directory);
    }
    /// Instantiate the one shared model cache on first use; it never fetches then.
    pub fn open_models(&mut self) -> Result<()> {
        self.dialogs.models.open()
    }
    /// Open the manager on a specific missing model, as `/09` does.
    pub fn open_model(&mut self, id: &str) -> Result<()> {
        self.dialogs.models.open_model(id)
    }
    /// Lend the process models cache the model manager opens over.
    pub fn set_model_cache(
        &mut self,
        cache: std::sync::Arc<crate::services::ai_access::ModelCache>,
    ) {
        self.dialogs.models.set_cache(cache);
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
    /// Hold the selected video item's frame at the playhead for the default hold.
    pub(in crate::editor) fn freeze_selected_frame(&mut self) -> Result<()> {
        let item = self
            .freeze_target()
            .ok_or(Error::Invalid("freeze selection"))?;
        let at = self.playback.position()?;
        // Frozen stills live in the project media folder beside sounds, music and takes.
        let dir = project::media_dir(self.session.saved_path());
        let protected = self
            .session
            .autosave()
            .map(|autosave| autosave.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        let budget = self.playback.budget().clone();
        let hold = time::Tick(FREEZE_HOLD_TICKS);
        let held = crate::services::assets::freeze::hold(
            &mut self.session,
            budget,
            item,
            at,
            hold,
            &dir,
            protected,
        );
        match held {
            Ok(_) => {
                self.set_toast(self.tr(Text::FreezeDone), None);
                Ok(())
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error)
            }
        }
    }
    /// The clip whose Replace media request waits for the window's native picker; taking it
    /// hands the request over.
    pub(crate) fn take_replace_request(&mut self) -> Option<ItemId> {
        self.timeline.take_replace_request()
    }
    /// Replace one clip's source with a file that is already a project asset. A file
    /// that has not been imported refuses, so the timeline never gains an unlisted source.
    pub fn replace_picked_file(&mut self, item: ItemId, path: PathBuf) -> Result<()> {
        let asset = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .find(|asset| asset.path() == path.as_path())
            .map(SourceAsset::id)
            .ok_or(Error::Invalid("import the file before replacing"))?;
        self.apply_result(Command::ReplaceMedia {
            items: vec![item],
            asset,
        })
        .map(|_| ())
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
                self.fail(error.to_string());
                Err(error)
            }
        }
    }
    pub fn snapshot_save_cancelled(&mut self) {
        self.player.snapshot.cancel();
    }
    /// Tests and non-window callers consume the same lifecycle request the
    /// workspace turns into the native picker.
    pub fn take_lifecycle_action(&mut self) -> Option<crate::services::lifecycle::EditorAction> {
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
        let corrections = self
            .playback
            .footage_corrections(self.session.project())
            .unwrap_or_default()
            .corrections();
        if let Err(error) = crate::services::preview::copy(&self.session, at, &corrections, ctx) {
            self.player.snapshot.error = Some(match &error {
                Error::Engine(engine::control::Error::Artifacts(
                    engine::ArtifactError::EmptyProject,
                )) => engine::control::ErrorCode::EmptyProject,
                _ => engine::control::ErrorCode::Failed,
            });
            self.fail(error.to_string());
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
    /// Render the assets surface in isolation for offscreen design audits.
    pub fn assets_ui(&mut self, ui: &mut egui::Ui) {
        Self::fonts(ui.ctx());
        icons::install(ui.ctx());
        Self::theme(ui.ctx(), self.preferences.dark);
        crate::kit::install_editor_density(ui.ctx());
        self.frame.widgets.clear();
        self.tick();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::panel(self.preferences.dark));
        self.frame.panels.insert("media", ui.max_rect());
        self.show_assets(ui);
        self.show_import_overlay(ui.ctx());
    }
    /// Replace the OS file-clipboard reader that Paste consults; tests inject a fixture.
    pub fn set_os_clipboard_files(&mut self, read: impl FnMut() -> Vec<PathBuf> + Send + 'static) {
        self.assets.os_files = crate::services::clipboard::FileClipboard::new(read);
    }
    /// Read the platform clipboard's files on Paste. The desktop wires this once per window.
    pub fn use_native_file_clipboard(&mut self) {
        self.assets.os_files = crate::services::clipboard::FileClipboard::native();
    }
    /// Replace the media boundary in tests; production keeps the media owner.
    pub fn set_import_probe(&mut self, probe: Arc<dyn ImportProbe>) -> Result<()> {
        self.with_assets(|assets, port| assets.set_import_probe(port, probe))
    }
    /// Replace thumbnail decoding in tests; production reuses media's decoder.
    pub fn set_thumbnailer(&mut self, thumbnailer: Arc<dyn ThumbnailSource>) -> Result<()> {
        self.with_assets(|assets, port| assets.set_thumbnailer(port, thumbnailer))
    }
    /// Persistent content-keyed thumbnail directory. Without one thumbnails are
    /// memoized for this session only.
    pub fn set_thumbnail_root(&mut self, root: PathBuf) {
        self.assets.set_thumbnail_root(root);
    }
    pub fn importing(&self) -> bool {
        self.assets.importing()
    }
    /// Accept a native drag-drop batch through the same path as the picker.
    pub fn drop_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.start_import(paths)
    }
    pub fn start_import(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.with_assets(|assets, port| assets.start_import(port, paths))
    }
    /// Replace the media selection; tests and agents use the same state as clicks.
    pub fn select_media(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        self.assets.select_media(ids);
    }
    /// App action `media.reveal`: switch to Media, scroll and highlight for 1.5 s.
    pub fn reveal_media(&mut self, asset: AssetId) -> Result<()> {
        self.with_assets(|assets, port| assets.reveal_media(port, asset))
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
fn display_label(value: &str, limit: usize) -> String {
    let mut label: String = value.chars().take(limit.saturating_add(1)).collect();
    if label.chars().count() > limit {
        label.pop();
        label.push('…');
    }
    label
}
