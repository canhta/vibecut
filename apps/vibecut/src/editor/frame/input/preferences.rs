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
    /// One guide-line edit (rulers, lines, lock, visibility) as one preference step in
    /// app history; never project JSON.
    pub(in crate::editor) fn edit_guide_lines(
        &mut self,
        edit: impl FnOnce(&crate::editor::GuideLines) -> Result<crate::editor::GuideLines>,
    ) -> Result<()> {
        let next = edit(&self.preferences.preview_lines)?;
        self.set_preference("preview_lines", serde_json::to_value(&next)?)
    }
    /// Star or unstar one catalogue id (`catalog.favorite.set`). Favourites are app
    /// presentation memory: written without an app-history entry, never project JSON.
    pub fn set_favorite(&mut self, family: &str, id: &str, favorite: bool) -> Result<()> {
        let next = if family == "sounds" {
            let sound_id: u64 = id.parse().map_err(|_| crate::Error::Invalid("sound id"))?;
            if favorite {
                // A star takes the metadata from the listed sound, the same as the Sounds tab.
                let sound = self
                    .assets
                    .sounds
                    .as_ref()
                    .and_then(|sounds| sounds.sound(sound_id))
                    .ok_or(crate::Error::SoundNotListed)?;
                self.preferences
                    .catalog_favorites
                    .with_sound(crate::preferences::SoundFavorite::from_result(&sound), true)?
            } else {
                self.preferences.catalog_favorites.without_sound(sound_id)?
            }
        } else {
            self.preferences
                .catalog_favorites
                .with(family, id, favorite)?
        };
        self.preferences
            .set("catalog_favorites", serde_json::to_value(&next)?)?;
        Ok(())
    }
    /// The starred ids of one catalogue family, oldest star first (`catalog.favorite.list`).
    pub fn catalog_favorites(&self, family: &str) -> Vec<String> {
        if family == "sounds" {
            return self
                .preferences
                .catalog_favorites
                .sounds()
                .iter()
                .map(|sound| sound.id.to_string())
                .collect();
        }
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
