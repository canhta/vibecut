//! One app action registry, canonical chords and global key dispatch.
//!
//! The registry is the single source for key, menu and button trigger ids.
//! Engine command ids and schemas remain owned by `engine::registry`; app-only
//! actions name their UI owner here. Bindings persist in the one versioned
//! preference store (`preferences::Preferences`), never in a second file.

use std::collections::{BTreeMap, BTreeSet};

use crate::i18n::{Locale, Text};
use project::Scene;
use time::Tick;

pub use crate::input::{
    BlockedReason, Chord, ChordError, Dispatch, DispatchContext, DispatchInfo, KEYBINDING_FORMAT,
    KEYBINDING_VERSION, KEYBINDINGS_RESET_COMMAND, KEYBINDINGS_SET_COMMAND, KeybindingError,
    KeybindingIssue, KeybindingPreferences, KeybindingProfile, Keymap, MAX_ACTIONS,
    MAX_BINDINGS_PER_ACTION, MAX_CHORD_TEXT, MAX_PROFILE_BYTES, Mods, Platform,
};

/// The eight groups the shortcuts dialog and keyboard guide use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionCategory {
    File,
    Edit,
    Playback,
    Navigation,
    Selection,
    History,
    Timeline,
    View,
}

impl ActionCategory {
    pub const ALL: &'static [Self] = &[
        Self::File,
        Self::Edit,
        Self::Playback,
        Self::Navigation,
        Self::Selection,
        Self::History,
        Self::Timeline,
        Self::View,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::Playback => "Playback",
            Self::Navigation => "Navigation",
            Self::Selection => "Selection",
            Self::History => "History",
            Self::Timeline => "Timeline",
            Self::View => "View",
        }
    }
}

/// Which way `playback.jump_bookmark` searches from the playhead; it never wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookmarkDirection {
    Next,
    Previous,
}

/// The one bookmark lookup behind enablement, dispatch and both menus.
/// Bookmarks sort by raw (start, id); each candidate targets its start clamped
/// to the scene duration, exactly as Seek clamps. Comparing those effective
/// times skips a marker already at the playhead, including markers past the
/// end that all land on the duration.
pub(crate) fn bookmark_target(
    scene: &Scene,
    at: Tick,
    direction: BookmarkDirection,
) -> Option<Tick> {
    bookmark_target_in(&scene.bookmarks, scene.duration(), at, direction)
}

pub(crate) fn bookmark_target_in(
    bookmarks: &[project::Bookmark],
    duration: Tick,
    at: Tick,
    direction: BookmarkDirection,
) -> Option<Tick> {
    let candidates = bookmarks.iter().map(|bookmark| {
        (
            bookmark.start,
            bookmark.id,
            bookmark.start.clamp(Tick::ZERO, duration),
        )
    });
    match direction {
        BookmarkDirection::Next => candidates
            .filter(|(_, _, target)| *target > at)
            .min_by_key(|(start, id, _)| (*start, *id))
            .map(|(_, _, target)| target),
        BookmarkDirection::Previous => candidates
            .filter(|(_, _, target)| *target < at)
            .max_by_key(|(start, id, _)| (*start, *id))
            .map(|(_, _, target)| target),
    }
}

/// Available generated shapes; unavailable catalog variants have no action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicShape {
    Rectangle,
    Ellipse,
    Polygon,
    Star,
}

/// Executable app workflows. Engine mutations inside a workflow still use the
/// engine owner; opening a dialog never advertises its later mutation as a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    InsertGraphic(GraphicShape),
    MediaView,
    MediaSort,
    MediaSortBy(crate::editor::MediaSortKey),
    RequestDeleteMedia,
    CancelDeleteMedia,
    ConfirmDeleteMedia,
    RevealMedia,
    InsertText,
    Copy,
    /// Copy the one selected item as the source of Paste attributes.
    CopyAttributes,
    Cut,
    Paste,
    Subtitles,
    SubtitleCue,
    Import,
    Export,
    Crop,
    Save,
    SaveAs,
    FocusHome,
    ExitProject,
    RenameProject,
    DeleteProject,
    CancelDeleteProject,
    ConfirmDeleteProject,
    CancelImport,
    Play,
    Pause,
    BackSecond,
    ForwardSecond,
    FrameBack,
    FrameForward,
    Start,
    End,
    SelectAll,
    DeselectAll,
    Escape,
    Undo,
    Redo,
    Snap,
    Ripple,
    Loop,
    PreviewAxis,
    /// Timeline pointer mode: Select clicks and drags clips, Split cuts them.
    SelectTool,
    SplitTool,
    /// Select every item that starts before / ends after the playhead.
    SelectLeftward,
    SelectRightward,
    ZoomIn,
    ZoomOut,
    OpenScenes,
    CloseScenes,
    SelectScenes,
    RequestDeleteScenes,
    CancelDeleteScenes,
    Theme,
    Locale,
    ResetLayout,
    ProjectInspector,
    DetailsInspector,
    RetryPreview,
    OpenModels,
    OpenVoiceover,
    OpenSceneSplit,
    OpenPreferences,
    OpenShortcuts,
    RunExport,
    RetryExport,
    ExportMarkIn,
    ExportMarkOut,
    ExportInOut,
    RenameScene,
    SceneRow,
    SelectItem,
    PreviewSnapshot,
    PreviewCopySnapshot,
    PreviewFullscreen,
    PreviewFit,
    PreviewZoomIn,
    PreviewZoomOut,
    PreviewZoomPreset,
    /// `preview.rulers`: show or hide the player's top and left rulers.
    PreviewRulers,
    /// `preview.guides.visible`: show or hide the guide lines.
    GuidesVisible,
    /// `preview.guides.lock`: lock or unlock the guide lines against dragging.
    GuidesLock,
    /// `preview.guides.clear`: remove every guide line.
    GuidesClear,
    /// `preview.guides.add_horizontal` and `preview.guides.add_vertical`.
    GuideAdd(crate::editor::LineAxis),
    /// `preview.guides.move`: move one guide line to a canvas position.
    GuideMove,
    Seek,
    JumpBookmark(BookmarkDirection),
    SetPreviewZoom,
    SetPreference,
    /// `catalog.favorite.set`: star or unstar one catalogue id.
    SetFavorite,
    /// `catalog.favorite.list`: read the starred ids of one catalogue family.
    ListFavorites,
    SetKeybinding,
    ResetKeybindings,
    ImportKeybindings,
    ExportKeybindings,
    OpenHelp,
    OpenFeedback,
}

/// Engine routing owns the executable identity. Delete follows the app's ripple
/// preference; every other route has one engine-owned command kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineAction {
    InsertMedia,
    Split,
    /// Hold the selected video item's frame at the playhead as a still.
    FreezeFrame,
    SplitLeft,
    SplitRight,
    SplitAll,
    Duplicate,
    Delete,
    RippleDelete,
    SetHidden,
    SetMuted,
    ExtractAudio,
    RecoverAudio,
    AddTrack,
    ToggleBookmark,
    CreateScene,
    DeleteScenes,
    /// Give the selected items the attribute groups of the copied item.
    PasteAttributes,
    /// Toggle one reflection axis over the current item selection.
    Flip(project::FlipAxis),
    /// Move the current item selection to one canvas edge or centre.
    Align(project::ItemAlignment),
    /// Space the current item selection evenly along one canvas axis.
    Distribute(project::DistributionAxis),
}
impl EngineAction {
    pub const fn command(self, ripple: bool) -> engine::CommandKind {
        use engine::CommandKind as K;
        match self {
            Self::InsertMedia => K::InsertItem,
            Self::Split => K::SplitAt,
            Self::FreezeFrame => K::FreezeFrame,
            Self::SplitLeft => K::TimelineSplitLeft,
            Self::SplitRight => K::TimelineSplitRight,
            Self::SplitAll => K::TimelineSplitAll,
            Self::Duplicate => K::DuplicateItems,
            Self::Delete if ripple => K::RippleDelete,
            Self::Delete => K::Delete,
            Self::RippleDelete => K::RippleDelete,
            Self::SetHidden => K::SetItemsHidden,
            Self::SetMuted => K::SetItemsMuted,
            Self::ExtractAudio => K::ExtractAudio,
            Self::RecoverAudio => K::RecoverAudio,
            Self::AddTrack => K::AddTrack,
            Self::ToggleBookmark => K::ToggleBookmark,
            Self::CreateScene => K::CreateScene,
            Self::DeleteScenes => K::DeleteScenes,
            Self::PasteAttributes => K::PasteAttributes,
            Self::Flip(_) => K::SetItemsFlipped,
            Self::Align(_) => K::AlignItems,
            Self::Distribute(_) => K::DistributeItems,
        }
    }
}

/// The executable route, shared by keys, menus, buttons and direct activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionTarget {
    Engine(EngineAction),
    Ui {
        owner: &'static str,
        action: UiAction,
    },
}

/// One app action. Defaults are canonical chord text.
#[derive(Clone, Copy, Debug)]
pub struct ActionSpec {
    pub id: &'static str,
    pub label: Text,
    pub category: ActionCategory,
    pub target: ActionTarget,
    pub defaults: &'static [&'static str],
    /// The action is intercepted only when the in-app clipboard has an entry.
    pub requires_clipboard: bool,
}

impl ActionSpec {
    const fn engine(
        id: &'static str,
        label: Text,
        category: ActionCategory,
        command: EngineAction,
        defaults: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            label,
            category,
            target: ActionTarget::Engine(command),
            defaults,
            requires_clipboard: false,
        }
    }

    const fn ui(
        id: &'static str,
        label: Text,
        category: ActionCategory,
        owner: &'static str,
        action: UiAction,
        defaults: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            label,
            category,
            target: ActionTarget::Ui { owner, action },
            defaults,
            requires_clipboard: matches!(action, UiAction::Paste),
        }
    }

    /// Parameterized widget families need an item/scene identity, so a global
    /// chord cannot invoke them. Shortcut editors list only bindable actions.
    pub const fn bindable(&self) -> bool {
        !matches!(
            self.target,
            ActionTarget::Ui {
                action: UiAction::RenameScene
                    | UiAction::SceneRow
                    | UiAction::SelectItem
                    | UiAction::SubtitleCue
                    | UiAction::RevealMedia
                    | UiAction::PreviewZoomPreset
                    | UiAction::GuideMove
                    | UiAction::Seek
                    | UiAction::SetPreviewZoom
                    | UiAction::SetPreference
                    | UiAction::SetFavorite
                    | UiAction::ListFavorites
                    | UiAction::SetKeybinding
                    | UiAction::ResetKeybindings
                    | UiAction::ImportKeybindings
                    | UiAction::ExportKeybindings,
                ..
            }
        )
    }
}

/// Every key, menu and button action in the desktop app.
mod editor;
mod menu;
mod shell;

pub use menu::{MENUS, Menu, MenuItem};

use std::sync::OnceLock;

/// Every key, menu and button action, assembled from each owner's table.
static REGISTRY: OnceLock<Vec<ActionSpec>> = OnceLock::new();

/// The one registry, in table order. Per-owner tables stay independent and are
/// concatenated once on first use.
pub fn actions() -> &'static [ActionSpec] {
    REGISTRY.get_or_init(|| {
        let mut all = Vec::with_capacity(editor::ACTIONS.len() + shell::ACTIONS.len());
        all.extend_from_slice(editor::ACTIONS);
        all.extend_from_slice(shell::ACTIONS);
        all
    })
}

pub fn action(id: &str) -> Option<&'static ActionSpec> {
    actions().iter().find(|spec| spec.id == id)
}

/// Resolve parameterized widgets through the same registry as ordinary actions.
pub(crate) fn resolve(id: &str) -> Option<(&'static ActionSpec, Option<u64>)> {
    if let Some(spec) = action(id) {
        return spec.bindable().then_some((spec, None));
    }
    if id
        .strip_prefix("subtitles.cue.")
        .is_some_and(|cue| !cue.is_empty())
    {
        return Some((action("subtitles.cue")?, None));
    }
    let (family, argument) = id.rsplit_once('.')?;
    let spec = action(family)?;
    if spec.bindable() {
        return None;
    }
    Some((spec, Some(argument.parse().ok()?)))
}

/// Markdown table generated from [`ACTIONS`]; the keyboard guide embeds it
/// between markers and a drift test compares the two.
pub fn keyboard_guide_table() -> String {
    let mut text = String::from(
        "| Action | Category | Default keys | Engine command | UI owner |\n|---|---|---|---|---|\n",
    );
    for spec in actions() {
        let defaults = if spec.defaults.is_empty() {
            "—".to_owned()
        } else {
            spec.defaults.join(", ")
        };
        let (engine, owner) = match spec.target {
            ActionTarget::Engine(route) => {
                let normal = route.command(false).id();
                let ripple = route.command(true).id();
                let commands = if normal == ripple {
                    format!("`{normal}`")
                } else {
                    format!("`{normal}` / `{ripple}` (ripple enabled)")
                };
                (commands, "—".to_owned())
            }
            ActionTarget::Ui { owner, .. } => ("—".to_owned(), owner.to_owned()),
        };
        text.push_str(&format!(
            "| `{}` — {} | {} | {} | {} | {} |\n",
            spec.id,
            spec.label.text(Locale::En),
            spec.category.label(),
            defaults,
            engine,
            owner
        ));
    }
    text
}

impl KeybindingPreferences {
    /// Custom chords replace the defaults entirely; an empty list unbinds.
    pub fn effective_chords(&self, spec: &ActionSpec) -> Vec<String> {
        if let Some(chords) = self.bindings.get(spec.id) {
            return chords.clone();
        }
        spec.defaults
            .iter()
            .map(|text| (*text).to_owned())
            .collect()
    }

    /// Set one action's bindings. Equal-to-default removes the custom marker.
    pub fn set_action(&mut self, action: &str, chords: Vec<String>) -> Result<(), KeybindingError> {
        let Some(spec) = self::action(action).filter(|spec| spec.bindable()) else {
            return Err(KeybindingError::UnknownAction(action.to_owned()));
        };
        if chords.len() > MAX_BINDINGS_PER_ACTION {
            return Err(KeybindingError::TooManyBindings(action.to_owned()));
        }
        let mut normalized = Vec::with_capacity(chords.len());
        for text in chords {
            let chord = Chord::parse(&text).map_err(|error| KeybindingError::InvalidChord {
                text: text.clone(),
                error,
            })?;
            let canonical = chord.canonical();
            if normalized.contains(&canonical) {
                return Err(KeybindingError::DuplicateBinding(canonical));
            }
            normalized.push(canonical);
        }
        self.check_conflicts(action, &normalized)?;
        let defaults: Vec<String> = spec
            .defaults
            .iter()
            .map(|text| (*text).to_owned())
            .collect();
        if normalized == defaults {
            self.bindings.remove(action);
            self.is_customized.remove(action);
        } else {
            self.bindings.insert(action.to_owned(), normalized);
            self.is_customized.insert(action.to_owned());
        }
        Ok(())
    }

    /// Restore one action to its registry defaults.
    pub fn reset_action(&mut self, action: &str) -> Result<bool, KeybindingError> {
        let Some(spec) = self::action(action).filter(|spec| spec.bindable()) else {
            return Err(KeybindingError::UnknownAction(action.to_owned()));
        };
        let defaults = spec
            .defaults
            .iter()
            .map(|text| (*text).to_owned())
            .collect::<Vec<_>>();
        self.check_conflicts(action, &defaults)?;
        let removed = self.bindings.remove(action).is_some();
        let marker = self.is_customized.remove(action);
        Ok(removed || marker)
    }

    fn check_conflicts(&self, action: &str, chords: &[String]) -> Result<(), KeybindingError> {
        let mut used = BTreeMap::new();
        for spec in actions() {
            if !spec.bindable() || spec.id == action {
                continue;
            }
            for text in self.effective_chords(spec) {
                if let Ok(chord) = Chord::parse(&text) {
                    used.insert(chord.canonical(), spec.id.to_owned());
                }
            }
        }
        for text in chords {
            let Ok(chord) = Chord::parse(text) else {
                continue;
            };
            let canonical = chord.canonical();
            if let Some(existing) = used.get(&canonical) {
                return Err(KeybindingError::Conflict {
                    chord: canonical,
                    existing: existing.clone(),
                    action: action.to_owned(),
                });
            }
        }
        Ok(())
    }
}

impl Keymap {
    /// Resolve the persisted map against the one action registry.
    pub fn resolve(preferences: &KeybindingPreferences) -> Self {
        let mut bindings: BTreeMap<Chord, &'static str> = BTreeMap::new();
        let mut specs: BTreeMap<&'static str, DispatchInfo> = BTreeMap::new();
        let mut issues = Vec::new();
        let mut known = BTreeSet::new();
        for spec in actions() {
            if !spec.bindable() {
                continue;
            }
            known.insert(spec.id);
            specs.insert(
                spec.id,
                DispatchInfo {
                    bindable: true,
                    requires_clipboard: spec.requires_clipboard,
                },
            );
            let mut seen = BTreeSet::new();
            for text in preferences.effective_chords(spec) {
                let Ok(chord) = Chord::parse(&text) else {
                    issues.push(KeybindingIssue::InvalidChord {
                        action: spec.id.to_owned(),
                        text,
                    });
                    continue;
                };
                if !seen.insert(chord) {
                    issues.push(KeybindingIssue::Duplicate {
                        action: spec.id.to_owned(),
                        chord: chord.canonical(),
                    });
                    continue;
                }
                if let Some(existing) = bindings.get(&chord).copied() {
                    issues.push(KeybindingIssue::Conflict {
                        chord: chord.canonical(),
                        first: existing.to_owned(),
                        second: spec.id.to_owned(),
                    });
                    continue;
                }
                bindings.insert(chord, spec.id);
            }
        }
        for action in preferences.bindings.keys() {
            if !known.contains(action.as_str())
                && !issues.iter().any(
                    |issue| matches!(issue, KeybindingIssue::UnknownAction(id) if id == action),
                )
            {
                issues.push(KeybindingIssue::UnknownAction(action.clone()));
            }
        }
        for action in &preferences.is_customized {
            if !known.contains(action.as_str())
                && !issues.iter().any(
                    |issue| matches!(issue, KeybindingIssue::UnknownAction(id) if id == action),
                )
            {
                issues.push(KeybindingIssue::UnknownAction(action.clone()));
            }
        }
        Keymap::new(bindings, specs, issues)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::egui;
    use crate::input::valid_action_id;
    use crate::preferences::{PreferenceIssue, PreferenceStore, Preferences};

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn chords_normalise_modifier_order_and_display_per_platform() {
        let chord = Chord::parse("shift+alt+ctrl+z").unwrap();
        assert_eq!(chord.canonical(), "Ctrl+Alt+Shift+Z");
        assert_eq!(chord.display(Platform::Mac), "⌥⇧⌘Z");
        assert_eq!(chord.display(Platform::Windows), "Ctrl+Alt+Shift+Z");
        assert_eq!(
            Chord::parse("⌥⇧⌘Z").unwrap().canonical(),
            "Ctrl+Alt+Shift+Z"
        );
        assert_eq!(
            Chord::parse("command+option+shift+k").unwrap().canonical(),
            "Ctrl+Alt+Shift+K"
        );
        assert_eq!(Chord::parse("esc").unwrap().display(Platform::Mac), "⎋");
        assert_eq!(
            Chord::parse("return").unwrap().display(Platform::Windows),
            "Enter"
        );
        let right = Chord::parse("shift+ctrl+right").unwrap();
        assert_eq!(right.parts(Platform::Mac), ["⌘", "⇧", "→"]);
        assert_eq!(right.parts(Platform::Windows), ["Ctrl", "Shift", "→"]);
        assert_eq!(
            Chord::parse("space").unwrap().parts(Platform::Mac),
            ["Space"]
        );
        let bracket = Chord::parse("ctrl+[").unwrap();
        assert_eq!(bracket.canonical(), "Ctrl+OpenBracket");
        assert_eq!(bracket.parts(Platform::Mac), ["⌘", "["]);
        assert_eq!(bracket.display(Platform::Mac), "⌘[");
        assert_eq!(bracket.display(Platform::Windows), "Ctrl+[");
    }

    #[test]
    fn invalid_chords_are_refused_with_typed_errors() {
        assert_eq!(Chord::parse(""), Err(ChordError::Empty));
        assert_eq!(Chord::parse("ctrl+"), Err(ChordError::MissingKey));
        assert_eq!(Chord::parse("ctrl"), Err(ChordError::MissingKey));
        assert!(matches!(
            Chord::parse("ctrl+banana"),
            Err(ChordError::UnknownKey(key)) if key == "banana"
        ));
        assert!(matches!(
            Chord::parse("ctrl+ctrl+k"),
            Err(ChordError::DuplicateModifier(_))
        ));
        assert!(matches!(
            Chord::parse(&"a".repeat(MAX_CHORD_TEXT + 1)),
            Err(ChordError::TooLong)
        ));
    }

    #[test]
    fn default_bindings_are_unique_and_complete() {
        assert_eq!(ActionCategory::ALL.len(), 8);
        for category in ActionCategory::ALL {
            assert!(
                actions().iter().any(|spec| spec.category == *category),
                "{} has no actions",
                category.label()
            );
        }
        let mut ids = BTreeSet::new();
        let mut chords = BTreeMap::new();
        for spec in actions() {
            assert!(ids.insert(spec.id), "duplicate action id {}", spec.id);
            assert!(valid_action_id(spec.id), "bad action id {}", spec.id);
            for text in spec.defaults {
                let chord = Chord::parse(text).unwrap();
                if let Some(existing) = chords.insert(chord, spec.id) {
                    panic!(
                        "{} and {} both default to {}",
                        existing,
                        spec.id,
                        chord.canonical()
                    );
                }
            }
        }
    }

    #[test]
    fn conflicts_name_both_actions_and_leave_the_map_untouched() {
        let mut preferences = KeybindingPreferences::default();
        preferences
            .set_action("timeline.delete", vec!["Ctrl+K".into()])
            .unwrap();
        let before = preferences.clone();
        let error = preferences
            .set_action("timeline.snap", vec!["Ctrl+K".into()])
            .unwrap_err();
        assert_eq!(
            error,
            KeybindingError::Conflict {
                chord: "Ctrl+K".into(),
                existing: "timeline.delete".into(),
                action: "timeline.snap".into(),
            }
        );
        assert_eq!(preferences, before);
    }

    #[test]
    fn resolver_reports_conflicts_and_unknown_actions_without_panicking() {
        let mut preferences = KeybindingPreferences::default();
        preferences
            .bindings
            .insert("future.action".into(), vec!["Ctrl+K".into()]);
        preferences
            .bindings
            .insert("timeline.delete".into(), vec!["Ctrl+K".into()]);
        preferences
            .bindings
            .insert("timeline.snap".into(), vec!["Ctrl+K".into()]);
        let keymap = Keymap::resolve(&preferences);
        assert!(keymap.issues().iter().any(
            |issue| matches!(issue, KeybindingIssue::UnknownAction(action) if action == "future.action")
        ));
        assert!(
            keymap
                .issues()
                .iter()
                .any(|issue| matches!(issue, KeybindingIssue::Conflict { .. }))
        );
    }

    #[test]
    fn keybinding_profiles_refuse_bad_input_and_merge_or_replace() {
        let current = KeybindingPreferences {
            bindings: BTreeMap::from([
                ("timeline.snap".into(), vec!["Ctrl+Shift+N".into()]),
                ("playback.seek".into(), vec!["F7".into()]),
            ]),
            is_customized: BTreeSet::from(["timeline.snap".into(), "playback.seek".into()]),
        };
        let known = |id: &str| crate::actions::action(id).is_some();
        let profile = format!(
            "{{\"format\":\"{KEYBINDING_FORMAT}\",\"version\":1,\"bindings\":{{\"timeline.snap\":[\"Ctrl+Shift+K\"],\"future.action\":[\"F9\"]}},\"isCustomized\":[\"timeline.snap\"]}}"
        );
        let imported = KeybindingProfile::import(profile.as_bytes()).unwrap();
        let merged = current
            .apply_profile(imported.clone(), crate::input::ImportMode::Merge, known)
            .unwrap();
        assert_eq!(merged.unknown_actions, ["future.action"]);
        assert_eq!(
            merged.preferences.bindings["timeline.snap"],
            ["Ctrl+Shift+K"]
        );
        assert_eq!(merged.preferences.bindings["playback.seek"], ["F7"]);
        assert!(!merged.preferences.bindings.contains_key("future.action"));
        let replaced = current
            .apply_profile(imported, crate::input::ImportMode::Replace, known)
            .unwrap();
        assert_eq!(replaced.preferences.bindings.len(), 1);
        assert_eq!(
            replaced.preferences.is_customized,
            BTreeSet::from(["timeline.snap".into()])
        );
        assert_eq!(replaced.unknown_actions, ["future.action"]);
        // A corrupt file never reaches apply_profile, so the current bindings stay as they were.
        assert_eq!(
            KeybindingProfile::import(b"{\"format\":"),
            Err(KeybindingError::Malformed)
        );
        assert_eq!(current.bindings.len(), 2);
    }

    #[test]
    fn unsupported_profile_versions_are_refused() {
        let newer = format!(
            "{{\"format\":\"{KEYBINDING_FORMAT}\",\"version\":2,\"bindings\":{{}},\"isCustomized\":[]}}"
        );
        assert_eq!(
            KeybindingProfile::import(newer.as_bytes()),
            Err(KeybindingError::UnsupportedVersion {
                found: 2,
                expected: KEYBINDING_VERSION,
            })
        );
        let wrong = String::from(
            "{\"format\":\"other\",\"version\":1,\"bindings\":{},\"isCustomized\":[]}",
        );
        assert_eq!(
            KeybindingProfile::import(wrong.as_bytes()),
            Err(KeybindingError::WrongFormat("other".into()))
        );
    }

    #[test]
    fn damaged_keybindings_are_sanitized_on_startup_without_hiding_unknown_actions() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut document = serde_json::to_value(Preferences::default()).unwrap();
        document["keybindings"] = serde_json::json!({
            "bindings": {
                "timeline.snap": ["Ctrl+Shift+N"],
                "bad action": []
            },
            "isCustomized": ["timeline.snap", "missing.action"]
        });
        std::fs::write(store.path(), document.to_string()).unwrap();
        let (preferences, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Keybindings));
        assert_eq!(
            preferences.keybindings.bindings["timeline.snap"],
            ["Ctrl+Shift+N"]
        );
        assert!(!preferences.keybindings.bindings.contains_key("bad action"));
        assert!(
            !preferences
                .keybindings
                .is_customized
                .contains("missing.action")
        );
    }

    #[test]
    fn hostile_binding_json_is_bounded() {
        assert_eq!(
            KeybindingProfile::import(&vec![b' '; MAX_PROFILE_BYTES + 1]),
            Err(KeybindingError::TooLarge)
        );
        assert_eq!(
            KeybindingProfile::import(
                br#"{"format":"vibecut-keybindings","version":1,"extra":true}"#
            ),
            Err(KeybindingError::Malformed)
        );
        let too_many = (0..=MAX_BINDINGS_PER_ACTION)
            .map(|index| format!("F{index}"))
            .collect::<Vec<_>>();
        let mut preferences = KeybindingPreferences::default();
        preferences
            .bindings
            .insert("timeline.snap".into(), too_many);
        assert!(matches!(
            preferences.validate(),
            Err(KeybindingError::TooManyBindings(_))
        ));
        let mut preferences = KeybindingPreferences::default();
        for index in 0..=MAX_ACTIONS {
            preferences
                .bindings
                .insert(format!("future.{index}"), Vec::new());
        }
        assert_eq!(preferences.validate(), Err(KeybindingError::TooManyActions));
        let mut preferences = KeybindingPreferences::default();
        preferences
            .bindings
            .insert("timeline.snap".into(), vec!["Nope+Key".into()]);
        assert!(matches!(
            preferences.validate(),
            Err(KeybindingError::InvalidChord { .. })
        ));
    }

    #[test]
    fn binding_preferences_round_trip_with_customized_flags() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut preferences = Preferences::default();
        preferences
            .keybindings
            .set_action("timeline.snap", vec!["Ctrl+Shift+N".into()])
            .unwrap();
        preferences
            .keybindings
            .set_action("timeline.ripple", vec!["Ctrl+R".into()])
            .unwrap();
        preferences
            .keybindings
            .set_action("timeline.split", Vec::new())
            .unwrap();
        store.save(&preferences).unwrap();
        let loaded = store.load_with_warning().0;
        assert_eq!(loaded.keybindings, preferences.keybindings);
        assert!(loaded.keybindings.is_customized.contains("timeline.snap"));
        assert!(loaded.keybindings.is_customized.contains("timeline.ripple"));
        assert!(loaded.keybindings.is_customized.contains("timeline.split"));
        assert_eq!(
            loaded
                .keybindings
                .effective_chords(action("timeline.split").unwrap()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn restoring_defaults_clears_customized_flag() {
        let mut preferences = KeybindingPreferences::default();
        preferences
            .set_action("timeline.snap", vec!["Ctrl+Shift+N".into()])
            .unwrap();
        assert!(preferences.is_customized.contains("timeline.snap"));
        let defaults = action("timeline.snap")
            .unwrap()
            .defaults
            .iter()
            .map(|text| (*text).to_owned())
            .collect();
        preferences.set_action("timeline.snap", defaults).unwrap();
        assert!(!preferences.is_customized.contains("timeline.snap"));
        assert!(!preferences.bindings.contains_key("timeline.snap"));

        preferences.set_action("timeline.snap", Vec::new()).unwrap();
        preferences
            .set_action("timeline.ripple", vec!["N".into()])
            .unwrap();
        let before = preferences.clone();
        assert!(matches!(
            preferences.reset_action("timeline.snap"),
            Err(KeybindingError::Conflict { .. })
        ));
        assert_eq!(preferences, before);
    }

    #[test]
    fn blocked_contexts_do_not_consume_or_dispatch() {
        let preferences = KeybindingPreferences::default();
        let keymap = Keymap::resolve(&preferences);
        let mut input = egui::InputState::default();
        input
            .events
            .push(key_event(egui::Key::N, egui::Modifiers::NONE));
        for context in [
            DispatchContext {
                overlay_open: true,
                ..Default::default()
            },
            DispatchContext {
                project_loading: true,
                ..Default::default()
            },
            DispatchContext {
                recording: true,
                ..Default::default()
            },
            DispatchContext {
                text_input: true,
                ..Default::default()
            },
        ] {
            assert!(matches!(
                keymap.take(&mut input, context),
                Dispatch::Blocked(_)
            ));
            assert_eq!(input.events.len(), 1);
        }
    }

    #[test]
    fn paste_falls_through_without_an_in_app_entry() {
        let preferences = KeybindingPreferences::default();
        let keymap = Keymap::resolve(&preferences);
        let mut input = egui::InputState::default();
        input.events.push(key_event(
            egui::Key::V,
            egui::Modifiers {
                command: true,
                ..Default::default()
            },
        ));
        assert_eq!(
            keymap.take(&mut input, DispatchContext::default()),
            Dispatch::FellThrough("edit.paste")
        );
        assert_eq!(
            input.events.len(),
            1,
            "media paste must still see the event"
        );
        assert_eq!(
            keymap.take(
                &mut input,
                DispatchContext {
                    clipboard_entry: true,
                    ..Default::default()
                }
            ),
            Dispatch::Action {
                action: "edit.paste",
                chord: Chord::parse("Ctrl+V").unwrap(),
            }
        );
        assert!(input.events.is_empty());
    }

    #[test]
    fn one_exact_press_dispatches_once_even_with_duplicate_events() {
        let keymap = Keymap::resolve(&KeybindingPreferences::default());
        let mut input = egui::InputState::default();
        input
            .events
            .push(key_event(egui::Key::N, egui::Modifiers::NONE));
        input
            .events
            .push(key_event(egui::Key::N, egui::Modifiers::NONE));
        let first = keymap.take(&mut input, DispatchContext::default());
        assert!(matches!(
            first,
            Dispatch::Action {
                action: "timeline.snap",
                ..
            }
        ));
        assert!(input.events.is_empty());
        assert_eq!(
            keymap.take(&mut input, DispatchContext::default()),
            Dispatch::NoMatch
        );
    }

    #[test]
    fn extra_shift_does_not_fire_a_less_specific_binding() {
        let keymap = Keymap::resolve(&KeybindingPreferences::default());
        let mut input = egui::InputState::default();
        input.events.push(key_event(
            egui::Key::Z,
            egui::Modifiers {
                command: true,
                shift: true,
                ..Default::default()
            },
        ));
        assert!(matches!(
            keymap.take(&mut input, DispatchContext::default()),
            Dispatch::Action {
                action: "edit.redo",
                ..
            }
        ));
        let mut input = egui::InputState::default();
        input.events.push(key_event(
            egui::Key::Z,
            egui::Modifiers {
                command: true,
                alt: true,
                ..Default::default()
            },
        ));
        assert_eq!(
            keymap.take(&mut input, DispatchContext::default()),
            Dispatch::NoMatch
        );
        for (platform, modifiers, expected) in [
            (Platform::Mac, egui::Modifiers::CTRL, None),
            (Platform::Mac, egui::Modifiers::MAC_CMD, Some("edit.undo")),
            (
                Platform::Mac,
                egui::Modifiers::MAC_CMD.plus(egui::Modifiers::CTRL),
                None,
            ),
            (Platform::Windows, egui::Modifiers::CTRL, Some("edit.undo")),
        ] {
            let mut modifiers = modifiers;
            modifiers.command = modifiers.mac_cmd || modifiers.ctrl && !platform.is_mac();
            let mut input = egui::InputState::default();
            input.events.push(key_event(egui::Key::Z, modifiers));
            let outcome = keymap.take_on_platform(&mut input, DispatchContext::default(), platform);
            if let Some(expected) = expected {
                assert!(matches!(outcome, Dispatch::Action { action, .. } if action == expected));
                assert!(input.events.is_empty());
            } else {
                assert_eq!(outcome, Dispatch::NoMatch);
                assert_eq!(input.events.len(), 1);
            }
        }
    }

    #[test]
    fn menu_rows_are_registry_actions_whose_chords_need_command() {
        for menu in MENUS {
            for item in menu.items {
                let MenuItem::Action(id) = item else { continue };
                let spec = action(id).unwrap_or_else(|| panic!("{id} is not registered"));
                for chord in spec.defaults {
                    let chord = Chord::parse(chord).unwrap();
                    // A plain key as a menu equivalent would steal typing from text fields.
                    assert!(chord.mods.command, "{id} {chord}");
                }
            }
        }
    }

    #[test]
    fn keyboard_guide_matches_the_registry() {
        let guide = include_str!("../../../docs/product/agent-control/keyboard.md");
        let begin = "<!-- BEGIN GENERATED KEYMAP -->\n";
        let end = "<!-- END GENERATED KEYMAP -->";
        let generated = keyboard_guide_table();
        let table = guide
            .split_once(begin)
            .and_then(|(_, rest)| rest.split_once(end))
            .map(|(table, _)| table)
            .expect("keyboard guide markers");
        assert_eq!(table, generated);
    }
}
