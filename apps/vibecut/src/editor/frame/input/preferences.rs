//! Persists editor preferences.

use crate::editor::{Editor, PreferenceCommand, Result};

impl Editor {
    /// One entry for every user-visible preference edit. App history is
    /// separate from Project history and never changes Project JSON.
    pub fn set_preference(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let before = self.preferences.get(key)?;
        self.preferences.set(key, value)?;
        let after = self.preferences.get(key)?;
        if before != after {
            // An editing mode must not shadow the project edit's next Undo.
            if key != "ripple" {
                self.frame
                    .preference_history
                    .record(PreferenceCommand::new(key, before, after));
            }
            if key == "keybindings" {
                self.sync_keymap();
            }
        }
        Ok(())
    }
    /// Section keys are dynamic; the whole open/closed map is one history entry.
    pub fn set_section_open(&mut self, key: &str, open: bool) -> Result<()> {
        let mut sections = self.preferences.sections.clone();
        sections.insert(key.to_owned(), open);
        self.set_preference("sections", serde_json::to_value(sections)?)
    }
    pub(in crate::editor) fn record_layout_change(&mut self, before: [f32; 3]) {
        let after = self.preferences.panels;
        self.preferences.panels = before;
        self.desk()
            .set_preference("panels", serde_json::json!(after));
    }
}
