//! Bounded preference file IO and the startup repair warning.

use super::{
    DEFAULT_PANELS, FontPickerPreferences, MAX_PREFERENCES_BYTES, MAX_TIMELINE_LANES,
    PREFERENCES_VERSION, Preferences, validate_panels,
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
    /// Strict read used by tests and callers that must refuse a damaged file.
    pub fn load(&self) -> Result<Preferences> {
        let value = self.read()?;
        value.validate()?;
        Ok(value)
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
        if value.recent_colors.len() > 16 {
            value.recent_colors.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.timeline_lane_count() > MAX_TIMELINE_LANES {
            value.timeline_lanes.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.font_picker.validate().is_err() {
            value.font_picker = FontPickerPreferences::default();
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
    use crate::i18n::Locale;
    use crate::panels::AssetsTab;
    use crate::preferences::FontTab;
    use crate::preferences::{
        PANEL_MAX, PANEL_MIN, PREFERENCE_KEYS, PREVIEW_MAX, PREVIEW_MIN, TIMELINE_MAX,
        TIMELINE_MIN, WindowGeometry, drag_panel,
    };
    use std::collections::BTreeSet;

    fn valid_preview(panels: [f32; 3]) -> bool {
        (PREVIEW_MIN..=PREVIEW_MAX).contains(&(1.0 - panels[0] - panels[1]))
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
        assert_eq!(Preferences::system().locale, Locale::from_system());
    }

    #[test]
    fn splitter_drags_clamp_each_region_to_its_documented_limits() {
        let start = DEFAULT_PANELS;
        let far_right = drag_panel(start, 0, 10.0);
        assert_eq!(far_right[0], PANEL_MAX);
        assert!(valid_preview(far_right));
        let far_left = drag_panel(start, 0, -10.0);
        assert_eq!(far_left[0], PANEL_MIN);
        assert!(valid_preview(far_left));
        let inspector_narrow = drag_panel(start, 1, 10.0);
        assert_eq!(inspector_narrow[1], PANEL_MIN);
        let inspector_wide = drag_panel(start, 1, -10.0);
        assert_eq!(
            inspector_wide[1],
            PANEL_MAX.min(1.0 - PREVIEW_MIN - start[0])
        );
        assert!(valid_preview(inspector_wide));
        let timeline_high = drag_panel(start, 2, -10.0);
        assert_eq!(timeline_high[2], TIMELINE_MAX);
        let timeline_low = drag_panel(start, 2, 10.0);
        assert_eq!(timeline_low[2], TIMELINE_MIN);
        assert_eq!(drag_panel(start, 7, 0.1), start);
        assert_eq!(drag_panel(start, 0, f32::NAN), start);
        assert!(validate_panels([0.40, 0.40, 0.50]).is_err());
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
            ("panels", serde_json::json!([0.30, 0.20, 0.35])),
            ("sections", serde_json::json!({"inspector.canvas": false})),
            ("assets_tab", serde_json::json!("captions")),
            ("properties_tabs", serde_json::json!({"text": "transform"})),
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
                    "bindings": {"timeline.snap": ["Ctrl+Shift+N"]},
                    "isCustomized": ["timeline.snap"]
                }),
            ),
            (
                "recent_colors",
                serde_json::json!([[10, 20, 30, 0], [255, 0, 100, 128]]),
            ),
            (
                "timeline_lanes",
                serde_json::json!({"12": [3, 4], "13": [7]}),
            ),
            (
                "font_picker",
                serde_json::json!({"tab": "favorites", "favorites": ["Anton", "Inter"]}),
            ),
            (
                "export",
                serde_json::json!({
                    "format": "webm",
                    "quality": "very_high",
                    "include_audio": false,
                    "output": {
                        "bit_rate": null,
                        "codec": "automatic",
                        "fps": null,
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
        let loaded = store.load().expect("load preferences");
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
            assert!(matches!(
                store.load(),
                Err(Error::Unsupported("preferences version"))
            ));
            assert_eq!(store.load_with_warning(), (Preferences::system(), None));
            store.save(&Preferences::default()).expect("overwrite old preferences");
            assert_eq!(store.load().expect("load saved"), Preferences::default());
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
        ] {
            assert!(preferences.set(key, value).is_err(), "{key}");
            assert_eq!(preferences, before, "{key} must fail atomically");
        }
    }

    #[test]
    fn tab_preferences_validate_kind_and_tab_ids_atomically() {
        let mut preferences = Preferences::default();
        preferences
            .set(
                "properties_tabs",
                serde_json::json!({
                    "text": "transform",
                    "video": "audio",
                    "graphic": "graphic"
                }),
            )
            .expect("valid tab memory");
        assert_eq!(
            preferences.properties_tabs.get("text").map(String::as_str),
            Some("transform")
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
    fn font_picker_preferences_validate_tabs_and_favorite_families_atomically() {
        let mut preferences = Preferences::default();
        preferences
            .set(
                "font_picker",
                serde_json::json!({"tab": "my_fonts", "favorites": ["Anton", "Inter"]}),
            )
            .expect("valid font preferences");
        assert_eq!(preferences.font_picker.tab, FontTab::MyFonts);
        assert_eq!(preferences.font_picker.favorites, ["Anton", "Inter"]);
        for value in [
            serde_json::json!({"tab": "unknown", "favorites": []}),
            serde_json::json!({"tab": "all", "favorites": ["Inter", "Inter"]}),
            serde_json::json!({"tab": "all", "favorites": [""]}),
            serde_json::json!({"tab": "all", "favorites": ["x".repeat(129)]}),
            serde_json::json!({"tab": "all", "favorites": vec!["Inter"; 257]}),
            serde_json::json!({"tab": "all", "favorites": [], "extra": true}),
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
                    tab: FontTab::Favorites,
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
    fn repeated_splitter_drags_always_produce_valid_preferences() {
        // Unequal side panels expose subtraction round-off at the preview minimum.
        let boundary = drag_panel([0.37875625, 0.25, 0.5], 1, -10.0);
        assert!(validate_panels(boundary).is_ok(), "{boundary:?}");
        for first in [0, 1] {
            let mut panels =
                drag_panel(DEFAULT_PANELS, first, if first == 0 { 10.0 } else { -10.0 });
            for axis in [1 - first, first, 2, 0, 1] {
                for delta in [-10.0, 0.00001, 10.0, -0.00001] {
                    panels = drag_panel(panels, axis, delta);
                    assert!(validate_panels(panels).is_ok(), "{panels:?}");
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
        assert_eq!(store.load().unwrap(), Preferences::default());
    }

    #[test]
    fn recent_colours_refuse_over_capacity_without_changing_preferences() {
        let mut preferences = Preferences::default();
        let recent = (0..16)
            .map(|r| project::Color([r, 0, 0, r]))
            .collect::<Vec<_>>();
        preferences
            .set("recent_colors", serde_json::to_value(&recent).unwrap())
            .unwrap();
        let before = preferences.clone();
        let mut oversized = recent;
        oversized.push(project::Color([255; 4]));
        assert!(
            preferences
                .set("recent_colors", serde_json::to_value(oversized).unwrap())
                .is_err()
        );
        assert_eq!(preferences, before);
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
        let dir = tempfile::tempdir().unwrap();
        let store = crate::preferences::PreferenceStore::new(dir.path().join("prefs.json"));
        store.save(&preferences).unwrap();
        let loaded = store.load().unwrap();
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
        let restored = store.load().unwrap();
        assert_eq!(restored.ai_models, preferences.ai_models);
        assert_eq!(
            restored.subtitles.transcription_model,
            preferences.subtitles.transcription_model
        );
    }
}
