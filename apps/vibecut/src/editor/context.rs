//! Lends editor areas the shared read context and the one write context.

mod port;
pub(crate) mod visuals;

use crate::{
    actions, egui,
    i18n::{Locale, Text},
    services::playback::Playback,
};

pub(in crate::editor) use port::{Note, Port, Read};

/// The persistence state the header shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SaveStatus {
    /// The last save failed.
    Failed,
    /// A save is pending or in flight.
    Saving,
    /// A memory session that nothing writes to disk.
    Unsaved,
    /// Autosaved; `clock` is the local `HH:MM:SS` the journal last took a change.
    Saved { clock: Option<String> },
}
pub(crate) use visuals::VisualQuery;

/// A Media-panel drag in flight: the dragged assets and the track kind they insert as.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MediaDrag {
    pub(crate) assets: Vec<media::AssetId>,
    pub(crate) kind: project::TrackKind,
}

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

impl CacheUi {
    /// The typed warning both cache controls show for the last failure or the
    /// last report's cause; `None` when the cache is healthy.
    pub(crate) fn problem(&self) -> Option<Text> {
        let cause = self.cause.as_deref();
        let runtime = match cause {
            Some(engine::control::Error::Runtime(error)) => Some(error),
            Some(_) => None,
            None => self
                .report
                .as_ref()
                .and_then(|report| report.cause.as_ref()),
        };
        if cause.is_none() && runtime.is_none() {
            return None;
        }
        if self
            .report
            .as_ref()
            .is_some_and(|report| report.root_unavailable)
        {
            return Some(Text::CacheRestart);
        }
        Some(match runtime {
            Some(engine::RuntimeError::Cancelled) => Text::CacheCancelled,
            Some(engine::RuntimeError::CacheCorrupt) => Text::CacheCorrupt,
            Some(engine::RuntimeError::CacheBusy) => Text::CacheBusy,
            Some(engine::RuntimeError::Edit(engine::EditError::Unsupported(_))) => {
                Text::CacheUnavailable
            }
            _ if matches!(
                cause,
                Some(engine::control::Error::Edit(
                    engine::EditError::Unsupported(_)
                ))
            ) =>
            {
                Text::CacheUnavailable
            }
            _ => Text::CacheFailed,
        })
    }
}
/// Read-only composition inputs shared by editor areas.
pub(crate) struct Context<'a> {
    pub(crate) playback: &'a Playback,
    pub(crate) keybindings: &'a actions::KeybindingPreferences,
    pub(crate) session: &'a engine::Session,
    /// Where the open project is saved; `None` until its first save.
    pub(crate) project_path: Option<&'a std::path::Path>,
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
    /// What the header says about persistence.
    pub(crate) save_status: SaveStatus,
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
    /// While a Media-panel asset is being dragged: the assets and the track kind they need.
    pub(crate) media_drag: Option<MediaDrag>,
    /// Armed chroma-key eyedropper; the player samples its next click.
    pub(crate) key_color_pick: Option<crate::editor::KeyPick>,
    /// Armed matte refinement brush; the player turns its drags into strokes.
    pub(crate) matte_brush: Option<crate::editor::MatteBrush>,
    /// The colour popover's eyedropper is armed; the player samples its next click.
    pub(crate) colour_sampling: bool,
    /// Keyword emphasis of the selected caption item's cue, when one item is selected.
    pub(crate) keywords: Option<KeywordUi>,
}
/// Keyword emphasis as the caption item's Text tab shows it. Plain values only: the Captions
/// panel owns the cue selection and the suggestion run, and the frame routes `KeywordAction`s
/// back to it.
#[derive(Clone)]
pub(crate) struct KeywordUi {
    /// The caption item this section acts on.
    pub(crate) item: project::ItemId,
    /// The one cue selected in the Captions panel, if exactly one is.
    pub(crate) cue: Option<captions::NfcText>,
    /// Cues selected in the Captions panel.
    pub(crate) selected: usize,
    /// Every selected cue carries verified word timing.
    pub(crate) timed: bool,
    /// Timed words of the selected cue: word id and text.
    pub(crate) words: Vec<(u32, String)>,
    /// Word ids the cue emphasises now, after validation.
    pub(crate) shown: Vec<u32>,
    /// Most words the cue may emphasise.
    pub(crate) limit: usize,
    /// A stored selection no longer validates against the cue's source text.
    pub(crate) stale: bool,
    /// Word highlight is on, so the player draws the keywords.
    pub(crate) highlight: bool,
    /// Suggestions waiting to be applied.
    pub(crate) suggested: usize,
    pub(crate) run: KeywordRun,
    pub(crate) problem: Option<Text>,
    /// Other subtitle work is running, so the section's edits wait.
    pub(crate) busy: bool,
}
/// Where a keyword suggestion run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum KeywordRun {
    #[default]
    Idle,
    AwaitingApproval,
    Working,
}
/// Keyword emphasis intentions. The frame routes them to the Captions panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeywordAction {
    Choose,
    Clear,
    Preview,
    /// The complete new word selection for the selected cue.
    Pick(Vec<u32>),
    Suggest,
    Apply,
    Cancel,
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

/// Read-only view a workflow step reads the open session through.
pub(in crate::editor) struct View<'a> {
    pub(in crate::editor) budget: &'a engine::ByteBudget,
    pub(in crate::editor) session: &'a engine::Session,
    pub(in crate::editor) preferences: &'a crate::preferences::Preferences,
    pub(in crate::editor) playhead: time::Tick,
    /// Whether no frame-owned surface blocks an edit the area would start.
    pub(in crate::editor) edit_ready: bool,
    /// The selected timeline transition, if any.
    pub(in crate::editor) transition: Option<project::TransitionId>,
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
    /// Run one registered action through the shared activation path.
    Action(String),
    /// Attach one effect to a clip through the properties workflow.
    AddEffect {
        item: project::ItemId,
        kind: String,
    },
    /// Ask for the folder that receives the selected clips' export.
    ExportClips,
    /// Insert one shape through its `graphic.insert.*` action when it is enabled, then
    /// tell the assets area to record it as recent.
    InsertShape(String),
    /// Switch the open scene the way the scene sheet's row does, without its select mode.
    SwitchScene(project::SceneId),
    /// Arm or leave the chroma-key eyedropper the player samples next.
    ArmKeyColor(Option<crate::editor::KeyPick>),
    /// Arm or leave the matte refinement brush the player paints with.
    ArmMatteBrush(Option<crate::editor::MatteBrush>),
    /// Ask the window to open the native media picker for one clip's replacement.
    ReplaceMedia(project::ItemId),
    /// Edits changed the project: drop the nested selection scopes it left dangling.
    PruneSelection,
    /// Hand one lifecycle step (a save, a picker) to the window that owns it.
    Lifecycle(crate::services::lifecycle::EditorAction),
}
