//! Owns the scene registry, theme selection, scene-name dispatch.
use super::Failure;
use super::application::{AppHarness, SECOND, Theme};
use std::collections::BTreeSet;
use time::Tick;
use vibecut::i18n::Locale;

pub(super) struct Scene {
    pub(super) name: &'static str,
    pub(super) reference_dark: Option<&'static str>,
    pub(super) reference_light: Option<&'static str>,
    pub(super) build: fn(Theme, f32) -> Result<AppHarness, Failure>,
}
impl Scene {
    pub(super) fn reference(&self, theme: Theme) -> Option<&'static str> {
        match theme {
            Theme::Dark => self.reference_dark,
            Theme::Light => self.reference_light,
        }
    }
}

pub(super) const SCENES: &[Scene] = &[
    Scene {
        name: "settings",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| settings_dialog(theme, scale, Locale::En),
    },
    Scene {
        name: "settings-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| settings_dialog(theme, scale, Locale::Vi),
    },
    Scene {
        name: "player-timecode",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "timecode"),
    },
    Scene {
        name: "player-invalid",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "invalid"),
    },
    Scene {
        name: "player-fullscreen",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "fullscreen"),
    },
    Scene {
        name: "player-menu",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "menu"),
    },
    Scene {
        name: "player-zoom",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "zoom"),
    },
    Scene {
        name: "player-ratio",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "ratio"),
    },
    Scene {
        name: "player-quality",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "quality"),
    },
    Scene {
        name: "player-loading",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "loading"),
    },
    Scene {
        name: "player-failed",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "failed"),
    },
    Scene {
        name: "player-offline",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "offline"),
    },
    Scene {
        name: "player-zoomed",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "zoomed"),
    },
    Scene {
        name: "player-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_variant(theme, scale, "vi"),
    },
    Scene {
        name: "player-media",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_scene(theme, scale, false),
    },
    Scene {
        name: "player-empty",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| player_scene(theme, scale, true),
    },
    Scene {
        name: "assets-empty",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "empty"),
    },
    Scene {
        name: "assets-media",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "media"),
    },
    Scene {
        name: "assets-effects",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "effects"),
    },
    Scene {
        name: "assets-effects-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "effects-vi"),
    },
    Scene {
        name: "assets-effects-phase3",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "effects-phase3"),
    },
    Scene {
        name: "assets-effects-phase3-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "effects-phase3-vi"),
    },
    Scene {
        name: "assets-filters",
        reference_dark: Some("capcut-desktop-2026-11-05/capcut-desktop-221739.png"),
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "filters"),
    },
    Scene {
        name: "assets-transitions",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "transitions"),
    },
    Scene {
        name: "assets-transitions-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "transitions-vi"),
    },
    Scene {
        name: "assets-text",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "text"),
    },
    Scene {
        name: "assets-loading",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "loading"),
    },
    Scene {
        name: "assets-error",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "error"),
    },
    Scene {
        name: "assets-offline",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "offline"),
    },
    Scene {
        name: "assets-music",
        reference_dark: Some("capcut-desktop-2026-11-05/capcut-desktop-221703.png"),
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "music"),
    },
    Scene {
        name: "assets-captions",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "captions"),
    },
    Scene {
        name: "assets-media-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "media-vi"),
    },
    Scene {
        name: "assets-empty-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| assets_scene(theme, scale, "empty-vi"),
    },
    Scene {
        name: "captions-empty",
        reference_dark: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        reference_light: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        build: |theme, scale| captions_scene(theme, scale, Locale::En, false),
    },
    Scene {
        name: "captions-empty-vi",
        reference_dark: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        reference_light: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        build: |theme, scale| captions_scene(theme, scale, Locale::Vi, false),
    },
    Scene {
        name: "captions-word-highlight",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| caption_word_scene(theme, scale, Locale::En),
    },
    Scene {
        name: "captions-word-highlight-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| caption_word_scene(theme, scale, Locale::Vi),
    },
    Scene {
        name: "captions-editor",
        reference_dark: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        reference_light: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        build: |theme, scale| captions_scene(theme, scale, Locale::En, true),
    },
    Scene {
        name: "captions-editor-vi",
        reference_dark: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        reference_light: Some("live/06-text-captions/shots/web/06-web-captions-panel.png"),
        build: |theme, scale| captions_scene(theme, scale, Locale::Vi, true),
    },
    Scene {
        name: "projects",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| projects_scene(theme, scale, None),
    },
    Scene {
        name: "projects-rename",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| projects_scene(theme, scale, Some("rename")),
    },
    Scene {
        name: "projects-delete",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| projects_scene(theme, scale, Some("delete")),
    },
    Scene {
        name: "projects-info",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| projects_scene(theme, scale, Some("info")),
    },
    Scene {
        name: "timeline-zoom-low",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_zoom(theme, scale, 0.5),
    },
    Scene {
        name: "timeline-zoom-high",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_zoom(theme, scale, 100.0),
    },
    Scene {
        name: "shorts-pick",
        reference_dark: Some("screens-2026-11-04/shorts-1-pick.png"),
        reference_light: Some("screens-2026-11-04/shorts-1-pick.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "pick"),
    },
    Scene {
        name: "shorts-setup",
        reference_dark: Some("screens-2026-11-04/shorts-2-setup.png"),
        reference_light: Some("screens-2026-11-04/shorts-2-setup.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "setup"),
    },
    Scene {
        name: "shorts-generating",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "generating"),
    },
    Scene {
        name: "shorts-results",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "results"),
    },
    Scene {
        name: "shorts-layout",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "layout"),
    },
    Scene {
        name: "shorts-trim",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "trim"),
    },
    Scene {
        name: "shorts-review",
        reference_dark: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        reference_light: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "review"),
    },
    Scene {
        name: "shorts-pick-vi",
        reference_dark: Some("screens-2026-11-04/shorts-1-pick.png"),
        reference_light: Some("screens-2026-11-04/shorts-1-pick.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::Vi, "pick"),
    },
    Scene {
        name: "shorts-setup-vi",
        reference_dark: Some("screens-2026-11-04/shorts-2-setup.png"),
        reference_light: Some("screens-2026-11-04/shorts-2-setup.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::Vi, "setup"),
    },
    Scene {
        name: "shorts-review-vi",
        reference_dark: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        reference_light: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::Vi, "review"),
    },
    Scene {
        name: "models",
        reference_dark: None,
        reference_light: None,
        build: models,
    },
    Scene {
        name: "models-vi",
        reference_dark: None,
        reference_light: None,
        build: models_vi,
    },
    Scene {
        name: "inspector-speed",
        reference_dark: None,
        reference_light: None,
        build: inspector_speed,
    },
    Scene {
        name: "inspector-audio",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_audio,
    },
    Scene {
        name: "inspector-audio-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_audio_vi,
    },
    Scene {
        name: "home",
        reference_dark: Some("capcut/capcut-desktop-my-projects.png"),
        reference_light: Some("capcut/capcut-desktop-my-projects.png"),
        build: home,
    },
    Scene {
        name: "home-projects",
        reference_dark: None,
        reference_light: None,
        build: home_projects,
    },
    Scene {
        name: "home-trash",
        reference_dark: None,
        reference_light: None,
        build: projects_trash,
    },
    Scene {
        name: "editor-empty",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: editor_empty,
    },
    Scene {
        name: "editor-media",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: editor_media,
    },
    Scene {
        name: "editor-transitions",
        reference_dark: Some("capcut-desktop-2026-11-05/capcut-desktop-221731.png"),
        reference_light: Some("capcut-desktop-2026-11-05/capcut-desktop-221731.png"),
        build: editor_transitions,
    },
    Scene {
        name: "editor-menu",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| editor_frame_state(theme, scale, "menu"),
    },
    Scene {
        name: "editor-rename",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| editor_frame_state(theme, scale, "rename"),
    },
    Scene {
        name: "editor-minimum",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| editor_frame_state(theme, scale, "minimum"),
    },
    Scene {
        name: "editor-frame-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| editor_frame_state(theme, scale, "vi"),
    },
    Scene {
        name: "timeline-selection",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: timeline_selection,
    },
    Scene {
        name: "timeline-selection-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: timeline_selection_vi,
    },
    Scene {
        name: "inspector-mixed-selection",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_mixed,
    },
    Scene {
        name: "text-item",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: text_item,
    },
    Scene {
        name: "text-item-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: text_item_vi,
    },
    Scene {
        name: "inspector-mixed-selection-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_mixed_vi,
    },
    Scene {
        name: "inspector-blend",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_blend,
    },
    Scene {
        name: "inspector-effects-streak",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-streak"),
    },
    Scene {
        name: "inspector-effects-streak-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-streak-vi"),
    },
    Scene {
        name: "inspector-effects-edge-lines",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-edge-lines"),
    },
    Scene {
        name: "inspector-effects-edge-lines-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-edge-lines-vi"),
    },
    Scene {
        name: "inspector-effects-relief",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-relief"),
    },
    Scene {
        name: "inspector-effects-relief-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-relief-vi"),
    },
    Scene {
        name: "inspector-effects",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects"),
    },
    Scene {
        name: "inspector-effects-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-vi"),
    },
    Scene {
        name: "inspector-effects-colours",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-colours"),
    },
    Scene {
        name: "inspector-effects-colours-picker",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-colours-picker"),
    },
    Scene {
        name: "inspector-effects-threshold",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-threshold"),
    },
    Scene {
        name: "inspector-effects-threshold-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-threshold-vi"),
    },
    Scene {
        name: "inspector-effects-colours-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-colours-vi"),
    },
    Scene {
        name: "inspector-effects-tonal",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-tonal"),
    },
    Scene {
        name: "inspector-effects-tonal-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-tonal-vi"),
    },
    Scene {
        name: "inspector-effects-mixer",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-mixer"),
    },
    Scene {
        name: "inspector-effects-mixer-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-mixer-vi"),
    },
    Scene {
        name: "inspector-effects-mixer-bottom",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-mixer-bottom"),
    },
    Scene {
        name: "inspector-effects-mixer-bottom-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| inspector_video_tab(theme, scale, "effects-mixer-bottom-vi"),
    },
    Scene {
        name: "inspector-video",
        reference_dark: None,
        reference_light: None,
        build: inspector_video,
    },
    Scene {
        name: "inspector-video-keyframes",
        reference_dark: Some("capcut-desktop-2026-11-05/capcut-desktop-221703.png"),
        reference_light: None,
        build: inspector_video_keyframes,
    },
    Scene {
        name: "inspector-remove-background",
        reference_dark: Some("capcut-desktop-2026-11-14/capcut-desktop-100332.png"),
        reference_light: None,
        build: inspector_remove_background,
    },
    Scene {
        name: "inspector-project",
        reference_dark: None,
        reference_light: None,
        build: inspector_project,
    },
    Scene {
        name: "inspector-project-vi",
        reference_dark: None,
        reference_light: None,
        build: inspector_project_vi,
    },
    Scene {
        name: "inspector-text",
        reference_dark: None,
        reference_light: None,
        build: inspector_text,
    },
    Scene {
        name: "inspector-transform",
        reference_dark: None,
        reference_light: None,
        build: inspector_transform,
    },
    Scene {
        name: "inspector-color",
        reference_dark: None,
        reference_light: None,
        build: inspector_color,
    },
    Scene {
        name: "inspector-blend-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_blend_vi,
    },
    Scene {
        name: "import-progress",
        reference_dark: Some("capcut/illustrations/capcut-modify-project.png"),
        reference_light: Some("capcut/illustrations/capcut-modify-project.png"),
        build: import_progress,
    },
    Scene {
        name: "import-complete-vi",
        reference_dark: Some("capcut/illustrations/capcut-modify-project.png"),
        reference_light: Some("capcut/illustrations/capcut-modify-project.png"),
        build: import_complete_vi,
    },
    Scene {
        name: "export-dialog",
        reference_dark: Some("capcut/capcut-desktop-export-2k4k.png"),
        reference_light: Some("capcut/capcut-desktop-export-2k4k.png"),
        build: export_dialog,
    },
    Scene {
        name: "export-resolution",
        reference_dark: Some("capcut/capcut-desktop-export-2k4k.png"),
        reference_light: Some("capcut/capcut-desktop-export-2k4k.png"),
        build: export_resolution,
    },
    Scene {
        name: "close-prompt",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: close_prompt,
    },
    Scene {
        name: "close-prompt-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: close_prompt_vi,
    },
    Scene {
        name: "timeline-main",
        reference_dark: None,
        reference_light: None,
        build: timeline_main,
    },
    Scene {
        name: "timeline-multi",
        reference_dark: None,
        reference_light: None,
        build: timeline_multi,
    },
    Scene {
        name: "timeline-fades-clamped",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_fades(theme, scale, "clamped"),
    },
    Scene {
        name: "timeline-fades-cancelled",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_fades(theme, scale, "cancelled"),
    },
    Scene {
        name: "timeline-fades-split",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_fades(theme, scale, "split"),
    },
    Scene {
        name: "timeline-fades-menu-vi",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_fades(theme, scale, "menu"),
    },
    Scene {
        name: "preview-axis",
        reference_dark: None,
        reference_light: None,
        build: preview_axis,
    },
    Scene {
        name: "preview-axis-reference",
        reference_dark: None,
        reference_light: None,
        build: preview_axis_reference,
    },
    Scene {
        name: "preview-axis-layers",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| preview_axis_ends(theme, scale, Tick(3 * SECOND)),
    },
    Scene {
        name: "preview-axis-tail",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| preview_axis_ends(theme, scale, Tick(6 * SECOND)),
    },
    Scene {
        name: "tracks",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        build: tracks,
    },
    Scene {
        name: "scene-delete",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        build: scene_delete,
    },
    Scene {
        name: "scene-create",
        reference_dark: None,
        reference_light: None,
        build: scene_create,
    },
    Scene {
        name: "scene-rename",
        reference_dark: None,
        reference_light: None,
        build: scene_rename,
    },
    Scene {
        name: "placement",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: placement,
    },
];
pub(super) fn select(
    names: impl Iterator<Item = String>,
) -> Result<Vec<(&'static Scene, Theme)>, Failure> {
    let names: Vec<String> = names.collect();
    if names.is_empty() {
        return Ok(SCENES
            .iter()
            .flat_map(|scene| Theme::ALL.map(|theme| (scene, theme)))
            .collect());
    }
    let mut selected = BTreeSet::new();
    let mut unknown = Vec::new();
    for name in names {
        let mut matched = false;
        for (index, scene) in SCENES.iter().enumerate() {
            if name == scene.name {
                for (theme_index, _theme) in Theme::ALL.into_iter().enumerate() {
                    selected.insert((index, theme_index));
                }
                matched = true;
            } else {
                for (theme_index, theme) in Theme::ALL.into_iter().enumerate() {
                    if name == format!("{}-{}", scene.name, theme.key()) {
                        selected.insert((index, theme_index));
                        matched = true;
                    }
                }
            }
        }
        if !matched {
            unknown.push(name);
        }
    }
    if !unknown.is_empty() {
        let known: Vec<_> = SCENES
            .iter()
            .flat_map(|scene| {
                Theme::ALL
                    .into_iter()
                    .map(move |theme| format!("{}-{}", scene.name, theme.key()))
            })
            .collect();
        return Err(format!(
            "unknown scene(s): {}; known scenes: {}",
            unknown.join(", "),
            known.join(", ")
        )
        .into());
    }
    selected
        .into_iter()
        .map(|(index, theme_index)| {
            let scene = SCENES.get(index).ok_or("scene index")?;
            let theme = Theme::ALL
                .into_iter()
                .nth(theme_index)
                .ok_or("theme index")?;
            Ok((scene, theme))
        })
        .collect()
}
mod assets;
mod editor;
mod home;
mod properties;
mod shorts;
mod timeline;
use assets::assets_scene;
use editor::{
    caption_word_scene, captions_scene, editor_empty, editor_frame_state, editor_media,
    editor_transitions, export_dialog, export_resolution, import_progress, models, models_vi,
    player_scene, player_variant, settings_dialog,
};
use home::{home, home_projects, projects_scene, projects_trash};
pub(crate) use properties::inspector_mixed_vi;
use properties::{
    close_prompt, close_prompt_vi, import_complete_vi, inspector_audio, inspector_audio_vi,
    inspector_blend, inspector_blend_vi, inspector_color, inspector_mixed, inspector_project,
    inspector_project_vi, inspector_remove_background, inspector_speed, inspector_text,
    inspector_transform, inspector_video, inspector_video_keyframes, inspector_video_tab,
    text_item, text_item_vi,
};
use shorts::shorts_scene;
use timeline::{
    placement, preview_axis, preview_axis_ends, preview_axis_reference, scene_create, scene_delete,
    scene_rename, timeline_fades, timeline_main, timeline_multi, timeline_selection,
    timeline_selection_vi, timeline_zoom, tracks,
};
