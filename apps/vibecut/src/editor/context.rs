//! Lends editor areas the shared read context and the one write context.

#[cfg(test)]
pub(crate) mod bench;
pub(crate) mod visuals;

use crate::{
    actions, egui,
    i18n::{Locale, Text},
    services::playback::Playback,
};

pub(crate) use visuals::VisualQuery;

/// Human-only desktop cache intentions, outside editing/agent commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheAction {
    Refresh,
    Clear,
    Cancel,
    Retry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CachePhase {
    Unavailable,
    Idle,
    Inspecting,
    Clearing,
    Cancelling,
    Failed,
}
/// Human-only preview-proxy intentions. The choice itself is session
/// presentation state; generation is a desktop job beside the cache controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxyAction {
    Toggle(bool),
    Generate,
    Cancel,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ProxyPhase {
    #[default]
    Idle,
    Generating,
    Failed,
}
/// Proxy generation as the Settings tab shows it.
#[derive(Clone, Default)]
pub(crate) struct ProxyUi {
    pub(crate) phase: ProxyPhase,
    /// Of the sources queued by the last request, how many finished.
    pub(crate) done: usize,
    pub(crate) total: usize,
    /// Progress of the source being encoded, 0..=1.
    pub(crate) progress: f32,
    pub(crate) cause: Option<std::sync::Arc<engine::ArtifactError>>,
    /// The proxies preview may read; `None` when the app composes none.
    pub(crate) index: Option<engine::proxies::ProxyIndex>,
}
/// Neutral Settings projection; worker and cache custody stay in the media owner.
#[derive(Clone)]
pub(crate) struct CacheUi {
    pub(crate) proxy: ProxyUi,
    pub(crate) phase: CachePhase,
    pub(crate) report: Option<std::sync::Arc<engine::cache::CacheMaintenanceReport>>,
    pub(crate) cause: Option<std::sync::Arc<engine::control::Error>>,
    pub(crate) checked: Option<std::time::Instant>,
}
impl Default for CacheUi {
    fn default() -> Self {
        Self {
            proxy: ProxyUi::default(),
            phase: CachePhase::Unavailable,
            report: None,
            cause: None,
            checked: None,
        }
    }
}
/// Read-only composition inputs shared by editor areas.
pub(crate) struct Context<'a> {
    pub(crate) playback: &'a Playback,
    pub(crate) keybindings: &'a actions::KeybindingPreferences,
    pub(crate) session: &'a engine::Session,
    pub(crate) locale: Locale,
    pub(crate) dark: bool,
    pub(crate) preferences: &'a crate::preferences::Preferences,
    /// What the selection currently scopes: items, mask points or keyframes.
    pub(crate) scope: crate::editor::selection::SelectionScope,
    /// The selected keyframes, flat across items and channels.
    pub(crate) keyframes:
        &'a std::collections::BTreeSet<crate::editor::selection::KeyframeSelection>,
    /// The selected mask points; the app owns this one canonical set.
    pub(crate) mask_points:
        &'a std::collections::BTreeSet<crate::editor::selection::MaskPointSelection>,
    /// The selected timeline transition the inspector edits instead of items.
    pub(crate) transition: Option<project::TransitionId>,
    pub(crate) save_status: Text,
    /// Playback position of the shared transport.
    pub(crate) playhead: time::Tick,
    /// Preview-axis pointer time; `None` follows the playhead.
    pub(crate) hover: Option<time::Tick>,
    pub(crate) ripple: bool,
    /// Last presented preview frame, for the export dialog's poster.
    pub(crate) preview: Option<&'a egui::TextureHandle>,
    /// Cache-only clip visual queries (thumbnail strip and waveform).
    pub(crate) visuals: &'a dyn VisualQuery,
    pub(crate) cache: CacheUi,
    /// Registered action ids the editor currently refuses.
    pub(crate) disabled: std::collections::BTreeSet<&'static str>,
    /// While an effect is being dragged: the clips that accept it.
    pub(crate) effect_targets: Option<std::collections::BTreeSet<project::ItemId>>,
    /// Armed chroma-key eyedropper; the player samples its next click.
    pub(crate) key_color_pick: Option<crate::editor::KeyPick>,
}
impl Context<'_> {
    /// The selected items; the engine session is their only owner.
    pub(crate) fn selection(&self) -> &std::collections::BTreeSet<project::ItemId> {
        self.session.selection()
    }
    /// Tooltip text from the action's first effective persisted chord.
    pub(crate) fn shortcut(&self, action: &str) -> Option<String> {
        shortcut_text(&self.preferences.keybindings, action)
    }
    /// One frame of the canvas rate, at least one tick.
    pub(crate) fn frame_ticks(&self) -> time::Tick {
        frame_ticks(self.session)
    }
    /// The project the timeline draws: a staged preview when one is open,
    /// otherwise the committed snapshot. Other areas keep the committed view.
    pub(crate) fn staged_project(&self) -> &std::sync::Arc<project::Project> {
        self.session
            .preview_project()
            .unwrap_or_else(|| self.session.project())
    }
    pub(crate) fn tr(&self, key: Text) -> &'static str {
        crate::i18n::text(self.locale, key)
    }
}

/// Read-only view a workflow reads the editor through while it holds the desk.
pub(in crate::editor) struct View<'a> {
    pub(in crate::editor) budget: &'a engine::ByteBudget,
    pub(in crate::editor) session: &'a engine::Session,
    pub(in crate::editor) preferences: &'a crate::preferences::Preferences,
    pub(in crate::editor) playhead: time::Tick,
}
impl View<'_> {
    /// The selected items; the engine session is their only owner.
    pub(in crate::editor) fn selection(&self) -> &std::collections::BTreeSet<project::ItemId> {
        self.session.selection()
    }
}

/// Tooltip text from an action's first effective chord.
pub(in crate::editor) fn shortcut_text(
    bindings: &actions::KeybindingPreferences,
    action: &str,
) -> Option<String> {
    let spec = actions::action(action)?;
    let chords = bindings.effective_chords(spec);
    let chord = actions::Chord::parse(chords.first()?).ok()?;
    Some(chord.display(actions::Platform::current()))
}

/// One frame of the project's canvas rate, at least one tick.
fn frame_ticks(session: &engine::Session) -> time::Tick {
    session
        .project()
        .data()
        .canvas
        .fps
        .frame_duration()
        .ok()
        .filter(|frame| *frame > time::Tick::ZERO)
        .unwrap_or(time::Tick(1))
}

/// The single selected item, if exactly one is selected.
pub(in crate::editor) fn single(
    selection: &std::collections::BTreeSet<project::ItemId>,
) -> Option<project::ItemId> {
    if selection.len() != 1 {
        return None;
    }
    selection.iter().next().copied()
}

/// The snapping rank the timeline and the preview canvas share: the smallest
/// pixel distance wins and equal distances break by the stable source order.
/// Each caller keeps its own threshold in the coordinate space it measures.
pub(in crate::editor) fn closest<T: Copy, S: Copy + Ord>(
    candidates: impl IntoIterator<Item = (T, S, f64)>,
) -> Option<(T, S, f64)> {
    let mut best: Option<(T, S, f64)> = None;
    for (at, source, pixels) in candidates {
        let better = best.is_none_or(|(_, best_source, best_pixels)| {
            pixels < best_pixels || (pixels == best_pixels && source < best_source)
        });
        if better {
            best = Some((at, source, pixels));
        }
    }
    best
}

/// Decoded thumbnails shared by every area, uploaded to the GPU on first use.
pub(crate) struct Thumbnails<'a> {
    pub(crate) images:
        &'a std::collections::BTreeMap<media::AssetId, engine::thumbnails::RetainedThumbnail>,
    pub(crate) textures:
        &'a std::cell::RefCell<std::collections::BTreeMap<media::AssetId, egui::TextureHandle>>,
}
impl Thumbnails<'_> {
    /// Upload one decoded thumbnail on first use; later frames reuse the handle.
    pub(crate) fn texture(
        &self,
        ctx: &egui::Context,
        id: media::AssetId,
    ) -> Option<egui::TextureHandle> {
        let mut textures = self.textures.borrow_mut();
        if let Some(texture) = textures.get(&id) {
            return Some(texture.clone());
        }
        let thumbnail = self.images.get(&id)?;
        // One retained image pays for one CPU image and GPU upload, even when
        // several library records refer to the same content/frame.
        if let Some(texture) = textures.iter().find_map(|(other, texture)| {
            self.images
                .get(other)
                .filter(|image| image.shares_pixels(thumbnail))
                .map(|_| texture.clone())
        }) {
            textures.insert(id, texture.clone());
            return Some(texture);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [thumbnail.width as usize, thumbnail.height as usize],
            &thumbnail.pixels,
        );
        let texture = ctx.load_texture(
            format!("media.thumbnail.{}", id.0),
            image,
            egui::TextureOptions::LINEAR,
        );
        textures.insert(id, texture.clone());
        Some(texture)
    }
}

/// A cross-area request the frame drains after the area that raised it returns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Request {
    /// Open the LUT source picker after the properties area returns.
    ImportLut,
    /// Move the shared transport.
    Seek(time::Tick),
    Cache(CacheAction),
    Proxy(ProxyAction),
    /// Open the model manager on a missing model.
    OpenModel(String),
}

/// The shared editor context. One area receives it for one frame; its edit
/// entry applies the command, reports the failure and resyncs the preview once
/// per committed change.
pub(crate) struct Desk<'a> {
    editor: &'a mut crate::editor::Editor,
}

/// The editor's ripple preference as a command: an edit or replacement preview is
/// wrapped in `timeline.edit` so the policy never rides on session state; a
/// `batch` becomes the policy's own command list. Library audio and every non-edit
/// command carry no policy.
fn ripple_policy(command: engine::Command, ripple: bool) -> engine::Command {
    use engine::{Command, Effect};
    match command {
        Command::Batch { commands } => Command::TimelineEdit { commands, ripple },
        command @ Command::TimelinePreview { .. } => Command::TimelineEdit {
            commands: vec![command],
            ripple,
        },
        command
            if command.effect() == Effect::Edit
                && !matches!(
                    command,
                    Command::TimelineEdit { .. }
                        | Command::AddLibrarySound { .. }
                        | Command::AddLibraryMusic { .. }
                ) =>
        {
            Command::TimelineEdit {
                commands: vec![command],
                ripple,
            }
        }
        command => command,
    }
}

impl<'a> Desk<'a> {
    pub(in crate::editor) fn new(editor: &'a mut crate::editor::Editor) -> Self {
        Self { editor }
    }

    /// Show a creator-facing failure on the editor's error line.
    pub(in crate::editor) fn fail(&mut self, message: impl Into<String>) {
        self.editor.error = Some(message.into());
    }

    /// Clear the error line after a successful retry.
    pub(in crate::editor) fn clear_error(&mut self) {
        self.editor.error = None;
    }

    /// Resync the preview with the current project snapshot.
    pub(in crate::editor) fn sync(&mut self) {
        self.editor.sync_preview();
    }

    /// The one selection write. The engine session owns the set; it refuses a
    /// change while a gesture is open or when an item is gone, and the refusal
    /// shows on the error line with the selection left as it was.
    pub(in crate::editor) fn select(
        &mut self,
        items: std::collections::BTreeSet<project::ItemId>,
    ) -> bool {
        match self.editor.session.set_selection(items) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Apply one typed command without reporting or resyncing.
    pub(in crate::editor) fn run_command(
        &mut self,
        command: engine::Command,
        ripple: bool,
    ) -> std::result::Result<engine::Outcome, engine::EditError> {
        self.editor.session.apply(ripple_policy(command, ripple))
    }

    /// Run one registered action through the editor's shared activation path.
    pub(in crate::editor) fn run_action(&mut self, id: &str) -> crate::Result<()> {
        self.editor.activate(id)
    }

    /// The transaction entry for callers that propagate the failure.
    pub(in crate::editor) fn transaction_result<T>(
        &mut self,
        run: impl FnOnce(&mut engine::Session) -> engine::control::Result<T>,
    ) -> crate::Result<T> {
        let result = self.editor.session.transaction(run);
        match result {
            Ok(value) => {
                self.sync();
                Ok(value)
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error.into())
            }
        }
    }

    /// Stage a preview and always show it; a reversal must restore the entry
    /// frame even when the engine reports no net change.
    pub(in crate::editor) fn preview(&mut self, command: engine::Command, ripple: bool) -> bool {
        match self.run_command(command, ripple) {
            Ok(_) => {
                self.sync();
                true
            }
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// The typed-command entry for callers that propagate the failure.
    pub(in crate::editor) fn apply_result(
        &mut self,
        command: engine::Command,
        ripple: bool,
    ) -> crate::Result<engine::Outcome> {
        let before = self.editor.session.project().clone();
        match self.editor.session.apply(ripple_policy(command, ripple)) {
            Ok(outcome) => {
                if outcome.changed
                    || !std::sync::Arc::ptr_eq(&before, self.editor.session.project())
                {
                    self.sync();
                }
                Ok(outcome)
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error.into())
            }
        }
    }

    /// Run one registered action, reporting a failure.
    pub(in crate::editor) fn act(&mut self, id: &str) -> bool {
        match self.run_action(id) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Record one preference edit in the app's own history, reporting a failure.
    pub(in crate::editor) fn set_preference(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) -> bool {
        match self.set_preference_result(key, value) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Record one preference edit in the app's own history, for callers that
    /// propagate the failure.
    pub(in crate::editor) fn set_preference_result(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) -> crate::Result<()> {
        self.editor.set_preference(key, value)
    }

    /// Queue a cross-area request; the frame drains it after this area returns.
    pub(in crate::editor) fn request(&mut self, request: Request) {
        self.editor.requests.push(request);
    }

    /// The editor, for reads and state the edit entry does not own.
    pub(in crate::editor) fn editor(&mut self) -> &mut crate::editor::Editor {
        self.editor
    }

    /// Lend the read view for one workflow step.
    pub(in crate::editor) fn view(&self) -> View<'_> {
        View {
            budget: self.editor.playback.budget(),
            session: &self.editor.session,
            preferences: &self.editor.preferences,
            playhead: self.editor.playback.position().unwrap_or(time::Tick::ZERO),
        }
    }

    /// Run one catalog operation against the editable session.
    pub(in crate::editor) fn with_session<T>(
        &mut self,
        run: impl FnOnce(&mut engine::Session) -> crate::Result<T>,
    ) -> crate::Result<T> {
        run(&mut self.editor.session)
    }

    /// Arm or leave the chroma-key eyedropper the player samples next.
    pub(in crate::editor) fn arm_key_color_pick(&mut self, pick: Option<crate::editor::KeyPick>) {
        self.editor.player.key_color_pick = pick;
    }

    /// Show a creator-facing toast for five seconds.
    pub(in crate::editor) fn toast(&mut self, text: &str, detail: Option<String>) {
        self.editor.set_toast(text, detail);
    }

    /// Select exactly one item, clearing every other selection.
    pub(in crate::editor) fn select_single(&mut self, item: project::ItemId) {
        self.editor.select_single(item);
    }

    /// Persist one section's open state as one preference edit.
    pub(in crate::editor) fn set_section_open(
        &mut self,
        key: &str,
        open: bool,
    ) -> crate::Result<()> {
        self.editor.set_section_open(key, open)
    }

    /// Write one preference without an app-history entry.
    pub(in crate::editor) fn store_preference(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) -> crate::Result<()> {
        self.editor.preferences.set(key, value)?;
        Ok(())
    }

    /// Adopt a fully validated preference candidate after its project edit
    /// succeeds, without adding an app-history entry.
    pub(in crate::editor) fn adopt_preferences(
        &mut self,
        preferences: crate::preferences::Preferences,
    ) {
        self.editor.preferences = preferences;
    }

    /// Show a draft project at `at` without touching the editable session.
    pub(in crate::editor) fn stage_frame(
        &mut self,
        at: time::Tick,
        project: std::sync::Arc<project::Project>,
    ) -> crate::Result<()> {
        self.editor.playback.seek(at, false, project)?;
        Ok(())
    }
}
