//! Bounded preference file IO and the startup repair warning.

use super::{
    DEFAULT_PANELS, EasingPreferences, FontPickerPreferences, MAX_PREFERENCES_BYTES,
    MAX_TIMELINE_LANES, PREFERENCES_VERSION, Preferences, validate_panels,
};
use crate::{Error, Result};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// Which part of a damaged preference document was replaced by defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferenceIssue {
    Read,
    Layout,
    Keybindings,
}
impl std::fmt::Display for PreferenceIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Read => "preferences could not be read",
            Self::Layout => "layout preferences were reset",
            Self::Keybindings => "invalid keybindings were removed",
        })
    }
}

pub struct PreferenceStore {
    path: PathBuf,
}
impl PreferenceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Startup read: a damaged document is replaced wholesale, while a damaged
    /// layout keeps unrelated preferences and is reported through the warning.
    pub fn load_with_warning(&self) -> (Preferences, Option<PreferenceIssue>) {
        let mut value = match self.read() {
            Ok(value) => value,
            Err(Error::Unsupported("preferences version")) => return (Preferences::system(), None),
            Err(_) => return (Preferences::system(), Some(PreferenceIssue::Read)),
        };
        let mut issue = None;
        if value.windows.len() > 64
            || value
                .windows
                .iter()
                .any(|(key, geometry)| key.len() > 64 || !geometry.valid())
        {
            value.windows.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if validate_panels(value.panels).is_err() {
            value.panels = DEFAULT_PANELS;
            issue = Some(PreferenceIssue::Layout);
        }
        if value.validate_sections().is_err() {
            value.sections.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.validate_properties_tabs().is_err() {
            value.properties_tabs.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.keybindings.validate().is_err() {
            let (sanitized, _) = std::mem::take(&mut value.keybindings).sanitize();
            value.keybindings = sanitized;
            issue = Some(PreferenceIssue::Keybindings);
        }
        if value.timeline_lane_count() > MAX_TIMELINE_LANES {
            value.timeline_lanes.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.font_picker.validate().is_err() {
            value.font_picker = FontPickerPreferences::default();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.easing.validate().is_err() {
            value.easing = EasingPreferences::default();
            issue = Some(PreferenceIssue::Layout);
        }
        (value, issue)
    }
    fn read(&self) -> Result<Preferences> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::system());
            }
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > MAX_PREFERENCES_BYTES {
            return Err(Error::Invalid("preference size"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_PREFERENCES_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(Error::Invalid("preference size"));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value.get("version").and_then(serde_json::Value::as_u64)
            != Some(u64::from(PREFERENCES_VERSION))
        {
            return Err(Error::Unsupported("preferences version"));
        }
        let mut preferences: Preferences = serde_json::from_value(value)?;
        preferences.ai_models.validate()?;
        preferences.resolve_models();
        Ok(preferences)
    }
    pub fn save(&self, value: &Preferences) -> Result<()> {
        value.validate()?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        project::atomic_write(&self.path, &serde_json::to_vec(value)?)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::editor::{AssetsTab, Guide};
    use crate::i18n::Locale;
    use crate::preferences::{EasingTab, SavedEasing};
    use crate::preferences::{
        MEDIA_MIN_PX, PLAYER_MIN_PX, PREFERENCE_KEYS, PROPERTIES_MIN_PX, TIMELINE_MAX,
        TIMELINE_MIN, WindowGeometry, column_widths, drag_panel,
    };
    use std::collections::BTreeSet;

    /// Column widths in px at `columns` of room, rounded for exact comparison.
    fn px(panels: [f32; 3], columns: f32) -> [f32; 3] {
        column_widths(panels, columns).map(f32::round)
    }

    /// Widths after a drag of `delta` (a fraction of the columns).
    fn dragged(panels: [f32; 3], axis: usize, delta: f32, columns: f32) -> [f32; 3] {
        px(drag_panel(panels, axis, delta, columns), columns)
    }

    /// A complete version-8 document as the app writes it, including the font
    /// picker section and the preview axis toggle.
    #[test]
    fn default_is_deterministic_and_system_preferences_follow_the_os() {
        assert_eq!(Preferences::default().locale, Locale::En);
        assert_eq!(Preferences::default().panels, DEFAULT_PANELS);
        assert!(Preferences::default().snapping);
        assert!(!Preferences::default().ripple);
        assert!(Preferences::default().preview_axis);
        assert!(!Preferences::default().playback_loop);
        assert_eq!(Preferences::default().preview_guide, Guide::default());
        assert!(Preferences::default().preview_overlays.notes);
        assert!(!Preferences::default().preview_overlays.guide);
        let lines = &Preferences::default().preview_lines;
        assert!(!lines.rulers && lines.shown && !lines.locked && lines.is_empty());
        assert_eq!(Preferences::system().locale, Locale::from_system());
    }

    #[test]
    fn splitter_drags_clamp_each_region_to_its_documented_limits() {
        let start = DEFAULT_PANELS;
        assert!(validate_panels(start).is_ok());
        assert_eq!(crate::preferences::WINDOW_MIN, [1268.0, 650.0]);
        // Measured pixel rows at 1840 x 1142: 1809 pt columns, 1091 pt timeline row.
        assert_eq!((start[0] * 1809.0).round(), 398.0);
        assert_eq!((start[1] * 1809.0).round(), 379.0);
        assert_eq!((start[2] * 1091.0).round(), 498.0);
        // Column minimums in px; first open 398 / 1032 / 379 at 1809 pt of columns.
        assert_eq!(
            [MEDIA_MIN_PX, PLAYER_MIN_PX, PROPERTIES_MIN_PX],
            [398.0, 398.0, 368.0]
        );
        assert_eq!(px(start, 1809.0), [398.0, 1032.0, 379.0]);
        // Resize rule: proportional, never below the minimums. At the minimum window
        // the stored fractions would give 272 / 259 pt; the minimums hold instead.
        assert_eq!(px(start, 1236.0), [398.0, 470.0, 368.0]);
        let reset_small = [398.0 / 1236.0, 368.0 / 1236.0, start[2]];
        assert_eq!(px(reset_small, 1236.0), [398.0, 470.0, 368.0]);
        assert_eq!(px(reset_small, 1809.0), [583.0, 688.0, 539.0]);
        let narrow = column_widths(start, 1000.0);
        assert!((narrow.iter().sum::<f32>() - 1000.0).abs() < 0.01);
        // Drag clamps: media max leaves the player and properties at theirs.
        assert_eq!(dragged(start, 0, 10.0, 1809.0), [1032.0, 398.0, 379.0]);
        assert_eq!(dragged(start, 0, -10.0, 1809.0), [398.0, 1032.0, 379.0]);
        assert_eq!(dragged(start, 1, -10.0, 1809.0), [398.0, 398.0, 1013.0]);
        assert_eq!(dragged(start, 1, 10.0, 1809.0), [398.0, 1043.0, 368.0]);
        // At the minimum window the divider has no room: media 398, player 470, properties 368.
        assert_eq!(
            dragged(reset_small, 0, -10.0, 1236.0),
            [398.0, 470.0, 368.0]
        );
        assert_eq!(dragged(reset_small, 0, 10.0, 1236.0), [470.0, 398.0, 368.0]);
        assert_eq!(
            dragged(reset_small, 1, -10.0, 1236.0),
            [398.0, 398.0, 440.0]
        );
        // Below the three minimums a column drag does nothing.
        assert_eq!(drag_panel(start, 0, 0.1, 1000.0), start);
        let timeline_one_row = drag_panel(start, 2, 10.0, 1809.0);
        assert_eq!(timeline_one_row[2], TIMELINE_MIN);
        assert_eq!((TIMELINE_MIN * 1091.0).round(), 204.0);
        let timeline_full = drag_panel(start, 2, -10.0, 1809.0);
        assert_eq!(timeline_full[2], TIMELINE_MAX);
        assert_eq!((TIMELINE_MAX * 1091.0).round(), 835.0);
        assert_eq!(drag_panel(start, 7, 0.1, 1809.0), start);
        assert_eq!(drag_panel(start, 0, f32::NAN, 1809.0), start);
        assert!(validate_panels([0.60, 0.40, 0.50]).is_err());
        assert!(validate_panels([0.0, 0.25, 0.50]).is_err());
        assert!(validate_panels([f32::NAN, 0.25, 0.50]).is_err());
        assert!(validate_panels([0.25, 0.25, 0.80]).is_err());
    }

    #[test]
    fn every_registered_preference_key_roundtrips_through_the_store() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut preferences = Preferences::default();
        for (key, value) in [
            ("locale", serde_json::json!("vi")),
            ("dark", serde_json::json!(false)),
            ("snapping", serde_json::json!(false)),
            ("ripple", serde_json::json!(true)),
            ("preview_axis", serde_json::json!(false)),
            ("playback_loop", serde_json::json!(true)),
            ("default_fps", serde_json::json!(60)),
            ("hardware_acceleration", serde_json::json!("off")),
            ("cache_limit_bytes", serde_json::json!(1u64 << 30)),
            ("panels", serde_json::json!([0.30, 0.20, 0.35])),
            ("sections", serde_json::json!({"inspector.canvas": false})),
            ("assets_tab", serde_json::json!("captions")),
            ("properties_tabs", serde_json::json!({"text": "effects"})),
            ("media_view", serde_json::json!("list")),
            (
                "media_sort",
                serde_json::json!({"key": "size", "order": "desc"}),
            ),
            (
                "projects",
                serde_json::json!({"view": "list", "sort": "duration", "ascending": true}),
            ),
            (
                "keybindings",
                serde_json::json!({
                    "active": 1,
                    "profiles": [
                        {"bindings": {}, "isCustomized": []},
                        {"bindings": {"timeline.snap": ["Ctrl+Shift+N"]}, "isCustomized": ["timeline.snap"]},
                        {"bindings": {}, "isCustomized": []},
                        {"bindings": {}, "isCustomized": []},
                        {"bindings": {}, "isCustomized": []}
                    ]
                }),
            ),
            (
                "sticker_recents",
                serde_json::json!(["flags:VN", "graphic.insert.star"]),
            ),
            (
                "timeline_lanes",
                serde_json::json!({"12": [3, 4], "13": [7]}),
            ),
            (
                "font_picker",
                serde_json::json!({"favorites": ["Anton", "Inter"]}),
            ),
            (
                "catalog_favorites",
                serde_json::json!({
                    "ids": {"effects": ["blur", "glow"]},
                    "sounds": [{
                        "id": 12,
                        "title": "Whoosh",
                        "duration_ms": 1000,
                        "preview_url": "https://cdn.freesound.org/previews/1/1-hq.mp3"
                    }]
                }),
            ),
            (
                "easing",
                serde_json::json!({
                    "tab": "saved",
                    "saved": [{"name": "Custom 1", "curve": [0.1, 0.2, 0.3, 1.4]}]
                }),
            ),
            (
                "preview_guide",
                serde_json::json!({"kind": "grid", "rows": 24, "cols": 1}),
            ),
            (
                "preview_guide",
                serde_json::json!({"kind": "custom", "top": 10, "bottom": 20, "left": 5, "right": 15}),
            ),
            (
                "preview_overlays",
                serde_json::json!({"guide": true, "notes": false}),
            ),
            ("track_height", serde_json::json!("tall")),
            ("waveform_size", serde_json::json!(100)),
            (
                "preview_lines",
                serde_json::json!({
                    "rulers": true,
                    "shown": false,
                    "locked": true,
                    "vertical": [0, -250],
                    "horizontal": [1080]
                }),
            ),
            (
                "export",
                serde_json::json!({
                    "format": "webm",
                    "quality": "very_high",
                    "include_audio": false,
                    "output": {
                        "audio_only": false,
                        "bit_rate": null,
                        "codec": "automatic",
                        "fps": null,
                        "gif": {"dither": "bayer", "max_colors": 256},
                        "profile": "automatic",
                        "range": {"kind": "full"},
                        "resolution": null,
                        "scene": {"kind": "active"},
                        "version": 1
                    }
                }),
            ),
        ] {
            assert!(PREFERENCE_KEYS.contains(&key), "unregistered key {key}");
            preferences.set(key, value).expect("set preference");
        }
        store.save(&preferences).expect("save preferences");
        let loaded = store.load_with_warning().0;
        assert_eq!(loaded.version, PREFERENCES_VERSION);
        for key in PREFERENCE_KEYS {
            assert_eq!(
                loaded.get(key).expect("get preference"),
                preferences.get(key).expect("get preference"),
                "{key} must round-trip"
            );
        }
        // Lane memory is keyed by project and pruned by the caller's existence test; one project's set never overwrites another's.
        let mut pruned = loaded.clone();
        pruned.retain_timeline_lanes(|project| project == project::ProjectId(12));
        assert_eq!(
            pruned.timeline_lanes.get(&project::ProjectId(12)).cloned(),
            Some(BTreeSet::from([3, 4]))
        );
        assert!(!pruned.timeline_lanes.contains_key(&project::ProjectId(13)));
        // Save As carries the issuing project's set to the fresh id; the source
        // keeps its own entry, a project with no open lanes carries nothing,
        // and the same-id case is a no-op.
        let mut carried = loaded.clone();
        carried.copy_timeline_lanes(project::ProjectId(12), project::ProjectId(14));
        assert_eq!(
            carried.timeline_lanes.get(&project::ProjectId(12)).cloned(),
            Some(BTreeSet::from([3, 4])),
            "the issuing project keeps its own lanes"
        );
        assert_eq!(
            carried.timeline_lanes.get(&project::ProjectId(14)).cloned(),
            Some(BTreeSet::from([3, 4])),
            "the copy opens with the issuing project's lanes"
        );
        let mut missing = loaded.clone();
        missing.copy_timeline_lanes(project::ProjectId(99), project::ProjectId(14));
        assert!(!missing.timeline_lanes.contains_key(&project::ProjectId(14)));
        let mut same = loaded.clone();
        same.copy_timeline_lanes(project::ProjectId(12), project::ProjectId(12));
        assert_eq!(same.timeline_lanes, loaded.timeline_lanes);
        // Hostile sizes cannot push the document past its lane bound: the copy
        // carries only what fits and the source still keeps its set.
        let mut over = Preferences::default();
        over.timeline_lanes.insert(
            project::ProjectId(12),
            (0..MAX_TIMELINE_LANES as u64 - 1).collect(),
        );
        over.timeline_lanes
            .insert(project::ProjectId(13), BTreeSet::from([4096]));
        over.copy_timeline_lanes(project::ProjectId(12), project::ProjectId(14));
        assert_eq!(
            over.timeline_lanes.get(&project::ProjectId(12)).cloned(),
            Some((0..MAX_TIMELINE_LANES as u64 - 1).collect()),
            "an already-full document still keeps the issuing project's set"
        );
        assert!(
            !over.timeline_lanes.contains_key(&project::ProjectId(14)),
            "no room under the bound means nothing is carried"
        );
        assert!(over.timeline_lane_count() <= MAX_TIMELINE_LANES);
        let mut partial = Preferences::default();
        partial
            .timeline_lanes
            .insert(project::ProjectId(12), (0..3000).collect());
        partial
            .timeline_lanes
            .insert(project::ProjectId(13), (0..100).collect());
        partial.copy_timeline_lanes(project::ProjectId(12), project::ProjectId(14));
        assert_eq!(
            partial.timeline_lanes.get(&project::ProjectId(14)).cloned(),
            Some((0..996).collect()),
            "an oversized copy is capped at the remaining document budget"
        );
        assert_eq!(
            partial
                .timeline_lanes
                .get(&project::ProjectId(12))
                .map(BTreeSet::len),
            Some(3000),
            "capping the copy never trims the issuing project's set"
        );
        assert_eq!(partial.timeline_lane_count(), MAX_TIMELINE_LANES);
        assert!(Preferences::default().get("unknown").is_err());
    }

    #[test]
    fn corrupt_layout_falls_back_to_defaults_with_a_warning() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut document = serde_json::to_value(Preferences::default()).expect("encode");
        document["locale"] = "vi".into();
        document["dark"] = false.into();
        document["panels"] = serde_json::json!([0.9, 0.9, 0.9]);
        std::fs::write(store.path(), document.to_string()).expect("write preferences");
        let (preferences, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Layout));
        assert_eq!(preferences.panels, DEFAULT_PANELS);
        assert_eq!(preferences.locale, Locale::Vi);
        assert!(!preferences.dark);
        std::fs::write(store.path(), b"{").expect("write corrupt preferences");
        let (preferences, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Read));
        assert_eq!(preferences, Preferences::system());
        std::fs::write(
            store.path(),
            br#"{"version":9,"locale":"en","dark":true,"panels":[0.25,0.25,0.5]}"#,
        )
        .expect("write newer preferences");
        assert_eq!(store.load_with_warning(), (Preferences::system(), None));
    }

    #[test]
    fn other_preference_version_is_dropped_and_overwritten() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        for document in [
            br#"{"version":1,"locale":"vi","dark":false,"panels":[0.25,0.25,0.5]}"#.as_slice(),
            br#"{"version":4,"locale":"en","dark":true,"panels":[0.25,0.25,0.5],"export":{"format":"mp4","quality":"high","include_audio":true}}"#.as_slice(),
            br#"{"version":5,"locale":"en","dark":true,"panels":[0.25,0.25,0.5],"export":{"format":"mp4","quality":"high","include_audio":true}}"#.as_slice(),
            br#"{"locale":"en","dark":true,"panels":[0.25,0.25,0.4]}"#.as_slice(),
        ] {
            std::fs::write(store.path(), document).expect("write old preferences");
            assert_eq!(store.load_with_warning(), (Preferences::system(), None));
            store.save(&Preferences::default()).expect("overwrite old preferences");
            assert_eq!(store.load_with_warning().0, Preferences::default());
        }
    }

    #[test]
    fn media_preferences_refuse_unknown_values_atomically() {
        let mut preferences = Preferences::default();
        let before = preferences.clone();
        let oversized_lanes = (0..=4096).collect::<Vec<_>>();
        for (key, value) in [
            ("media_view", serde_json::json!("columns")),
            (
                "media_sort",
                serde_json::json!({"key": "bogus", "order": "asc"}),
            ),
            (
                "media_sort",
                serde_json::json!({"key": "name", "order": "bogus"}),
            ),
            ("timeline_lanes", serde_json::json!({"12": oversized_lanes})),
            ("default_fps", serde_json::json!(48)),
            ("default_fps", serde_json::json!("30")),
            ("hardware_acceleration", serde_json::json!("maybe")),
            ("cache_limit_bytes", serde_json::json!((256u64 << 20) - 1)),
            ("cache_limit_bytes", serde_json::json!((8u64 << 30) + 1)),
            ("track_height", serde_json::json!("huge")),
            ("waveform_size", serde_json::json!(45)),
            ("waveform_size", serde_json::json!("60")),
        ] {
            assert!(preferences.set(key, value).is_err(), "{key}");
            assert_eq!(preferences, before, "{key} must fail atomically");
        }
    }

    #[test]
    fn guide_preferences_refuse_out_of_range_and_unknown_values_atomically() {
        let mut preferences = Preferences::default();
        let before = preferences.clone();
        for (key, value) in [
            (
                "preview_guide",
                serde_json::json!({"kind": "grid", "rows": 0, "cols": 3}),
            ),
            (
                "preview_guide",
                serde_json::json!({"kind": "grid", "rows": 3, "cols": 25}),
            ),
            ("preview_guide", serde_json::json!({"kind": "custom"})),
            (
                "preview_guide",
                serde_json::json!({"kind": "custom", "top": 50, "bottom": 50, "left": 0, "right": 0}),
            ),
            (
                "preview_guide",
                serde_json::json!({"kind": "custom", "top": 0, "bottom": 0, "left": 60, "right": 40}),
            ),
            (
                "preview_guide",
                serde_json::json!({"kind": "custom", "top": -5, "bottom": 0, "left": 0, "right": 0}),
            ),
            (
                "preview_guide",
                serde_json::json!({"kind": "custom", "top": 100, "bottom": 0, "left": 0, "right": 0}),
            ),
            ("preview_guide", serde_json::json!({"kind": "spotlight"})),
            ("preview_overlays", serde_json::json!({"guide": true})),
            (
                "preview_lines",
                serde_json::json!({"rulers": true, "shown": true, "locked": false, "vertical": []}),
            ),
            (
                "preview_lines",
                serde_json::json!({
                    "rulers": true, "shown": true, "locked": false,
                    "vertical": [100001], "horizontal": []
                }),
            ),
            (
                "preview_lines",
                serde_json::json!({
                    "rulers": true, "shown": true, "locked": false,
                    "vertical": [], "horizontal": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
                }),
            ),
            (
                "preview_lines",
                serde_json::json!({
                    "rulers": true, "shown": true, "locked": false,
                    "vertical": [], "horizontal": [], "snap": true
                }),
            ),
            (
                "preview_overlays",
                serde_json::json!({"guide": true, "notes": true, "hud": false}),
            ),
        ] {
            assert!(preferences.set(key, value).is_err(), "{key} must refuse");
            assert_eq!(preferences, before);
        }
    }

    #[test]
    fn tab_preferences_validate_kind_and_tab_ids_atomically() {
        let mut preferences = Preferences::default();
        preferences
            .set(
                "properties_tabs",
                serde_json::json!({
                    "text": "effects",
                    "video": "audio",
                    "graphic": "graphic"
                }),
            )
            .expect("valid tab memory");
        assert_eq!(
            preferences.properties_tabs.get("text").map(String::as_str),
            Some("effects")
        );
        for value in [
            serde_json::json!({"unknown": "transform"}),
            serde_json::json!({"text": "masks"}),
            serde_json::json!({"text": "unknown"}),
        ] {
            let before = preferences.clone();
            assert!(preferences.set("properties_tabs", value).is_err());
            assert_eq!(preferences, before);
        }
        let mut unknown = Preferences::default();
        unknown
            .properties_tabs
            .insert("text".into(), "masks".into());
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        std::fs::write(
            store.path(),
            serde_json::to_vec(&Preferences {
                properties_tabs: unknown.properties_tabs,
                ..Default::default()
            })
            .expect("encode preferences"),
        )
        .expect("write preferences");
        let (loaded, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Layout));
        assert!(loaded.properties_tabs.is_empty());
        assert_eq!(loaded.assets_tab, AssetsTab::Media);
    }

    #[test]
    fn font_picker_preferences_validate_favorite_families_atomically() {
        let mut preferences = Preferences::default();
        preferences
            .set(
                "font_picker",
                serde_json::json!({"favorites": ["Anton", "Inter"]}),
            )
            .expect("valid font preferences");
        assert_eq!(preferences.font_picker.favorites, ["Anton", "Inter"]);
        for value in [
            serde_json::json!({"tab": "all", "favorites": []}),
            serde_json::json!({"favorites": ["Inter", "Inter"]}),
            serde_json::json!({"favorites": [""]}),
            serde_json::json!({"favorites": ["x".repeat(129)]}),
            serde_json::json!({"favorites": vec!["Inter"; 257]}),
            serde_json::json!({"favorites": [], "extra": true}),
        ] {
            let before = preferences.clone();
            assert!(preferences.set("font_picker", value).is_err());
            assert_eq!(preferences, before);
        }
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        std::fs::write(
            store.path(),
            serde_json::to_vec(&Preferences {
                font_picker: FontPickerPreferences {
                    favorites: vec![String::new()],
                },
                ..Default::default()
            })
            .expect("encode preferences"),
        )
        .expect("write preferences");
        let (loaded, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Layout));
        assert_eq!(loaded.font_picker, FontPickerPreferences::default());
    }

    #[test]
    fn easing_preferences_validate_saved_curves_atomically_and_repair_on_load() {
        let mut preferences = Preferences::default();
        preferences
            .set(
                "easing",
                serde_json::json!({
                    "tab": "saved",
                    "saved": [
                        {"name": "Custom 1", "curve": [0.1, 0.2, 0.3, 1.4]},
                        {"name": "Custom 3", "curve": [0.0, -0.5, 1.0, 1.0]}
                    ]
                }),
            )
            .expect("valid easing preferences");
        assert_eq!(preferences.easing.tab, EasingTab::Saved);
        assert_eq!(preferences.easing.next_name(), "Custom 4");
        for value in [
            serde_json::json!({"tab": "unknown", "saved": []}),
            serde_json::json!({"tab": "saved", "saved": [{"name": "", "curve": [0, 0, 1, 1]}]}),
            serde_json::json!({"tab": "saved", "saved": [{"name": "x".repeat(65), "curve": [0, 0, 1, 1]}]}),
            serde_json::json!({"tab": "saved", "saved": [
                {"name": "A", "curve": [0, 0, 1, 1]}, {"name": "A", "curve": [0, 0, 1, 1]}]}),
            serde_json::json!({"tab": "saved", "saved": [{"name": "A", "curve": [1.5, 0, 1, 1]}]}),
            serde_json::json!({"tab": "saved", "saved": [{"name": "A", "curve": [0, 0, 1]}]}),
            serde_json::json!({"tab": "saved", "saved": (0..65).map(|n| serde_json::json!({"name": format!("P{n}"), "curve": [0, 0, 1, 1]})).collect::<Vec<_>>()}),
            serde_json::json!({"tab": "saved", "saved": [], "extra": true}),
        ] {
            let before = preferences.clone();
            assert!(preferences.set("easing", value).is_err());
            assert_eq!(preferences, before);
        }
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        std::fs::write(
            store.path(),
            serde_json::to_vec(&Preferences {
                easing: EasingPreferences {
                    tab: EasingTab::Saved,
                    saved: vec![SavedEasing {
                        name: String::new(),
                        curve: [0.0, 0.0, 1.0, 1.0],
                    }],
                },
                ..Default::default()
            })
            .expect("encode preferences"),
        )
        .expect("write preferences");
        let (loaded, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Layout));
        assert_eq!(loaded.easing, EasingPreferences::default());
    }

    #[test]
    fn repeated_splitter_drags_always_produce_valid_preferences() {
        for columns in [1236.0, 1809.0] {
            for first in [0, 1] {
                let mut panels = drag_panel(
                    DEFAULT_PANELS,
                    first,
                    if first == 0 { 10.0 } else { -10.0 },
                    columns,
                );
                for axis in [1 - first, first, 2, 0, 1] {
                    for delta in [-10.0, 0.00001, 10.0, -0.00001] {
                        panels = drag_panel(panels, axis, delta, columns);
                        assert!(validate_panels(panels).is_ok(), "{panels:?}");
                        let [media, player, properties] = column_widths(panels, columns);
                        assert!(media >= MEDIA_MIN_PX - 0.01, "{media}");
                        assert!(player >= PLAYER_MIN_PX - 0.01, "{player}");
                        assert!(properties >= PROPERTIES_MIN_PX - 0.01, "{properties}");
                    }
                }
            }
        }
    }

    #[test]
    fn oversized_preference_save_preserves_previous_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut preferences = Preferences::default();
        store.save(&preferences).unwrap();
        for index in 0..128 {
            preferences.sections.insert(format!("{index:064}"), false);
        }
        for index in 0..64 {
            preferences
                .windows
                .insert(format!("{index:064}"), WindowGeometry::default());
        }
        preferences
            .timeline_lanes
            .insert(project::ProjectId(1), (0..4096).collect());
        assert!(store.save(&preferences).is_err());
        assert_eq!(store.load_with_warning().0, Preferences::default());
    }
}

#[cfg(test)]
mod model_tests {
    #![allow(clippy::unwrap_used)]
    #[test]
    fn custom_remote_models_round_trip_and_resolve_into_caption_and_shorts_requests() {
        let mut settings = crate::preferences::AiPreferences::default();
        for row in ai::defaults::DEFAULTS
            .iter()
            .filter(|row| row.shipped_default)
        {
            settings
                .select(row.provider, row.feature, "fixture-5")
                .unwrap();
        }
        let mut preferences = crate::preferences::Preferences::default();
        preferences.subtitles.transcription = "openai".into();
        preferences
            .set("ai_models", serde_json::to_value(&settings).unwrap())
            .unwrap();
        assert_eq!(preferences.subtitles.transcription_model, "fixture-5");
        assert_eq!(preferences.subtitles.translation_model, "fixture-5");
        assert_eq!(preferences.subtitles.fillers_model, "fixture-5");
        let dir = tempfile::tempdir().unwrap();
        let store = crate::preferences::PreferenceStore::new(dir.path().join("prefs.json"));
        store.save(&preferences).unwrap();
        let loaded = store.load_with_warning().0;
        assert_eq!(loaded.ai_models, settings);
        assert_eq!(loaded.subtitles.transcription_model, "fixture-5");
        assert_eq!(
            loaded.subtitles.local_transcription_model,
            "whisper-large-v3-turbo"
        );
        let mut shorts = crate::shorts::State::new(
            Default::default(),
            engine::ByteBudget::new(64 << 20).unwrap(),
        )
        .unwrap();
        shorts.request.suggestion_provider = "anthropic".into();
        shorts.request.transcription_provider = "elevenlabs".into();
        shorts.apply_model_preferences(&loaded.ai_models);
        assert_eq!(shorts.request.suggestion_model, "fixture-5");
        assert_eq!(shorts.request.transcription_model, "fixture-5");
        for row in ai::defaults::DEFAULTS
            .iter()
            .filter(|row| row.shipped_default)
        {
            for index in 0..crate::services::remote_models::MAX_RECENT_MODELS {
                let model = format!("{index}{}", "x".repeat(ai::defaults::MAX_MODEL_BYTES - 1));
                preferences
                    .ai_models
                    .select(row.provider, row.feature, &model)
                    .unwrap();
            }
        }
        preferences.resolve_models();
        store.save(&preferences).unwrap();
        let restored = store.load_with_warning().0;
        assert_eq!(restored.ai_models, preferences.ai_models);
        // One text-task provider; each task snapshots its own row for that provider.
        preferences
            .ai_models
            .select("anthropic", ai::Operation::Translate, "fixture-translate")
            .unwrap();
        preferences
            .ai_models
            .select(
                "anthropic",
                ai::Operation::ClassifyFillers,
                "fixture-filler",
            )
            .unwrap();
        preferences
            .set("subtitles", {
                let mut subtitles = preferences.subtitles.clone();
                subtitles.text_provider = "anthropic".into();
                serde_json::to_value(subtitles).unwrap()
            })
            .unwrap();
        assert_eq!(preferences.subtitles.text_provider, "anthropic");
        assert_eq!(preferences.subtitles.translation_model, "fixture-translate");
        assert_eq!(preferences.subtitles.fillers_model, "fixture-filler");
        assert_eq!(
            restored.subtitles.transcription_model,
            preferences.subtitles.transcription_model
        );
    }
}
