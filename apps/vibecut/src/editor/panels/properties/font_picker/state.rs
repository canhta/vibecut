//! Owns the font picker's catalog state, tab/category filtering and inspection.
use crate::egui;
use crate::preferences::{FontPickerPreferences, FontTab};
use engine::control::{FontEntry, FontState};
use engine::jobs::JobId;
use std::collections::BTreeSet;

pub(in crate::editor) struct PendingImport {
    pub snapshot: std::sync::Arc<project::Project>,
    pub selection: BTreeSet<project::ItemId>,
    pub at: time::Tick,
    pub lifetime: u64,
    pub choice: Option<super::FontChoice>,
    pub job: Option<JobId>,
}

/// Which family group All fonts shows within its source tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    All,
    System,
    Google,
    Custom,
}
impl Category {
    pub const ALL: [Self; 4] = [Self::All, Self::System, Self::Google, Self::Custom];
    pub const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::System => "system",
            Self::Google => "google",
            Self::Custom => "custom",
        }
    }
}

/// Listing state of the picker: the previews are loading, ready or refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Loading,
    Ready,
    Failed,
}

/// One in-flight or failed catalog download.
#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub family: String,
    pub job: Option<JobId>,
    pub progress: f32,
    pub cancelling: bool,
    pub failed: bool,
}

/// Transient picker state; the tab and favourites live in preferences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub open: bool,
    pub(crate) lifetime: u64,
    pub(crate) importing: bool,
    pub(crate) import_failed: bool,
    pub anchor: Option<egui::Pos2>,
    pub query: String,
    pub category: Category,
    pub status: Status,
    pub entries: Vec<FontEntry>,
    pub loaded: bool,
    /// A `fonts.list` request has been emitted and no answer is in yet.
    pub request_sent: bool,
    /// The current catalog includes verified imported families.
    pub has_imports: bool,
    pub current: Option<String>,
    pub download: Option<Download>,
    /// Families whose bytes the host installed as preview typefaces.
    pub preview_fonts: BTreeSet<String>,
    /// Families installed this pass; they become usable on the next pass.
    pub preview_pending: BTreeSet<String>,
}
impl State {
    /// The source tab set; My fonts filters verified imported families.
    pub const TABS: [FontTab; 3] = [FontTab::All, FontTab::MyFonts, FontTab::Favorites];

    pub fn open(&mut self, anchor: egui::Pos2, current: Option<String>) {
        self.lifetime = self.lifetime.wrapping_add(1);
        self.open = true;
        self.anchor = Some(anchor);
        self.query.clear();
        self.category = Category::All;
        self.current = current;
        if !self.loaded {
            self.status = Status::Loading;
            self.request_sent = false;
        }
    }
    pub fn close(&mut self) {
        self.lifetime = self.lifetime.wrapping_add(1);
        self.open = false;
    }
    pub fn tabs(&self) -> [FontTab; 3] {
        Self::TABS
    }
    pub fn tab(&self, preferences: &FontPickerPreferences) -> FontTab {
        preferences.tab
    }
    /// Entries the active tab, category and search show, in catalog order.
    pub fn visible(&self, preferences: &FontPickerPreferences) -> Vec<&FontEntry> {
        let query = self.query.trim().to_lowercase();
        let tab = self.tab(preferences);
        self.entries
            .iter()
            .filter(|entry| match tab {
                FontTab::All => match self.category {
                    Category::All => true,
                    Category::System => entry.origin == engine::control::FontOrigin::System,
                    Category::Google => entry.origin == engine::control::FontOrigin::Google,
                    Category::Custom => entry.origin == engine::control::FontOrigin::Custom,
                },
                FontTab::Favorites => preferences
                    .favorites
                    .iter()
                    .any(|family| family == &entry.family),
                FontTab::MyFonts => entry.origin == engine::control::FontOrigin::Custom,
            })
            .filter(|entry| query.is_empty() || entry.family.to_lowercase().contains(&query))
            .collect()
    }
    /// Next available family after `family` in the visible order, then before it.
    pub fn next_available(
        &self,
        preferences: &FontPickerPreferences,
        family: &str,
    ) -> Option<String> {
        let visible = self.visible(preferences);
        let at = visible.iter().position(|entry| entry.family == family)?;
        visible
            .iter()
            .cycle()
            .skip(at + 1)
            .take(visible.len().saturating_sub(1))
            .find(|entry| entry.state == FontState::Available)
            .map(|entry| entry.family.clone())
    }
    pub fn inspect(&self, preferences: &FontPickerPreferences) -> Inspection {
        Inspection {
            open: self.open,
            tab: self.tab(preferences),
            category: self.category,
            query: self.query.clone(),
            status: self.status,
            entries: self.entries.clone(),
            favorites: preferences.favorites.clone(),
            download: self.download.clone(),
            has_imports: self.has_imports,
            preview_fonts: self.preview_fonts.iter().cloned().collect(),
        }
    }
}

/// Snapshot for tests and agents; it never carries file paths or bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    pub open: bool,
    pub tab: FontTab,
    pub category: Category,
    pub query: String,
    pub status: Status,
    pub entries: Vec<FontEntry>,
    pub favorites: Vec<String>,
    pub download: Option<Download>,
    pub has_imports: bool,
    /// Installed preview typefaces, as evidence that rows use their own face.
    pub preview_fonts: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{Category, State, Status};
    use crate::preferences::{FontPickerPreferences, FontTab};
    use engine::control::{FontEntry, FontOrigin, FontState};

    fn entry(family: &str, origin: FontOrigin, state: FontState) -> FontEntry {
        FontEntry {
            family: family.into(),
            origin,
            styles: vec!["Regular".into()],
            state,
            license: None,
            source: None,
            download_bytes: None,
        }
    }
    fn state() -> State {
        State {
            open: true,
            status: Status::Ready,
            entries: vec![
                entry("Inter", FontOrigin::Bundled, FontState::Available),
                entry("Arial", FontOrigin::System, FontState::Available),
                entry("Lobster", FontOrigin::Google, FontState::NotDownloaded),
            ],
            loaded: true,
            ..State::default()
        }
    }

    #[test]
    fn tabs_mirror_the_source_set_with_my_fonts_always_present() {
        let state = state();
        assert_eq!(
            state.tabs(),
            [FontTab::All, FontTab::MyFonts, FontTab::Favorites]
        );
        let preferences = FontPickerPreferences {
            tab: FontTab::MyFonts,
            ..FontPickerPreferences::default()
        };
        assert_eq!(state.tab(&preferences), FontTab::MyFonts);
        assert!(
            state.visible(&preferences).is_empty(),
            "no imports exist yet"
        );
        let mut imported = state;
        imported.has_imports = true;
        assert_eq!(imported.tabs().len(), 3);
    }

    #[test]
    fn search_category_and_favorites_filter_the_visible_catalog() {
        let state = state();
        let mut preferences = FontPickerPreferences::default();
        assert_eq!(state.visible(&preferences).len(), 3);
        let mut search = state.clone();
        search.query = "LO".into();
        let visible = search.visible(&preferences);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].family, "Lobster");
        let mut system = state.clone();
        system.category = Category::System;
        assert_eq!(system.visible(&preferences)[0].family, "Arial");
        let mut google = state.clone();
        google.category = Category::Google;
        assert_eq!(google.visible(&preferences)[0].family, "Lobster");
        let mut custom = state.clone();
        custom.category = Category::Custom;
        assert!(custom.visible(&preferences).is_empty());
        preferences.favorites = vec!["Inter".into(), "Lobster".into()];
        preferences.tab = FontTab::Favorites;
        let favorites: Vec<_> = state
            .visible(&preferences)
            .into_iter()
            .map(|entry| entry.family.as_str())
            .collect();
        assert_eq!(favorites, ["Inter", "Lobster"]);
    }

    #[test]
    fn a_failed_preview_falls_back_to_the_next_available_family() {
        let state = state();
        let preferences = FontPickerPreferences::default();
        assert_eq!(
            state.next_available(&preferences, "Arial").as_deref(),
            Some("Inter"),
            "the search wraps past a not-downloaded family"
        );
        let mut catalog = state;
        catalog.entries[1].state = FontState::NotDownloaded;
        catalog.entries[2].state = FontState::NotDownloaded;
        assert_eq!(
            catalog.next_available(&preferences, "Arial").as_deref(),
            Some("Inter"),
            "a not-downloaded neighbour is skipped"
        );
    }
}
