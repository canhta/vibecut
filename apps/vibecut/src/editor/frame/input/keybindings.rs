//! Maintains editor shortcut bindings.

use crate::editor::state::ShortcutConflict;
use crate::editor::{AppCommand, Editor, Error, Keymap, Result, ShortcutTab, Text, actions, egui};
use crate::i18n;

impl Editor {
    /// One binding change command for the shortcuts dialog; it uses the same
    /// app preference history as every other user-visible preference edit.
    pub fn set_keybinding(&mut self, action: &str, chords: Vec<String>) -> Result<()> {
        let mut next = self.preferences.keybindings.clone();
        next.set_action(action, chords)?;
        self.store_keybindings(next)
    }
    pub fn reset_keybinding(&mut self, action: &str) -> Result<()> {
        let mut next = self.preferences.keybindings.clone();
        next.reset_action(action)?;
        self.store_keybindings(next)
    }
    pub fn reset_keybindings(&mut self) -> Result<()> {
        let mut next = self.preferences.keybindings.clone();
        next.reset_all();
        self.store_keybindings(next)
    }
    pub(in crate::editor) fn store_keybindings(
        &mut self,
        keybindings: actions::KeybindingPresets,
    ) -> Result<()> {
        self.set_preference("keybindings", serde_json::to_value(keybindings)?)
    }
    /// Open the shortcuts dialog with a working copy of the stored map. Rows
    /// read the copy; Save commits it and Cancel or Escape drops it.
    pub fn open_shortcuts(&mut self) {
        self.frame.open_shortcuts();
        self.frame.shortcut_draft = Some(self.preferences.keybindings.active().clone());
    }
    pub(in crate::editor) fn close_shortcuts(&mut self) {
        self.frame.close_shortcuts();
    }
    /// Save: commit the working copy into the active profile as one preference
    /// edit, then close. The other profiles are carried over unchanged.
    pub(in crate::editor) fn save_shortcuts(&mut self) -> Result<()> {
        let Some(draft) = self.frame.shortcut_draft.clone() else {
            return Ok(());
        };
        if draft != *self.preferences.keybindings.active() {
            let mut presets = self.preferences.keybindings.clone();
            *presets.active_mut() = draft;
            self.store_keybindings(presets)?;
        }
        self.close_shortcuts();
        Ok(())
    }
    /// Make another named profile the active one. It is one preference edit;
    /// the dialog's working copy follows the newly active profile, so unsaved
    /// edits to the previous one are dropped.
    pub fn select_shortcut_preset(&mut self, preset: actions::KeybindingPreset) -> Result<()> {
        let mut next = self.preferences.keybindings.clone();
        next.active = preset;
        self.store_keybindings(next)?;
        self.frame.shortcut_draft = Some(self.preferences.keybindings.active().clone());
        self.frame.shortcut_error = None;
        self.frame.shortcut_conflict = None;
        self.frame.shortcut_recording = None;
        Ok(())
    }
    /// The dialog's working copy, or the active profile when no dialog is open.
    fn shortcut_working_copy(&self) -> actions::KeybindingPreferences {
        self.frame
            .shortcut_draft
            .clone()
            .unwrap_or_else(|| self.preferences.keybindings.active().clone())
    }
    pub fn shortcut_tab(&self) -> ShortcutTab {
        self.frame.shortcut_tab
    }
    pub fn set_shortcut_tab(&mut self, tab: ShortcutTab) {
        self.frame.set_shortcut_tab(tab);
    }
    /// Record one chord for an action. With the dialog open the chord goes to
    /// the working copy, and a chord another action owns opens the conflict
    /// prompt while the edit box keeps recording. With the dialog closed it
    /// stores at once. Tests also use it for chord text no egui key event can
    /// produce.
    pub fn record_shortcut(&mut self, action: &str, chord: &str) -> Result<()> {
        if !self.frame.shortcuts_open {
            return self.dispatch(AppCommand::SetKeybinding {
                action: action.to_owned(),
                chords: vec![chord.to_owned()],
            });
        }
        let mut next = self.shortcut_working_copy();
        match next.set_action(action, vec![chord.to_owned()]) {
            Ok(()) => {
                self.frame.shortcut_draft = Some(next);
                self.frame.shortcut_error = None;
                self.frame.shortcut_recording = None;
                Ok(())
            }
            Err(actions::KeybindingError::Conflict {
                chord: canonical,
                existing,
                ..
            }) => {
                self.frame.shortcut_error = None;
                self.frame.shortcut_conflict = Some(ShortcutConflict {
                    action: action.to_owned(),
                    chord: canonical,
                    owner: existing,
                });
                Ok(())
            }
            Err(error) => {
                let error = Error::Keybinding(error);
                self.frame.shortcut_error = Some(self.shortcut_error_message(&error));
                Err(error)
            }
        }
    }
    /// Override in the conflict prompt: the recording action takes the chord
    /// and its owner loses it. The edit box closes.
    pub(in crate::editor) fn override_shortcut_conflict(&mut self) {
        let Some(conflict) = self.frame.shortcut_conflict.take() else {
            return;
        };
        let mut next = self.shortcut_working_copy();
        match next.override_chord(&conflict.action, &conflict.chord, &conflict.owner) {
            Ok(()) => {
                self.frame.shortcut_draft = Some(next);
                self.frame.shortcut_error = None;
                self.frame.shortcut_recording = None;
            }
            Err(error) => {
                self.frame.shortcut_error =
                    Some(self.shortcut_error_message(&Error::Keybinding(error)));
            }
        }
    }
    /// Cancel in the conflict prompt: the edit box stays open for another chord.
    pub(in crate::editor) fn cancel_shortcut_conflict(&mut self) {
        self.frame.shortcut_conflict = None;
    }
    /// Reset one action, or every action, in the working copy. Save commits it.
    pub(in crate::editor) fn reset_shortcut_draft(&mut self, action: Option<&str>) {
        let mut next = self.shortcut_working_copy();
        let result = match action {
            Some(id) => next.reset_action(id).map(|_| ()),
            None => {
                next.reset_all();
                Ok(())
            }
        };
        match result {
            Ok(()) => {
                self.frame.shortcut_draft = Some(next);
                self.frame.shortcut_error = None;
            }
            Err(error) => {
                self.frame.shortcut_error =
                    Some(self.shortcut_error_message(&Error::Keybinding(error)));
            }
        }
    }
    pub(in crate::editor) fn shortcut_error_message(&self, error: &Error) -> String {
        let locale = self.preferences.locale;
        match error {
            Error::Keybinding(actions::KeybindingError::Conflict {
                chord, existing, ..
            }) => {
                let action = actions::action(existing)
                    .map_or(existing.as_str(), |spec| spec.label.text(locale));
                let chord = actions::Chord::parse(chord).map_or_else(
                    |_| chord.clone(),
                    |parsed| parsed.display(actions::Platform::current()),
                );
                i18n::format(
                    locale,
                    Text::ShortcutsErrorConflict,
                    &[("chord", &chord), ("action", action)],
                )
            }
            Error::Keybinding(actions::KeybindingError::InvalidChord { text, .. }) => {
                i18n::format(locale, Text::ShortcutsErrorInvalid, &[("text", text)])
            }
            other => other.to_string(),
        }
    }
    /// Read the first key event while a row records. Every key, Escape included,
    /// is recorded as the chord. The edit box ends by Cancel in the conflict
    /// prompt, a second click on its row, or closing the dialog.
    pub(in crate::editor) fn capture_shortcut(&mut self, ctx: &egui::Context) {
        let Some(action) = self.frame.shortcut_recording else {
            return;
        };
        let mut pressed = None;
        ctx.input_mut(|input| {
            let index = input.events.iter().position(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            });
            if let Some(index) = index {
                if let egui::Event::Key { key, modifiers, .. } = input.events.remove(index) {
                    pressed = Some((key, modifiers));
                }
            }
        });
        if let Some((key, modifiers)) = pressed {
            let chord = actions::Chord::new(
                actions::Mods {
                    command: modifiers.command,
                    alt: modifiers.alt,
                    shift: modifiers.shift,
                },
                key,
            );
            // A refused chord keeps recording; the next chord can replace it.
            let _ = self.record_shortcut(action, &chord.canonical());
        }
    }
    /// Rebuild only when the persisted map changes, including direct test edits.
    pub(in crate::editor) fn sync_keymap(&mut self) {
        if self.frame.keymap_source != self.preferences.keybindings {
            self.frame.keymap = Keymap::resolve(&self.preferences.keybindings);
            self.frame.keybinding_issues = self.frame.keymap.issues().to_vec();
            self.frame.keymap_source = self.preferences.keybindings.clone();
        }
    }
}
