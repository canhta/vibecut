//! Owns the font picker's catalog state, chip filter, sections and inspection.
use crate::egui;
use crate::preferences::FontPickerPreferences;
use engine::control::{FontEntry, FontState};
use engine::jobs::JobId;
use std::collections::BTreeSet;

pub(in crate::editor) struct PendingImport {
    pub snapshot: std::sync::Arc<project::Project>,
    pub selection: BTreeSet<project::ItemId>,
    pub at: time::Tick,
    pub lifetime: u64,
    pub job: Option<JobId>,
}

/// The reference's filter chips: every family, or only those with a recorded licence
/// (bundled, catalogue and imported families; system fonts carry none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Commercial,
}
impl Filter {
    pub const ALL: [Self; 2] = [Self::All, Self::Commercial];
    pub const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Commercial => "commercial",
        }
    }
    fn admits(self, entry: &FontEntry) -> bool {
        match self {
            Self::All => true,
            Self::Commercial => entry.license.is_some(),
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

/// Transient picker state; the favourites live in preferences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub open: bool,
    pub(crate) lifetime: u64,
    pub(crate) importing: bool,
    pub(crate) import_failed: bool,
    pub anchor: Option<egui::Pos2>,
    pub query: String,
    pub filter: Filter,
    /// Whether the Presets section (bundled, catalogue and imported families) is expanded.
    pub presets_collapsed: bool,
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
    pub fn open(&mut self, anchor: egui::Pos2, current: Option<String>) {
        self.lifetime = self.lifetime.wrapping_add(1);
        self.open = true;
        self.anchor = Some(anchor);
        self.query.clear();
        self.filter = Filter::All;
        self.presets_collapsed = false;
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
    /// The families the chip and search show: the Presets section (every origin but the
    /// operating system, catalogue order) and the System font section. Favourites come first
    /// within a section.
    pub fn sections(
        &self,
        preferences: &FontPickerPreferences,
    ) -> (Vec<&FontEntry>, Vec<&FontEntry>) {
        let query = self.query.trim().to_lowercase();
        let (mut presets, mut system): (Vec<&FontEntry>, Vec<&FontEntry>) = self
            .entries
            .iter()
            .filter(|entry| self.filter.admits(entry))
            .filter(|entry| query.is_empty() || entry.family.to_lowercase().contains(&query))
            .partition(|entry| entry.origin != engine::control::FontOrigin::System);
        for section in [&mut presets, &mut system] {
            section.sort_by_key(|entry| {
                !preferences
                    .favorites
                    .iter()
                    .any(|family| family == &entry.family)
            });
        }
        (presets, system)
    }
    /// Every shown family in section order, collapsed or not.
    pub fn visible(&self, preferences: &FontPickerPreferences) -> Vec<&FontEntry> {
        let (mut presets, system) = self.sections(preferences);
        presets.extend(system);
        presets
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
            filter: self.filter,
            presets_collapsed: self.presets_collapsed,
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
    pub filter: Filter,
    pub presets_collapsed: bool,
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
    use super::{Filter, State, Status};
    use crate::preferences::FontPickerPreferences;
    use engine::control::{FontEntry, FontOrigin, FontState};

    fn entry(family: &str, origin: FontOrigin, state: FontState) -> FontEntry {
        FontEntry {
            family: family.into(),
            origin,
            styles: vec!["Regular".into()],
            state,
            license: (origin != FontOrigin::System).then(|| "OFL-1.1".into()),
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
    fn sections_split_presets_from_system_fonts_and_float_favorites() {
        let state = state();
        let mut preferences = FontPickerPreferences::default();
        let (presets, system) = state.sections(&preferences);
        let names = |list: &[&engine::control::FontEntry]| {
            list.iter()
                .map(|entry| entry.family.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&presets), ["Inter", "Lobster"]);
        assert_eq!(names(&system), ["Arial"]);
        preferences.favorites = vec!["Lobster".into()];
        let (presets, _) = state.sections(&preferences);
        assert_eq!(
            names(&presets),
            ["Lobster", "Inter"],
            "a favourite sorts first"
        );
    }

    #[test]
    fn search_and_the_commercial_chip_filter_the_visible_catalog() {
        let state = state();
        let preferences = FontPickerPreferences::default();
        assert_eq!(state.visible(&preferences).len(), 3);
        let mut search = state.clone();
        search.query = "LO".into();
        let visible = search.visible(&preferences);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].family, "Lobster");
        let mut commercial = state.clone();
        commercial.filter = Filter::Commercial;
        let (presets, system) = commercial.sections(&preferences);
        assert_eq!(presets.len(), 2);
        assert!(system.is_empty(), "system fonts carry no licence record");
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
