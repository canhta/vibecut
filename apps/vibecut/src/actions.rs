//! One app action registry, canonical chords and global key dispatch.
//!
//! The registry is the single source for key, menu and button trigger ids.
//! Engine command ids and schemas remain owned by `engine::registry`; app-only
//! actions name their UI owner here. Bindings persist in the one versioned
//! preference store (`catalog::Preferences`), never in a second file.

use crate::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use thiserror::Error;

/// Longest single chord text accepted from storage or a recording UI.
pub const MAX_CHORD_TEXT: usize = 64;
/// Most bindings one action may have.
pub const MAX_BINDINGS_PER_ACTION: usize = 8;
/// Most binding entries a preference document may hold.
pub const MAX_ACTIONS: usize = 96;
/// Largest keybinding profile accepted by import.
pub const MAX_PROFILE_BYTES: usize = 16 * 1024;
/// Import/export profile format and version. Unsupported versions are refused.
pub const KEYBINDING_FORMAT: &str = "vibecut-keybindings";
pub const KEYBINDING_VERSION: u64 = 1;
/// App-local commands used by the shortcuts dialog (p1-parity/04).
pub const KEYBINDINGS_SET_COMMAND: &str = "ui.keybindings.set";
pub const KEYBINDINGS_RESET_COMMAND: &str = "ui.keybindings.reset";

/// The platform decides display glyphs. Mapping is compile-time in production
/// and explicit in tests, so macOS and Windows behaviour is checked on Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub const fn is_mac(self) -> bool {
        matches!(self, Self::Mac)
    }
}

/// The portable modifier set. `command` is Cmd on macOS and Ctrl elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mods {
    pub command: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key with modifiers. Chords compare and persist by canonical text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: egui::Key,
}

impl Chord {
    pub const fn new(mods: Mods, key: egui::Key) -> Self {
        Self { mods, key }
    }

    pub const fn plain(key: egui::Key) -> Self {
        Self {
            mods: Mods {
                command: false,
                alt: false,
                shift: false,
            },
            key,
        }
    }

    /// Reads canonical text and common aliases: modifier words in any order and
    /// case, macOS glyphs (`⌘`, `⌥`, `⇧`), glued or with `+`, and the key names
    /// [`egui::Key`] reads. `Ctrl++` is the Plus key.
    pub fn parse(text: &str) -> Result<Self, ChordError> {
        if text.len() > MAX_CHORD_TEXT {
            return Err(ChordError::TooLong);
        }
        let mut rest = text.trim();
        if rest.is_empty() {
            return Err(ChordError::Empty);
        }
        let mut mods = Mods::default();
        loop {
            if let Some((modifier, after)) = strip_glyph(rest) {
                add_modifier(&mut mods, modifier, rest)?;
                rest = match after.strip_prefix('+') {
                    Some(tail) if !tail.is_empty() => tail,
                    _ => after,
                };
                if rest.trim().is_empty() {
                    return Err(ChordError::MissingKey);
                }
                continue;
            }
            let first = rest.chars().next().map_or(0, char::len_utf8);
            let separator = rest
                .get(first..)
                .and_then(|tail| tail.find('+'))
                .map(|index| index + first);
            match separator {
                Some(index) => {
                    let token = rest.get(..index).unwrap_or_default().trim();
                    let after = rest.get(index + 1..).unwrap_or_default();
                    if after.trim().is_empty() {
                        return Err(ChordError::MissingKey);
                    }
                    let modifier = modifier_named(token)
                        .ok_or_else(|| ChordError::UnknownModifier(token.to_owned()))?;
                    add_modifier(&mut mods, modifier, token)?;
                    rest = after;
                }
                None => {
                    let token = rest.trim();
                    if modifier_named(token).is_some() {
                        return Err(ChordError::MissingKey);
                    }
                    let key =
                        parse_key(token).ok_or_else(|| ChordError::UnknownKey(token.to_owned()))?;
                    return Ok(Self { mods, key });
                }
            }
        }
    }

    /// Canonical text: `Ctrl+Alt+Shift+Key`.
    pub fn canonical(self) -> String {
        self.to_string()
    }

    /// Menus and the shortcuts dialog: `⌥⇧⌘K` on macOS, `Ctrl+Alt+Shift+K`
    /// elsewhere. The canonical chord never changes with the platform.
    pub fn display(self, platform: Platform) -> String {
        let mut text = String::new();
        if platform.is_mac() {
            if self.mods.alt {
                text.push('⌥');
            }
            if self.mods.shift {
                text.push('⇧');
            }
            if self.mods.command {
                text.push('⌘');
            }
            text.push_str(mac_key(self.key));
        } else {
            if self.mods.command {
                text.push_str("Ctrl+");
            }
            if self.mods.alt {
                text.push_str("Alt+");
            }
            if self.mods.shift {
                text.push_str("Shift+");
            }
            text.push_str(self.key.name());
        }
        text
    }

    /// Exact match, unlike egui's `consume_key` which ignores extra Shift/Alt.
    /// On macOS a physical Control key is never folded into Command.
    pub fn matches(self, platform: Platform, modifiers: egui::Modifiers) -> bool {
        if self.mods.command != modifiers.command
            || self.mods.alt != modifiers.alt
            || self.mods.shift != modifiers.shift
        {
            return false;
        }
        if platform.is_mac() && modifiers.ctrl {
            return false;
        }
        true
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.command {
            formatter.write_str("Ctrl+")?;
        }
        if self.mods.alt {
            formatter.write_str("Alt+")?;
        }
        if self.mods.shift {
            formatter.write_str("Shift+")?;
        }
        formatter.write_str(self.key.name())
    }
}

impl std::str::FromStr for Chord {
    type Err = ChordError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

#[derive(Clone, Copy)]
enum Modifier {
    Command,
    Alt,
    Shift,
}

fn modifier_named(name: &str) -> Option<Modifier> {
    const NAMES: &[(&str, Modifier)] = &[
        ("cmd", Modifier::Command),
        ("command", Modifier::Command),
        ("meta", Modifier::Command),
        ("super", Modifier::Command),
        ("mod", Modifier::Command),
        ("ctrl", Modifier::Command),
        ("control", Modifier::Command),
        ("ctl", Modifier::Command),
        ("⌘", Modifier::Command),
        ("⌃", Modifier::Command),
        ("alt", Modifier::Alt),
        ("opt", Modifier::Alt),
        ("option", Modifier::Alt),
        ("⌥", Modifier::Alt),
        ("shift", Modifier::Shift),
        ("⇧", Modifier::Shift),
    ];
    NAMES
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
        .map(|(_, modifier)| *modifier)
}

fn strip_glyph(text: &str) -> Option<(Modifier, &str)> {
    let mut chars = text.chars();
    let modifier = match chars.next()? {
        '⌘' | '⌃' => Modifier::Command,
        '⌥' => Modifier::Alt,
        '⇧' => Modifier::Shift,
        _ => return None,
    };
    Some((modifier, chars.as_str()))
}

fn add_modifier(mods: &mut Mods, modifier: Modifier, written: &str) -> Result<(), ChordError> {
    let slot = match modifier {
        Modifier::Command => &mut mods.command,
        Modifier::Alt => &mut mods.alt,
        Modifier::Shift => &mut mods.shift,
    };
    if *slot {
        return Err(ChordError::DuplicateModifier(written.to_owned()));
    }
    *slot = true;
    Ok(())
}

fn parse_key(name: &str) -> Option<egui::Key> {
    let alias = match name.to_ascii_lowercase().as_str() {
        "esc" => Some(egui::Key::Escape),
        "return" => Some(egui::Key::Enter),
        "del" => Some(egui::Key::Delete),
        "pgup" => Some(egui::Key::PageUp),
        "pgdn" => Some(egui::Key::PageDown),
        "spacebar" => Some(egui::Key::Space),
        "ins" => Some(egui::Key::Insert),
        "arrowup" => Some(egui::Key::ArrowUp),
        "arrowdown" => Some(egui::Key::ArrowDown),
        "arrowleft" => Some(egui::Key::ArrowLeft),
        "arrowright" => Some(egui::Key::ArrowRight),
        _ => None,
    };
    alias.or_else(|| egui::Key::from_name(name)).or_else(|| {
        egui::Key::ALL
            .iter()
            .copied()
            .find(|key| key.name().eq_ignore_ascii_case(name))
    })
}

fn mac_key(key: egui::Key) -> &'static str {
    match key {
        egui::Key::ArrowDown => "↓",
        egui::Key::ArrowLeft => "←",
        egui::Key::ArrowRight => "→",
        egui::Key::ArrowUp => "↑",
        egui::Key::Escape => "⎋",
        egui::Key::Tab => "⇥",
        egui::Key::Backspace => "⌫",
        egui::Key::Enter => "⏎",
        egui::Key::Delete => "⌦",
        other => other.name(),
    }
}

/// Why a chord was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ChordError {
    #[error("empty shortcut")]
    Empty,
    #[error("shortcut longer than {MAX_CHORD_TEXT} bytes")]
    TooLong,
    #[error("shortcut has no key")]
    MissingKey,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("unknown modifier `{0}`")]
    UnknownModifier(String),
    #[error("modifier `{0}` given twice")]
    DuplicateModifier(String),
}

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

/// Executable app workflows. Engine mutations inside a workflow still use the
/// engine owner; opening a dialog never advertises its later mutation as a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    MediaView,
    MediaSort,
    MediaSortBy(crate::media_panel::MediaSortKey),
    RequestDeleteMedia,
    CancelDeleteMedia,
    ConfirmDeleteMedia,
    RevealMedia,
    InsertText,
    Subtitles,
    SubtitleCue,
    Import,
    Export,
    Save,
    SaveAs,
    Back,
    RenameProject,
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
    RunExport,
    RetryExport,
    RenameScene,
    SceneRow,
    SelectItem,
    PreviewSnapshot,
    PreviewFullscreen,
    PreviewFit,
    PreviewZoomIn,
    PreviewZoomOut,
    PreviewZoomPreset,
}

/// Engine routing owns the executable identity. Delete follows the app's ripple
/// preference; every other route has one engine-owned command kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineAction {
    InsertMedia,
    Split,
    Delete,
    RippleDelete,
    AddTrack,
    CreateScene,
    DeleteScenes,
}
impl EngineAction {
    pub const fn command(self, ripple: bool) -> engine::CommandKind {
        use engine::CommandKind as K;
        match self {
            Self::InsertMedia => K::InsertItem,
            Self::Split => K::SplitAt,
            Self::Delete if ripple => K::RippleDelete,
            Self::Delete => K::Delete,
            Self::RippleDelete => K::RippleDelete,
            Self::AddTrack => K::AddTrack,
            Self::CreateScene => K::CreateScene,
            Self::DeleteScenes => K::DeleteScenes,
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
    Pending {
        owner: &'static str,
        ticket: &'static str,
    },
}

/// One app action. Defaults are canonical chord text.
#[derive(Clone, Copy, Debug)]
pub struct ActionSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub category: ActionCategory,
    pub target: ActionTarget,
    pub defaults: &'static [&'static str],
    /// The action is intercepted only when the in-app clipboard has an entry.
    pub requires_clipboard: bool,
}

impl ActionSpec {
    const fn engine(
        id: &'static str,
        label: &'static str,
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
        label: &'static str,
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
            requires_clipboard: false,
        }
    }

    const fn pending(
        id: &'static str,
        label: &'static str,
        category: ActionCategory,
        owner: &'static str,
        ticket: &'static str,
        defaults: &'static [&'static str],
        requires_clipboard: bool,
    ) -> Self {
        Self {
            id,
            label,
            category,
            target: ActionTarget::Pending { owner, ticket },
            defaults,
            requires_clipboard,
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
                    | UiAction::PreviewZoomPreset,
                ..
            }
        )
    }

    pub const fn available(&self) -> bool {
        !matches!(self.target, ActionTarget::Pending { .. })
    }

    pub fn pending_ticket(&self) -> Option<&'static str> {
        match self.target {
            ActionTarget::Pending { ticket, .. } => Some(ticket),
            _ => None,
        }
    }

    fn owner(self) -> Option<&'static str> {
        match self.target {
            ActionTarget::Ui { owner, .. } => Some(owner),
            ActionTarget::Pending { owner, .. } => Some(owner),
            ActionTarget::Engine(_) => None,
        }
    }
}

/// Every key, menu and button action in the desktop app. Actions whose payload
/// belongs to an open ticket are registered here but marked unavailable.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec::ui(
        "media.view",
        "Toggle media view",
        ActionCategory::View,
        "media library preferences",
        UiAction::MediaView,
        &[],
    ),
    ActionSpec::ui(
        "media.sort",
        "Toggle media sort order",
        ActionCategory::View,
        "media library preferences",
        UiAction::MediaSort,
        &[],
    ),
    ActionSpec::ui(
        "media.sort.name",
        "Sort media by name",
        ActionCategory::View,
        "media library preferences",
        UiAction::MediaSortBy(crate::media_panel::MediaSortKey::Name),
        &[],
    ),
    ActionSpec::ui(
        "media.sort.type",
        "Sort media by type",
        ActionCategory::View,
        "media library preferences",
        UiAction::MediaSortBy(crate::media_panel::MediaSortKey::Type),
        &[],
    ),
    ActionSpec::ui(
        "media.sort.duration",
        "Sort media by duration",
        ActionCategory::View,
        "media library preferences",
        UiAction::MediaSortBy(crate::media_panel::MediaSortKey::Duration),
        &[],
    ),
    ActionSpec::ui(
        "media.sort.size",
        "Sort media by size",
        ActionCategory::View,
        "media library preferences",
        UiAction::MediaSortBy(crate::media_panel::MediaSortKey::Size),
        &[],
    ),
    ActionSpec::ui(
        "media.delete",
        "Request media deletion",
        ActionCategory::Edit,
        "media removal confirmation",
        UiAction::RequestDeleteMedia,
        &[],
    ),
    ActionSpec::ui(
        "media.cancel_delete",
        "Cancel media deletion",
        ActionCategory::Edit,
        "media removal confirmation",
        UiAction::CancelDeleteMedia,
        &[],
    ),
    ActionSpec::ui(
        "media.confirm_delete",
        "Confirm media deletion",
        ActionCategory::Edit,
        "media removal confirmation workflow",
        UiAction::ConfirmDeleteMedia,
        &[],
    ),
    ActionSpec::ui(
        "media.reveal",
        "Reveal media",
        ActionCategory::File,
        "media library reveal",
        UiAction::RevealMedia,
        &[],
    ),
    ActionSpec::ui(
        "text.insert",
        "Insert text",
        ActionCategory::Edit,
        "text insertion workflow",
        UiAction::InsertText,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.cue",
        "Select subtitle cue",
        ActionCategory::Edit,
        "subtitle cue selection",
        UiAction::SubtitleCue,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.cancel",
        "Cancel subtitle operation",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.consent.approve",
        "Approve subtitle upload",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.consent.deny",
        "Deny subtitle upload",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.generate",
        "Generate subtitles",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.translate",
        "Translate subtitles",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.split",
        "Split subtitle cue",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.merge",
        "Merge subtitle cues",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.earlier",
        "Nudge subtitle earlier",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.later",
        "Nudge subtitle later",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.select_all",
        "Select all subtitle cues",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.models",
        "Manage subtitle model",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.bilingual",
        "Toggle bilingual subtitles",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.force",
        "Toggle subtitle generation override",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "subtitles.dismiss",
        "Dismiss subtitle notice",
        ActionCategory::Edit,
        "subtitle editor workflow",
        UiAction::Subtitles,
        &[],
    ),
    ActionSpec::ui(
        "file.import",
        "Import media",
        ActionCategory::File,
        "workspace import picker",
        UiAction::Import,
        &[],
    ),
    ActionSpec::ui(
        "file.export",
        "Export",
        ActionCategory::File,
        "export dialog",
        UiAction::Export,
        &[],
    ),
    ActionSpec::ui(
        "project.save",
        "Save",
        ActionCategory::File,
        "workspace lifecycle",
        UiAction::Save,
        &[],
    ),
    ActionSpec::ui(
        "project.save_as",
        "Save as",
        ActionCategory::File,
        "workspace lifecycle",
        UiAction::SaveAs,
        &[],
    ),
    ActionSpec::ui(
        "project.back",
        "Back to projects",
        ActionCategory::File,
        "workspace lifecycle",
        UiAction::Back,
        &[],
    ),
    ActionSpec::ui(
        "project.rename",
        "Rename project",
        ActionCategory::File,
        "project rename input",
        UiAction::RenameProject,
        &[],
    ),
    ActionSpec::engine(
        "media.insert",
        "Add to timeline",
        ActionCategory::File,
        EngineAction::InsertMedia,
        &[],
    ),
    ActionSpec::ui(
        "import.cancel",
        "Cancel import",
        ActionCategory::File,
        "workspace import queue",
        UiAction::CancelImport,
        &[],
    ),
    ActionSpec::pending(
        "edit.copy",
        "Copy",
        ActionCategory::Edit,
        "clipboard",
        "p1-parity/20",
        &["Ctrl+C"],
        false,
    ),
    ActionSpec::pending(
        "edit.paste",
        "Paste",
        ActionCategory::Edit,
        "clipboard",
        "p1-parity/20",
        &["Ctrl+V"],
        true,
    ),
    ActionSpec::engine(
        "timeline.split",
        "Split at playhead",
        ActionCategory::Edit,
        EngineAction::Split,
        &["S", "Ctrl+B"],
    ),
    ActionSpec::pending(
        "timeline.split_left",
        "Split and remove left",
        ActionCategory::Edit,
        "timeline commands",
        "p1-parity/16",
        &["Q"],
        false,
    ),
    ActionSpec::pending(
        "timeline.split_right",
        "Split and remove right",
        ActionCategory::Edit,
        "timeline commands",
        "p1-parity/16",
        &["W"],
        false,
    ),
    ActionSpec::engine(
        "timeline.delete",
        "Delete selection",
        ActionCategory::Edit,
        EngineAction::Delete,
        &["Backspace", "Delete"],
    ),
    ActionSpec::engine(
        "timeline.ripple_delete",
        "Ripple delete",
        ActionCategory::Edit,
        EngineAction::RippleDelete,
        &["Shift+Backspace", "Shift+Delete"],
    ),
    ActionSpec::ui(
        "transport.play",
        "Play/Pause",
        ActionCategory::Playback,
        "playback transport",
        UiAction::Play,
        &["Space"],
    ),
    ActionSpec::ui(
        "transport.pause",
        "Pause",
        ActionCategory::Playback,
        "playback transport",
        UiAction::Pause,
        &["K"],
    ),
    ActionSpec::ui(
        "transport.back",
        "Seek back 1 second",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::BackSecond,
        &["J"],
    ),
    ActionSpec::ui(
        "transport.forward",
        "Seek forward 1 second",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::ForwardSecond,
        &["L"],
    ),
    ActionSpec::ui(
        "transport.jump_back",
        "Jump back 1 second",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::BackSecond,
        &["Shift+Left"],
    ),
    ActionSpec::ui(
        "transport.jump_forward",
        "Jump forward 1 second",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::ForwardSecond,
        &["Shift+Right"],
    ),
    ActionSpec::ui(
        "transport.frame_back",
        "Step one frame back",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::FrameBack,
        &["Left"],
    ),
    ActionSpec::ui(
        "transport.frame_forward",
        "Step one frame forward",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::FrameForward,
        &["Right"],
    ),
    ActionSpec::ui(
        "transport.start",
        "Go to start",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::Start,
        &["Home", "Enter"],
    ),
    ActionSpec::ui(
        "transport.end",
        "Go to end",
        ActionCategory::Navigation,
        "playback transport",
        UiAction::End,
        &["End"],
    ),
    ActionSpec::ui(
        "timeline.select_all",
        "Select all",
        ActionCategory::Selection,
        "timeline selection",
        UiAction::SelectAll,
        &["Ctrl+A"],
    ),
    ActionSpec::ui(
        "timeline.deselect_all",
        "Deselect all",
        ActionCategory::Selection,
        "timeline selection",
        UiAction::DeselectAll,
        &[],
    ),
    ActionSpec::pending(
        "timeline.duplicate",
        "Duplicate",
        ActionCategory::Selection,
        "timeline commands",
        "p1-parity/16",
        &["Ctrl+D"],
        false,
    ),
    ActionSpec::ui(
        "selection.escape",
        "Cancel interaction or deselect",
        ActionCategory::Selection,
        "timeline selection",
        UiAction::Escape,
        &["Escape"],
    ),
    ActionSpec::ui(
        "edit.undo",
        "Undo",
        ActionCategory::History,
        "app and engine history",
        UiAction::Undo,
        &["Ctrl+Z"],
    ),
    ActionSpec::ui(
        "edit.redo",
        "Redo",
        ActionCategory::History,
        "app and engine history",
        UiAction::Redo,
        &["Ctrl+Shift+Z", "Ctrl+Y"],
    ),
    ActionSpec::ui(
        "timeline.snap",
        "Toggle snapping",
        ActionCategory::Timeline,
        "timeline view",
        UiAction::Snap,
        &["N"],
    ),
    ActionSpec::ui(
        "timeline.ripple",
        "Toggle ripple editing",
        ActionCategory::Timeline,
        "timeline view",
        UiAction::Ripple,
        &[],
    ),
    ActionSpec::ui(
        "timeline.zoom_in",
        "Zoom in",
        ActionCategory::Timeline,
        "timeline view",
        UiAction::ZoomIn,
        &[],
    ),
    ActionSpec::ui(
        "timeline.zoom_out",
        "Zoom out",
        ActionCategory::Timeline,
        "timeline view",
        UiAction::ZoomOut,
        &[],
    ),
    ActionSpec::engine(
        "timeline.add_track",
        "Add track",
        ActionCategory::Timeline,
        EngineAction::AddTrack,
        &[],
    ),
    ActionSpec::ui(
        "scene.open",
        "Open scenes",
        ActionCategory::Timeline,
        "scene switcher",
        UiAction::OpenScenes,
        &[],
    ),
    ActionSpec::ui(
        "scene.close",
        "Close scenes",
        ActionCategory::Timeline,
        "scene switcher",
        UiAction::CloseScenes,
        &[],
    ),
    ActionSpec::engine(
        "scene.create",
        "Create scene",
        ActionCategory::Timeline,
        EngineAction::CreateScene,
        &[],
    ),
    ActionSpec::ui(
        "scene.select",
        "Select scenes",
        ActionCategory::Timeline,
        "scene switcher",
        UiAction::SelectScenes,
        &[],
    ),
    ActionSpec::ui(
        "scene.delete",
        "Delete scenes",
        ActionCategory::Timeline,
        "scene delete confirmation",
        UiAction::RequestDeleteScenes,
        &[],
    ),
    ActionSpec::ui(
        "scene.cancel_delete",
        "Cancel scene delete",
        ActionCategory::Timeline,
        "scene switcher",
        UiAction::CancelDeleteScenes,
        &[],
    ),
    ActionSpec::engine(
        "scene.confirm_delete",
        "Confirm scene delete",
        ActionCategory::Timeline,
        EngineAction::DeleteScenes,
        &[],
    ),
    ActionSpec::ui(
        "appearance.theme",
        "Toggle theme",
        ActionCategory::View,
        "preference store",
        UiAction::Theme,
        &[],
    ),
    ActionSpec::ui(
        "appearance.locale",
        "Toggle language",
        ActionCategory::View,
        "preference store",
        UiAction::Locale,
        &[],
    ),
    ActionSpec::ui(
        "layout.reset",
        "Reset layout",
        ActionCategory::View,
        "preference store",
        UiAction::ResetLayout,
        &[],
    ),
    ActionSpec::ui(
        "inspector.project",
        "Project properties",
        ActionCategory::View,
        "selection inspector",
        UiAction::ProjectInspector,
        &[],
    ),
    ActionSpec::ui(
        "inspector.details",
        "Element details",
        ActionCategory::View,
        "selection inspector",
        UiAction::DetailsInspector,
        &[],
    ),
    ActionSpec::ui(
        "preview.retry",
        "Retry preview",
        ActionCategory::View,
        "playback transport",
        UiAction::RetryPreview,
        &[],
    ),
    ActionSpec::ui(
        "models.open",
        "Manage models",
        ActionCategory::View,
        "model manager",
        UiAction::OpenModels,
        &[],
    ),
    ActionSpec::ui(
        "export.run",
        "Run export",
        ActionCategory::View,
        "export dialog",
        UiAction::RunExport,
        &[],
    ),
    ActionSpec::ui(
        "export.retry",
        "Retry export",
        ActionCategory::View,
        "export dialog",
        UiAction::RetryExport,
        &[],
    ),
    ActionSpec::ui(
        "scene.rename",
        "Rename scene",
        ActionCategory::View,
        "scene rename input",
        UiAction::RenameScene,
        &[],
    ),
    ActionSpec::ui(
        "scene.row",
        "Choose scene",
        ActionCategory::View,
        "scene switcher",
        UiAction::SceneRow,
        &[],
    ),
    ActionSpec::ui(
        "timeline.item",
        "Select item",
        ActionCategory::View,
        "timeline selection",
        UiAction::SelectItem,
        &[],
    ),
    ActionSpec::ui(
        "preview.snapshot",
        "Open snapshot dialog",
        ActionCategory::View,
        "preview snapshot dialog",
        UiAction::PreviewSnapshot,
        &[],
    ),
    ActionSpec::ui(
        "preview.fullscreen",
        "Toggle preview fullscreen",
        ActionCategory::View,
        "preview viewport",
        UiAction::PreviewFullscreen,
        &[],
    ),
    ActionSpec::ui(
        "preview.zoom.fit",
        "Fit preview",
        ActionCategory::View,
        "preview view",
        UiAction::PreviewFit,
        &[],
    ),
    ActionSpec::ui(
        "preview.zoom.in",
        "Zoom preview in",
        ActionCategory::View,
        "preview view",
        UiAction::PreviewZoomIn,
        &[],
    ),
    ActionSpec::ui(
        "preview.zoom.out",
        "Zoom preview out",
        ActionCategory::View,
        "preview view",
        UiAction::PreviewZoomOut,
        &[],
    ),
    ActionSpec::ui(
        "preview.zoom",
        "Set preview zoom percentage",
        ActionCategory::View,
        "preview view",
        UiAction::PreviewZoomPreset,
        &[],
    ),
];

pub fn actions() -> &'static [ActionSpec] {
    ACTIONS
}

pub fn action(id: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.id == id)
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
    for spec in ACTIONS {
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
            ActionTarget::Pending { owner, ticket } => {
                ("—".to_owned(), format!("{owner} (unavailable; `{ticket}`)"))
            }
        };
        text.push_str(&format!(
            "| `{}` — {} | {} | {} | {} | {} |\n",
            spec.id,
            spec.label,
            spec.category.label(),
            defaults,
            engine,
            owner
        ));
    }
    text
}

/// Persisted binding overrides and per-action `isCustomized` markers. Defaults
/// and metadata stay in [`ACTIONS`], so this document only names deltas.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeybindingPreferences {
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
    #[serde(rename = "isCustomized", default)]
    pub is_customized: BTreeSet<String>,
}

impl KeybindingPreferences {
    /// Structural bounds and parseability. Unknown action ids are retained so
    /// the dispatcher can report them instead of silently deleting user data.
    pub fn validate(&self) -> Result<(), KeybindingError> {
        if self.bindings.len() > MAX_ACTIONS || self.is_customized.len() > MAX_ACTIONS {
            return Err(KeybindingError::TooManyActions);
        }
        for (action, chords) in &self.bindings {
            if !valid_action_id(action) {
                return Err(KeybindingError::InvalidActionId(action.clone()));
            }
            if chords.len() > MAX_BINDINGS_PER_ACTION {
                return Err(KeybindingError::TooManyBindings(action.clone()));
            }
            for text in chords {
                Chord::parse(text).map_err(|error| KeybindingError::InvalidChord {
                    text: text.clone(),
                    error,
                })?;
            }
        }
        for action in &self.is_customized {
            if !valid_action_id(action) {
                return Err(KeybindingError::InvalidActionId(action.clone()));
            }
            if !self.bindings.contains_key(action) {
                return Err(KeybindingError::CustomizedWithoutBinding(action.clone()));
            }
        }
        Ok(())
    }

    /// Startup repair for a damaged binding map: drop only invalid entries and
    /// keep the rest of the user's preferences. Returns whether anything changed.
    pub fn sanitize(mut self) -> (Self, bool) {
        let mut changed = false;
        let mut bindings = BTreeMap::new();
        for (action, chords) in std::mem::take(&mut self.bindings) {
            if bindings.len() >= MAX_ACTIONS
                || !valid_action_id(&action)
                || chords.len() > MAX_BINDINGS_PER_ACTION
                || chords.iter().any(|text| Chord::parse(text).is_err())
            {
                changed = true;
                continue;
            }
            bindings.insert(action, chords);
        }
        self.bindings = bindings;
        let mut customized = BTreeSet::new();
        for action in std::mem::take(&mut self.is_customized) {
            if customized.len() >= MAX_ACTIONS
                || !valid_action_id(&action)
                || !self.bindings.contains_key(&action)
            {
                changed = true;
                continue;
            }
            customized.insert(action);
        }
        self.is_customized = customized;
        (self, changed)
    }

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

    /// Restore every action to its registry defaults.
    pub fn reset_all(&mut self) -> bool {
        let changed = !self.bindings.is_empty() || !self.is_customized.is_empty();
        self.bindings.clear();
        self.is_customized.clear();
        changed
    }

    fn check_conflicts(&self, action: &str, chords: &[String]) -> Result<(), KeybindingError> {
        let mut used = BTreeMap::new();
        for spec in ACTIONS {
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

fn valid_action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A portable binding profile for the import/export entry points in p1-parity/04.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeybindingProfile {
    pub format: String,
    pub version: u64,
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
    #[serde(rename = "isCustomized", default)]
    pub is_customized: BTreeSet<String>,
}

impl KeybindingProfile {
    pub fn from_preferences(preferences: KeybindingPreferences) -> Self {
        Self {
            format: KEYBINDING_FORMAT.to_owned(),
            version: KEYBINDING_VERSION,
            bindings: preferences.bindings,
            is_customized: preferences.is_customized,
        }
    }

    /// Import rejects unsupported schemas and oversized documents before any
    /// state changes. Unknown actions survive for the resolver to report.
    pub fn import(bytes: &[u8]) -> Result<KeybindingPreferences, KeybindingError> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(KeybindingError::TooLarge);
        }
        let profile: Self =
            serde_json::from_slice(bytes).map_err(|_| KeybindingError::Malformed)?;
        if profile.format != KEYBINDING_FORMAT {
            return Err(KeybindingError::WrongFormat(profile.format));
        }
        if profile.version != KEYBINDING_VERSION {
            return Err(KeybindingError::UnsupportedVersion {
                found: profile.version,
                expected: KEYBINDING_VERSION,
            });
        }
        let preferences = KeybindingPreferences {
            bindings: profile.bindings,
            is_customized: profile.is_customized,
        };
        preferences.validate()?;
        Ok(preferences)
    }

    pub fn export(preferences: KeybindingPreferences) -> Result<Vec<u8>, KeybindingError> {
        preferences.validate()?;
        let profile = Self::from_preferences(preferences);
        let bytes = serde_json::to_vec(&profile).map_err(|_| KeybindingError::Malformed)?;
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(KeybindingError::TooLarge);
        }
        Ok(bytes)
    }
}

/// Why binding storage or import was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum KeybindingError {
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("invalid chord `{text}`: {error}")]
    InvalidChord { text: String, error: ChordError },
    #[error("binding `{chord}` is already used by `{existing}`")]
    Conflict {
        chord: String,
        existing: String,
        action: String,
    },
    #[error("binding `{0}` is listed twice")]
    DuplicateBinding(String),
    #[error("too many bindings for `{0}`")]
    TooManyBindings(String),
    #[error("too many keybinding entries")]
    TooManyActions,
    #[error("`{0}` is marked customized without a binding entry")]
    CustomizedWithoutBinding(String),
    #[error("invalid action id `{0}`")]
    InvalidActionId(String),
    #[error("keybinding profile is too large")]
    TooLarge,
    #[error("keybinding profile format `{0}` is unsupported")]
    WrongFormat(String),
    #[error("keybinding profile version {found} is unsupported; expected {expected}")]
    UnsupportedVersion { found: u64, expected: u64 },
    #[error("keybinding profile is malformed")]
    Malformed,
}

/// A non-fatal binding problem reported by the resolver. Unknown actions are
/// kept in the document and surfaced here rather than silently ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeybindingIssue {
    UnknownAction(String),
    InvalidChord {
        action: String,
        text: String,
    },
    Conflict {
        chord: String,
        first: String,
        second: String,
    },
    Duplicate {
        action: String,
        chord: String,
    },
}

impl fmt::Display for KeybindingIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAction(action) => {
                write!(formatter, "unknown keybinding action `{action}`")
            }
            Self::InvalidChord { action, text } => {
                write!(formatter, "invalid chord `{text}` for `{action}`")
            }
            Self::Conflict {
                chord,
                first,
                second,
            } => write!(
                formatter,
                "chord `{chord}` is bound to both `{first}` and `{second}`"
            ),
            Self::Duplicate { action, chord } => {
                write!(formatter, "chord `{chord}` is listed twice for `{action}`")
            }
        }
    }
}

/// Resolved chords and the non-fatal problems found while resolving them.
#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: BTreeMap<Chord, &'static str>,
    issues: Vec<KeybindingIssue>,
}

impl Keymap {
    pub fn resolve(preferences: &KeybindingPreferences) -> Self {
        let mut bindings: BTreeMap<Chord, &'static str> = BTreeMap::new();
        let mut issues = Vec::new();
        let mut known = BTreeSet::new();
        for spec in ACTIONS {
            if !spec.bindable() {
                continue;
            }
            known.insert(spec.id);
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
        Self { bindings, issues }
    }

    pub fn issues(&self) -> &[KeybindingIssue] {
        &self.issues
    }

    /// Consume one exact key press and return the action it triggers. Blocked
    /// contexts swallow the press without consuming it; a paste without an
    /// in-app entry falls through to the media paste owner.
    pub fn take(&self, input: &mut egui::InputState, context: DispatchContext) -> Dispatch {
        self.take_on_platform(input, context, Platform::current())
    }

    fn take_on_platform(
        &self,
        input: &mut egui::InputState,
        context: DispatchContext,
        platform: Platform,
    ) -> Dispatch {
        if let Some(reason) = context.blocked() {
            return Dispatch::Blocked(reason);
        }
        let mut found = None;
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            {
                let chord = Chord {
                    mods: Mods {
                        command: modifiers.command,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                    },
                    key: *key,
                };
                if chord.matches(platform, *modifiers)
                    && let Some(action) = self.bindings.get(&chord).copied()
                {
                    found = Some((*key, chord, action));
                    break;
                }
            }
        }
        let Some((key, chord, action)) = found else {
            return Dispatch::NoMatch;
        };
        let Some(spec) = self::action(action).filter(|spec| spec.bindable()) else {
            return Dispatch::NoMatch;
        };
        if spec.requires_clipboard && !context.clipboard_entry {
            return Dispatch::FellThrough(action);
        }
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key {
                    key: pressed_key,
                    pressed: true,
                    modifiers,
                    ..
                } if *pressed_key == key && chord.matches(platform, *modifiers)
            )
        });
        if spec.available() {
            Dispatch::Action { action, chord }
        } else {
            Dispatch::Unavailable {
                action,
                owner: spec.owner().unwrap_or("another ticket"),
            }
        }
    }
}

/// Why global dispatch is inert. The editor still lets the focused widget see
/// the key event so text editing and Escape-to-blur keep working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedReason {
    Overlay,
    ProjectLoading,
    Recording,
    TextInput,
}

/// Everything outside the keymap that can block global dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DispatchContext {
    pub overlay_open: bool,
    pub project_loading: bool,
    pub recording: bool,
    pub text_input: bool,
    /// The in-app clipboard has an item or keyframe entry. Presence, not payload,
    /// is the dispatch contract; p1-parity/20 owns the payload.
    pub clipboard_entry: bool,
}

impl DispatchContext {
    pub fn blocked(&self) -> Option<BlockedReason> {
        if self.overlay_open {
            Some(BlockedReason::Overlay)
        } else if self.project_loading {
            Some(BlockedReason::ProjectLoading)
        } else if self.recording {
            Some(BlockedReason::Recording)
        } else if self.text_input {
            Some(BlockedReason::TextInput)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Blocked(BlockedReason),
    NoMatch,
    FellThrough(&'static str),
    Unavailable {
        action: &'static str,
        owner: &'static str,
    },
    Action {
        action: &'static str,
        chord: Chord,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::catalog::{PREFERENCES_VERSION, PreferenceIssue, PreferenceStore, Preferences};

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
                ACTIONS.iter().any(|spec| spec.category == *category),
                "{} has no actions",
                category.label()
            );
        }
        let mut ids = BTreeSet::new();
        let mut chords = BTreeMap::new();
        for spec in ACTIONS {
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
        std::fs::write(
            store.path(),
            serde_json::to_vec(&serde_json::json!({
                "version": PREFERENCES_VERSION,
                "locale": "en",
                "dark": true,
                "panels": [0.25, 0.25, 0.5],
                "keybindings": {
                    "bindings": {
                        "timeline.snap": ["Ctrl+Shift+N"],
                        "bad action": []
                    },
                    "isCustomized": ["timeline.snap", "missing.action"]
                }
            }))
            .unwrap(),
        )
        .unwrap();
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
        let loaded = store.load().unwrap();
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
            Dispatch::Unavailable {
                action: "edit.paste",
                owner: "clipboard",
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
    fn keyboard_guide_matches_the_registry() {
        let guide = include_str!("../../../docs/product/keyboard.md");
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
