//! Persists editor preferences.

use crate::editor::{Editor, PreferenceCommand, Result};

impl Editor {
    /// One entry per preference edit except editing mode/surface state. App history is
    /// separate from Project history and never changes Project JSON.
    pub fn set_preference(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let before = self.preferences.get(key)?;
        self.preferences.set(key, value)?;
        let after = self.preferences.get(key)?;
        if before != after {
            // An editing mode must not shadow the project edit's next Undo.
            if !matches!(key, "ripple" | "timeline_lanes") {
                self.frame
                    .preference_history
                    .record(PreferenceCommand::new(key, before, after));
            }
            if key == "keybindings" {
                self.sync_keymap();
            }
            if key == "ripple" {
                self.sync_gap_policy();
            }
        }
        Ok(())
    }
    /// Star or unstar one catalogue id (`catalog.favorite.set`). Favourites are app
    /// presentation memory: written without an app-history entry, never project JSON.
    pub fn set_favorite(&mut self, family: &str, id: &str, favorite: bool) -> Result<()> {
        let next = self
            .preferences
            .catalog_favorites
            .with(family, id, favorite)?;
        self.preferences
            .set("catalog_favorites", serde_json::to_value(&next)?)?;
        Ok(())
    }
    /// The starred ids of one catalogue family, oldest star first (`catalog.favorite.list`).
    pub fn catalog_favorites(&self, family: &str) -> Vec<String> {
        self.preferences.catalog_favorites.list(family).to_vec()
    }
    /// Hand the ripple preference to the session, which alone decides how an edit
    /// treats the gap it leaves. A gesture in flight keeps its policy; the next
    /// frame retries.
    pub(in crate::editor) fn sync_gap_policy(&mut self) {
        let enabled = self.preferences.ripple;
        if self.session.ripple() == enabled || self.session.transaction_active() {
            return;
        }
        if let Err(error) = self
            .session
            .apply(engine::Command::TimelineSetRipple { enabled })
        {
            self.fail(error.to_string());
        }
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
        if let Err(error) = self.set_preference("panels", serde_json::json!(after)) {
            self.fail(error.to_string());
        }
    }
}
