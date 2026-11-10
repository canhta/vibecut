//! Maintains editor shortcut bindings.

use crate::editor::{
    Editor, Error, Keymap, PREFERENCE_COMMAND, Result, ShortcutTab, Text, actions, egui,
};
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
        keybindings: actions::KeybindingPreferences,
    ) -> Result<()> {
        self.dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "keybindings", "value": keybindings}),
        )
    }
    /// Open the shortcuts dialog. Rows read the live effective bindings, so a
    /// command or undo elsewhere is reflected on the next frame.
    pub fn open_shortcuts(&mut self) {
        self.frame.open_shortcuts();
    }
    pub(in crate::editor) fn close_shortcuts(&mut self) {
        self.frame.close_shortcuts();
    }
    pub fn shortcut_tab(&self) -> ShortcutTab {
        self.frame.shortcut_tab
    }
    pub fn set_shortcut_tab(&mut self, tab: ShortcutTab) {
        self.frame.set_shortcut_tab(tab);
    }
    /// Record one chord for an action. The dialog calls this after a key press;
    /// tests also use it for chord text no egui key event can produce.
    pub fn record_shortcut(&mut self, action: &str, chord: &str) -> Result<()> {
        match self.dispatch(
            actions::KEYBINDINGS_SET_COMMAND,
            serde_json::json!({"action": action, "bindings": [chord]}),
        ) {
            Ok(()) => {
                self.frame.shortcut_error = None;
                self.frame.shortcut_recording = None;
                Ok(())
            }
            Err(error) => {
                self.frame.shortcut_error = Some(self.shortcut_error_message(&error));
                Err(error)
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
    /// Read the first key event while a row records. Escape cancels recording
    /// without closing the dialog; the returned bool tells the dialog that this
    /// frame's Escape was consumed by the recording gesture.
    pub(in crate::editor) fn capture_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let Some(action) = self.frame.shortcut_recording else {
            return false;
        };
        let mut pressed = None;
        let mut cancel = false;
        let mut used = None;
        ctx.input_mut(|input| {
            for (index, event) in input.events.iter().enumerate() {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                {
                    if *key == egui::Key::Escape && modifiers.is_none() {
                        cancel = true;
                    } else {
                        pressed = Some((*key, *modifiers));
                    }
                    used = Some(index);
                    break;
                }
            }
            if let Some(index) = used {
                input.events.remove(index);
            }
        });
        if cancel {
            self.frame.shortcut_recording = None;
            self.frame.shortcut_error = None;
            return true;
        }
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
        false
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
