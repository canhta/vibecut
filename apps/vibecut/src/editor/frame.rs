//! Owns editor frame composition.
#[cfg(test)]
pub(crate) mod bench;
mod input;
mod layout;
mod lend;
mod player;
mod project_delete;
mod route;
mod shortcuts;
mod topbar;
mod view;
mod workflow;

pub use input::{AppCommand, PreviewZoom};

use super::state::PreferenceHistory;
use super::widgets::Widgets;
use super::{BTreeMap, Context, Rect, ShortcutTab};
use crate::{
    actions::{self, KeybindingIssue, Keymap},
    egui,
};

/// Transient editor-window chrome, independent of every other window.
pub(super) struct State {
    pub(super) layout_drag: Option<[f32; 3]>,
    pub(super) rename: Option<String>,
    pub(super) rename_focus: bool,
    /// The header Delete confirmation is open.
    pub(super) delete_open: bool,
    /// Header controls painted this frame, routed by the frame after `show`.
    pub(super) painted: Vec<(&'static str, Option<&'static str>, egui::Response)>,
    /// Automation records for every control the editor painted.
    pub(super) widgets: Widgets,
    pub(super) panels: BTreeMap<&'static str, Rect>,
    /// App-owned preference history, deliberately separate from the project's.
    pub(super) preference_history: PreferenceHistory,
    pub(super) keymap: Keymap,
    pub(super) keymap_source: actions::KeybindingPreferences,
    pub(super) keybinding_issues: Vec<KeybindingIssue>,
    /// A host-set capture flag; global keys wait. A running voice-over take counts
    /// too, read from its dialog by `dispatch_context`.
    pub(super) recording_active: bool,
    /// Whether a text field held keyboard focus this frame; Paste then stays in the field.
    pub(super) text_focused: bool,
    pub(super) shortcuts_open: bool,
    pub(super) shortcut_tab: ShortcutTab,
    pub(super) shortcut_recording: Option<&'static str>,
    pub(super) shortcut_error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self::new(&crate::preferences::Preferences::default())
    }
}

impl State {
    pub(super) fn new(preferences: &crate::preferences::Preferences) -> Self {
        let keymap = Keymap::resolve(&preferences.keybindings);
        Self {
            layout_drag: None,
            rename: None,
            rename_focus: false,
            delete_open: false,
            painted: Vec::new(),
            widgets: Widgets::default(),
            panels: BTreeMap::new(),
            preference_history: PreferenceHistory::default(),
            keybinding_issues: keymap.issues().to_vec(),
            keymap_source: preferences.keybindings.clone(),
            keymap,
            recording_active: false,
            text_focused: false,
            shortcuts_open: false,
            shortcut_tab: ShortcutTab::Timeline,
            shortcut_recording: None,
            shortcut_error: None,
        }
    }
    /// Open the shortcuts dialog on its first tab.
    pub(super) fn open_shortcuts(&mut self) {
        self.shortcuts_open = true;
        self.shortcut_tab = ShortcutTab::Timeline;
        self.shortcut_recording = None;
        self.shortcut_error = None;
    }
    pub(super) fn close_shortcuts(&mut self) {
        self.shortcuts_open = false;
        self.shortcut_recording = None;
        self.shortcut_error = None;
    }
    /// Switch tabs; a recording gesture belongs to the old tab.
    pub(super) fn set_shortcut_tab(&mut self, tab: ShortcutTab) {
        self.shortcut_tab = tab;
        self.shortcut_recording = None;
        self.shortcut_error = None;
    }
}

pub(super) enum Action {
    Activate(&'static str),
    Rename(String),
}

pub(super) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    topbar::show(state, cx, ui)
}
