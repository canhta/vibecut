//! Offscreen UI audit scenes for design review.
//!
//! `cargo xtask ui-audit` renders named scenes through the real [`Editor`] and
//! [`Workspace`] widget paths at 1x and 2x, plus an `index.html` that places each
//! render beside its matching `.spikes/ui-reference/` image when one exists.
//! This is an audit tool, not a test: it never runs in `cargo xtask ci`.
#[path = "../tests/it/support/shorts.rs"]
mod shorts_fixture;
#[path = "../tests/it/support/subtitles.rs"]
mod subtitles_fixture;
use captions::{Cue, CueText, CueTrack, LanguageTag};
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{
    ByteBudget, Command, CreatedId, ItemParamPatch, NewItem, Placement, Session, TrackPlacement,
    TrimEdge,
};
use media::{AssetData, AssetId, ContentHash, MediaError, SourceAsset, SourceStream, StreamKind};
use project::{
    AudioParams, CaptionParams, Color, ItemId, ItemParams, OverlayParams, Project, ProjectId,
    TextParams, TrackId, TrackKind, VideoParams,
};
use render::{AudioBlock, ExecutionHooks, Image, MemoryKind, SourceProvider, UnavailableReason};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, ExitStatus, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    AppCommand, Editor, Inspection, PreviewZoom, egui,
    i18n::Locale,
    import::ImportProbe,
    model_manager::ModelManager,
    preferences::Preferences,
    services::playback::{Files, Playback, Sources},
    workspace::Workspace,
};

const SECOND: i64 = TICKS_PER_SECOND;
const WINDOW: [f32; 2] = [1440.0, 900.0];
const SCALES: [f32; 2] = [1.0, 2.0];

type Failure = Box<dyn std::error::Error>;
type AppHarness = Harness<'static, AuditApp>;

/// Audit roots: Home, the full editor and the dedicated Shorts window.
enum App {
    Home {
        workspace: Box<Workspace>,
        _directory: tempfile::TempDir,
    },
    Editor(Box<Editor>),
    Player(Box<Editor>),
    Assets(Box<Editor>),
    Shorts(Box<vibecut::shorts::State>),
}
impl App {
    fn ui(&mut self, ui: &mut egui::Ui) {
        match self {
            Self::Home { workspace, .. } => workspace.ui(ui),
            Self::Editor(editor) => editor.ui(ui),
            Self::Player(editor) => {
                let initialized = ui.ctx().data_mut(|data| {
                    let id = egui::Id::unique("player.audit.initialized");
                    let initialized = data.get_temp::<bool>(id).unwrap_or(false);
                    data.insert_temp(id, true);
                    initialized
                });
                if !initialized {
                    editor.ui(ui);
                    return;
                }
                editor.tick();
                ui.painter().rect_filled(
                    ui.max_rect(),
                    0,
                    vibecut::window_color(editor.preferences.dark),
                );
                let rect = egui::Rect::from_min_size(
                    egui::pos2(654.0, 74.0),
                    egui::vec2(
                        if editor.session.project().duration() == Tick::ZERO {
                            578.0
                        } else {
                            628.0
                        },
                        532.0,
                    ),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                editor.preview_ui(&mut child);
            }
            Self::Shorts(shorts) => {
                let cx = vibecut::shorts::Context {
                    editor_available: true,
                };
                vibecut::shorts::show(shorts, &cx, ui);
            }
            Self::Assets(editor) => {
                let rect =
                    egui::Rect::from_min_size(egui::pos2(9.0, 36.0), egui::vec2(582.0, 532.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                child.set_clip_rect(rect);
                editor.assets_ui(&mut child);
            }
        }
    }
    fn busy(&self) -> bool {
        match self {
            Self::Home { workspace, .. } => workspace.busy(),
            Self::Editor(_) | Self::Player(_) | Self::Assets(_) => false,
            Self::Shorts(shorts) => shorts.busy(),
        }
    }
    fn inspect(&self) -> Result<Inspection, Failure> {
        match self {
            Self::Editor(editor) | Self::Player(editor) | Self::Assets(editor) => {
                Ok(editor.inspect()?)
            }
            Self::Home { .. } => Err("the Home scene has no editor inspection".into()),
            Self::Shorts(_) => Err("the Shorts scene has no editor inspection".into()),
        }
    }
    fn presented_frame(&self) -> Option<Tick> {
        match self {
            Self::Editor(editor) | Self::Player(editor) | Self::Assets(editor) => {
                editor.presented_frame()
            }
            Self::Shorts(shorts) => shorts.presented_frame(),
            Self::Home { workspace, .. } => workspace
                .focused_editor()
                .and_then(|editor| editor.presented_frame()),
        }
    }
}

/// eframe mode hands the app the same margin-free root Ui the native shell gets;
/// kittest's plain Ui mode inserts an 8 px central-panel frame that the real
/// window does not have.
struct AuditApp {
    app: App,
    _model_cache: Option<tempfile::TempDir>,
    _sound_library: Option<tempfile::TempDir>,
}
impl AuditApp {
    fn busy(&self) -> bool {
        self.app.busy()
    }
    fn inspect(&self) -> Result<Inspection, Failure> {
        self.app.inspect()
    }
    fn presented_frame(&self) -> Option<Tick> {
        self.app.presented_frame()
    }
    fn session(&self) -> Option<&Session> {
        match &self.app {
            App::Editor(editor) | App::Player(editor) | App::Assets(editor) => {
                Some(&editor.session)
            }
            App::Home { .. } => None,
            App::Shorts(shorts) => shorts.preview_session(),
        }
    }
}
impl eframe::App for AuditApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Theme {
    Dark,
    Light,
}
impl Theme {
    const ALL: [Self; 2] = [Self::Dark, Self::Light];
    fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    fn dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

struct Scene {
    name: &'static str,
    reference_dark: Option<&'static str>,
    reference_light: Option<&'static str>,
    build: fn(Theme, f32) -> Result<AppHarness, Failure>,
}
impl Scene {
    fn reference(&self, theme: Theme) -> Option<&'static str> {
        match theme {
            Theme::Dark => self.reference_dark,
            Theme::Light => self.reference_light,
        }
    }
}

const SCENES: &[Scene] = &[
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
        name: "inspector-video-blend",
        reference_dark: None,
        reference_light: None,
        build: inspector_video_blend,
    },
    Scene {
        name: "inspector-project",
        reference_dark: None,
        reference_light: None,
        build: inspector_project,
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

fn preferences(theme: Theme, locale: Locale) -> Preferences {
    Preferences {
        locale,
        dark: theme.dark(),
        ..Preferences::default()
    }
}

fn harness(app: App, scale: f32) -> AppHarness {
    harness_sized(app, scale, WINDOW)
}

fn harness_sized(app: App, scale: f32, size: [f32; 2]) -> AppHarness {
    Harness::builder()
        .with_size(egui::vec2(size[0], size[1]))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp {
            app,
            _model_cache: None,
            _sound_library: None,
        })
}

fn home(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let workspace = Workspace::new(
        directory.path().join("recovery"),
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    // Discovery starts in the background; Home shows New/Open once it settles.
    for _ in 0..500 {
        app.run_steps(1);
        if !app.state().busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    Ok(app)
}

/// Home at the measured desktop window size with a full row of projects.
const HOME_WINDOW: [f32; 2] = [1728.0, 1030.0];

fn home_projects(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("projects");
    {
        let mut library = engine::library::ProjectLibrary::open(&root, ByteBudget::new(64 << 20)?)?;
        for id in 1..=10u64 {
            library.execute(
                ProjectId(id),
                Command::CreateProject {
                    id: ProjectId(id),
                    name: format!("Draft {id}").into(),
                },
            )?;
        }
    }
    let workspace = Workspace::with_projects(
        directory.path().join("recovery"),
        root,
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness_sized(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
        HOME_WINDOW,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("home audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    Ok(app)
}

fn projects_trash(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("projects");
    {
        let mut library = engine::library::ProjectLibrary::open(&root, ByteBudget::new(64 << 20)?)?;
        for id in 1..=6u64 {
            library.execute(
                ProjectId(id),
                Command::CreateProject {
                    id: ProjectId(id),
                    name: format!("Draft {id}").into(),
                },
            )?;
        }
        for id in [2, 5] {
            library.execute(ProjectId(id), Command::DeleteProject {})?;
        }
    }
    let workspace = Workspace::with_projects(
        directory.path().join("recovery"),
        root,
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness_sized(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
        HOME_WINDOW,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("home trash audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.get(By::new().predicate(|node| node.author_id() == Some("projects.trash")))
        .click();
    app.run_steps(2);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("home trash audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Keep the pointer off the toggle so the capture shows its selected state,
    // not the click's hover surface.
    app.hover_at(egui::pos2(4.0, 4.0));
    app.run_steps(2);
    Ok(app)
}

fn projects_scene(theme: Theme, scale: f32, dialog: Option<&str>) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("projects");
    {
        let mut library = engine::library::ProjectLibrary::open(&root, ByteBudget::new(64 << 20)?)?;
        for (id, name) in [
            (1, "Product launch"),
            (2, "Tiếng Việt: bản dựng"),
            (3, "Monthly recap"),
        ] {
            library.execute(
                ProjectId(id),
                Command::CreateProject {
                    id: ProjectId(id),
                    name: name.into(),
                },
            )?;
        }
    }
    let workspace = Workspace::with_projects(
        directory.path().join("recovery"),
        root,
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.state().busy() {
        app.run_steps(1);
        if Instant::now() >= deadline {
            return Err("projects audit timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    if let Some(dialog) = dialog {
        app.get(
            By::new()
                .predicate(|node| node.author_id() == Some("projects.card.0000000000000001.menu")),
        )
        .click();
        app.run_steps(2);
        let id = format!("projects.card.0000000000000001.{dialog}");
        app.get(By::new().predicate(move |node| node.author_id() == Some(id.as_str())))
            .click();
        app.run_steps(2);
    }
    Ok(app)
}

fn empty_session(name: &str) -> Result<Session, Failure> {
    Ok(Session::new(
        Project::empty(ProjectId(46), name.into())?,
        50,
    )?)
}

fn app_editor(editor: Editor, scale: f32) -> AppHarness {
    harness(App::Editor(Box::new(editor)), scale)
}

fn empty_editor(theme: Theme, locale: Locale) -> Result<Editor, Failure> {
    let session = empty_session("Untitled Project")?;
    // Media import admits a 256 MiB probe/work lease before it opens a file.
    let mut playback = Playback::new(&session, ByteBudget::new(512 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    Ok(Editor::new(session, playback, preferences(theme, locale)))
}

fn editor_empty(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    Ok(harness_sized(
        App::Editor(Box::new(empty_editor(theme, Locale::En)?)),
        scale,
        [1727.0, 1029.0],
    ))
}

fn editor_frame_state(theme: Theme, scale: f32, state: &str) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(
        theme,
        if state == "vi" {
            Locale::Vi
        } else {
            Locale::En
        },
    )?;
    if state == "rename" {
        editor.activate("project.rename")?;
    }
    let size = if state == "minimum" {
        [1280.0, 800.0]
    } else {
        [1727.0, 1029.0]
    };
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, size);
    app.run_steps(3);
    if state == "menu" {
        app.get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
            .click();
        app.run_steps(2);
    }
    Ok(app)
}

fn captions_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
    filled: bool,
) -> Result<AppHarness, Failure> {
    let mut editor = if filled {
        subtitles_fixture::editor("vi")
    } else {
        empty_editor(theme, locale)?
    };
    editor.preferences = preferences(theme, locale);
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Captions;
    editor.prepare_subtitles()?;
    if filled {
        let mut panel = editor.subtitles.take().ok_or("captions missing")?;
        panel.tick(&mut editor.session)?;
        panel.edit_text(
            &mut editor.session,
            &"cue-0".into(),
            captions::LanguageTag::parse("en")?,
            "Hello, and welcome to the caption editor.",
        )?;
        panel.select_cue(&editor.session, 0, false, false)?;
        editor.subtitles = Some(panel);
        editor
            .playback
            .seek(Tick(2 * SECOND), false, editor.session.project().clone())?;
    }
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.run_steps(4);
    Ok(app)
}

struct NoTransfer;
impl ai::transport::DownloadTransport for NoTransfer {
    fn get(
        &self,
        _: &str,
        _: &[(String, String)],
    ) -> Result<ai::transport::HttpStreamResponse, ai::Error> {
        Err(ai::Error::Network)
    }
}

fn models(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    models_locale(theme, scale, Locale::En)
}

fn models_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    models_locale(theme, scale, Locale::Vi)
}

fn models_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let cache = tempfile::tempdir()?;
    let budget = ByteBudget::new(8 << 20)?;
    let (artifacts, _) = engine::cache::ArtifactCache::open(cache.path(), budget.clone(), 8)?;
    let transfers = engine::models::ModelTransfers::new(
        artifacts,
        Arc::new(NoTransfer),
        Arc::new(engine::models::SystemModelIo),
    );
    let manager = ModelManager::with_transfers(budget, ai::models::catalog()?, transfers)?;
    let mut editor = empty_editor(theme, locale)?;
    editor.set_model_manager(manager);
    editor.open_models()?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.state_mut()._model_cache = Some(cache);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.run_steps(1);
        if !app
            .state()
            .inspect()?
            .models
            .as_ref()
            .is_some_and(|models| models.checking)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("model check did not finish before the audit deadline".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(4);
    Ok(app)
}

fn source_stream(kind: StreamKind, duration: Tick) -> Result<SourceStream, Failure> {
    Ok(testkit::source::stream(kind).duration(duration).build())
}

fn source_asset(id: u64, kind: StreamKind, path: PathBuf) -> Result<SourceAsset, Failure> {
    let path = std::path::absolute(path)?;
    let duration = Tick(10 * SECOND);
    Ok(testkit::source::asset(id, path)
        .format("mov,mp4")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
        .stream(source_stream(kind, duration)?)
        .build())
}

fn range(start: i64, end: i64) -> Result<SourceRange, Failure> {
    Ok(SourceRange::new(Tick(start * SECOND), Tick(end * SECOND))?)
}

fn track(session: &mut Session, name: &str, kind: TrackKind) -> Result<TrackId, Failure> {
    let outcome = session.apply(Command::AddTrack {
        placement: TrackPlacement::Auto,
        name: name.into(),
        kind,
    })?;
    Ok(outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            CreatedId::Track(id) => Some(id),
            _ => None,
        })
        .ok_or("created track id")?)
}

fn insert(
    session: &mut Session,
    track: TrackId,
    start: Tick,
    source_range: SourceRange,
    asset: Option<AssetId>,
    content: ItemParams,
) -> Result<ItemId, Failure> {
    let outcome = session.apply(Command::InsertItem {
        placement: Placement::Track { track },
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: asset,
            source_range,
            start,
            content,
        },
    })?;
    Ok(outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            CreatedId::Item(id) => Some(id),
            _ => None,
        })
        .ok_or("created item id")?)
}

/// Deterministic healthy media for populated-editor scenes. The provider is
/// injected through the same public `Sources` seam production `Files` uses, so
/// the audit exercises the real preview/render path without opening a file.
struct SyntheticSources;
impl Sources for SyntheticSources {
    fn create(&self) -> Box<dyn SourceProvider> {
        Box::new(SyntheticFrames)
    }
}
struct SyntheticFrames;
impl SourceProvider for SyntheticFrames {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> render::Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: Tick,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<Image>> {
        const WIDTH: u32 = 640;
        const HEIGHT: u32 = 360;
        let mut bytes = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let [red, green, blue] = if (160..480).contains(&x) && (90..270).contains(&y) {
                    [238, 196, 52]
                } else {
                    [
                        (24 + x * 180 / WIDTH) as u8,
                        (58 + y * 130 / HEIGHT) as u8,
                        148,
                    ]
                };
                bytes.extend_from_slice(&[red, green, blue, 255]);
            }
        }
        Ok(Some(Image::from_rgba8(
            WIDTH, HEIGHT, &bytes, false, 13, hooks,
        )?))
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<AudioBlock> {
        AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)
    }
}

fn wait_for_frame(editor: &mut Editor) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.playback.frame().is_none() {
        editor.playback.poll(std::time::Instant::now())?;
        if Instant::now() >= deadline {
            return Err("preview frame timeout".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn wait_for_presented(app: &mut AppHarness) -> Result<(), Failure> {
    for _ in 0..500 {
        app.run_steps(1);
        if app.state().presented_frame().is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("preview frame was not presented".into())
}

/// Capture settled pixels: wait for every async effect preview, then load the
/// finished textures and paint the frame the audit reads.
fn settle(app: &mut AppHarness) -> Result<(), Failure> {
    let timeout = Duration::from_secs(10);
    match &mut app.state_mut().app {
        App::Editor(editor) | App::Player(editor) | App::Assets(editor) => {
            editor.settle_previews(timeout)?;
        }
        App::Home { workspace, .. } => {
            if let Some(editor) = workspace.focused_editor_mut() {
                editor.settle_previews(timeout)?;
            }
        }
        App::Shorts(_) => {}
    }
    app.run_steps(2);
    Ok(())
}

fn media_editor(theme: Theme, locale: Locale) -> Result<(Editor, ItemId), Failure> {
    let mut session = empty_session("Launch cut")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(
                id,
                kind,
                std::env::temp_dir().join(format!("source-{id}.mp4")),
            )?,
        })?;
    }
    let main = session.project().data().scene().timeline.main.id;
    let first = insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 6)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    )?;
    insert(
        &mut session,
        main,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            crop: None,
            muted: false,
        }),
    )?;
    let audio = track(&mut session, "Audio", TrackKind::Audio)?;
    insert(
        &mut session,
        audio,
        Tick::ZERO,
        range(0, 10)?,
        Some(AssetId(2)),
        ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
        }),
    )?;
    let text = track(&mut session, "Text", TrackKind::Text)?;
    insert(
        &mut session,
        text,
        Tick(2 * SECOND),
        range(2, 5)?,
        None,
        ItemParams::Text(TextParams {
            content: "Chapter one".into(),
            font_size: 48.0,
            color: Color([255, 255, 255, 255]),
            ..Default::default()
        }),
    )?;
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, preferences(theme, locale));
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    Ok((editor, first))
}

fn timeline_zoom(theme: Theme, scale: f32, zoom: f64) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.apply(Command::SetViewState {
        zoom,
        scroll: 0.0,
        playhead: Tick::ZERO,
    })?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn player_editor(theme: Theme, empty: bool) -> Result<Editor, Failure> {
    let mut session = empty_session("Interview")?;
    let rate = FrameRate::FPS_30;
    let duration = rate.tick_of((3600 + 18 * 60 + 28) * 30 + 6)?;
    session.apply(Command::RenameScene {
        scene: session.project().data().current_scene_id,
        name: "Timeline 01".into(),
    })?;
    if !empty {
        let mut asset: AssetData = source_asset(
            1,
            StreamKind::Video,
            std::env::temp_dir().join("player-source.mp4"),
        )?
        .into();
        asset.probe.duration = Some(duration);
        asset.probe.streams = vec![source_stream(StreamKind::Video, duration)?];
        session.apply(Command::AddAsset {
            asset: SourceAsset::try_from(asset)?,
        })?;
        let main = session.project().data().scene().timeline.main.id;
        insert(
            &mut session,
            main,
            Tick::ZERO,
            SourceRange::new(Tick::ZERO, duration)?,
            Some(AssetId(1)),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                volume: 0.0,
                crop: None,
                muted: false,
                fades: Default::default(),
            }),
        )?;
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    if !empty {
        playback.seek(rate.tick_of(11 * 30 + 7)?, false, session.project().clone())?;
    }
    let mut editor = Editor::new(session, playback, preferences(theme, Locale::En));
    if !empty {
        wait_for_frame(&mut editor)?;
    }
    Ok(editor)
}
fn player_scene(theme: Theme, scale: f32, empty: bool) -> Result<AppHarness, Failure> {
    player_variant(theme, scale, if empty { "empty" } else { "media" })
}
fn player_variant(theme: Theme, scale: f32, variant: &str) -> Result<AppHarness, Failure> {
    let mut editor = player_editor(theme, variant == "empty")?;
    if variant == "offline" {
        editor.playback.set_device_available(false);
    }
    if variant == "vi" {
        editor.preferences.locale = Locale::Vi;
    }
    if variant == "zoomed" {
        editor.dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(1.25)))?;
    }
    if matches!(variant, "loading" | "failed") {
        editor.playback = Playback::new(
            &editor.session,
            ByteBudget::new(256 << 20)?,
            if variant == "loading" {
                Arc::new(LoadingSources)
            } else {
                Arc::new(Files)
            },
        )?;
        editor.playback.set_device_available(true);
    }
    if variant == "fullscreen" {
        editor.activate("preview.fullscreen")?;
    }
    let app_root = if variant == "fullscreen" {
        App::Editor(Box::new(editor))
    } else {
        App::Player(Box::new(editor))
    };
    let mut app = harness_sized(app_root, scale, [1840.0, 1142.0]);
    app.run_steps(3);
    if !matches!(variant, "empty" | "loading" | "failed") {
        wait_for_presented(&mut app)?;
    }
    if variant == "failed" {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app
            .state()
            .inspect()?
            .widgets
            .iter()
            .any(|widget| widget.id == "preview.retry")
        {
            if Instant::now() >= deadline {
                return Err("player failure audit timed out".into());
            }
            app.run_steps(1);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if let Some(id) = match variant {
        "menu" => Some("preview.menu"),
        "zoom" => Some("preview.zoom"),
        "ratio" => Some("preview.ratio"),
        "quality" => Some("preview.quality"),
        "timecode" | "invalid" => Some("preview.timecode"),
        _ => None,
    } {
        app.get(By::new().predicate(|node| node.author_id() == Some(id)))
            .click();
        app.run_steps(3);
    }
    if variant == "invalid" {
        app.get(By::new().predicate(|node| node.author_id() == Some("preview.timecode")))
            .type_text("99:99:99:99");
        app.key_press(egui::Key::Enter);
        app.run_steps(3);
    }
    Ok(app)
}
/// Keep the loading scene genuinely pending until its preview jobs are cancelled.
struct LoadingSources;
impl Sources for LoadingSources {
    fn create(&self) -> Box<dyn SourceProvider> {
        Box::new(LoadingFrames)
    }
}
struct LoadingFrames;
impl SourceProvider for LoadingFrames {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<UnavailableReason>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !hooks.cancelled() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(render::Error::Cancelled)
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> render::Result<Option<Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<AudioBlock> {
        AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)
    }
}

fn editor_media(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, Locale::En)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}

fn timeline_selection_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let (editor, item) = media_editor(theme, locale)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    let id = format!("timeline.item.{}", item.0);
    let node = app
        .query_all(By::new().predicate(|node| node.author_id() == Some(id.as_str())))
        .next()
        .ok_or_else(|| format!("missing automation id {id}"))?;
    node.click();
    app.run_steps(3);
    Ok(app)
}

fn timeline_selection(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_selection_locale(theme, scale, Locale::En)
}

fn timeline_selection_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_selection_locale(theme, scale, Locale::Vi)
}

fn inspector_speed(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, item) = media_editor(theme, Locale::En)?;
    editor.selection = [item].into_iter().collect();
    editor.session.apply(Command::SetSpeed {
        items: vec![item],
        speed: time::Speed::from_f64(1.5)?,
        maintain_pitch: true,
    })?;
    editor
        .preferences
        .set("properties_tabs", serde_json::json!({"video":"speed"}))?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    widget_rect(&app, "inspector.speed.pitch")?;
    Ok(app)
}

fn inspector_audio_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, locale)?;
    let audio = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .and_then(|track| track.items.first())
        .ok_or("audio item")?
        .id;
    editor.selection = [audio].into_iter().collect();
    editor.set_shared_field("audio_gain", "-3")?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn inspector_audio(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_audio_locale(theme, scale, Locale::En)
}

fn inspector_audio_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_audio_locale(theme, scale, Locale::Vi)
}

fn widget_rect(app: &AppHarness, id: &str) -> Result<egui::Rect, Failure> {
    app.state()
        .inspect()?
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .map(|widget| widget.rect)
        .ok_or_else(|| format!("missing widget {id}").into())
}

fn inspector_mixed_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, locale)?;
    let media: Vec<_> = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .filter(|item| matches!(item.content, ItemParams::Video(_) | ItemParams::Audio(_)))
        .map(|item| item.id)
        .collect();
    editor.session.apply(Command::Delete { items: media })?;
    let (text_track, first) = {
        let scene = editor.session.project().data().scene();
        let track = scene
            .timeline
            .iter()
            .find(|track| track.kind == TrackKind::Text)
            .ok_or("text track")?;
        (track.id, track.items.first().ok_or("first text item")?.id)
    };
    let second = insert(
        &mut editor.session,
        text_track,
        Tick(6 * SECOND),
        range(6, 8)?,
        None,
        ItemParams::Text(TextParams {
            content: "Khác".into(),
            font_size: 24.0,
            color: Color([20, 30, 40, 255]),
            ..Default::default()
        }),
    )?;
    let main = editor.session.project().data().scene().timeline.main.id;
    let background = insert(
        &mut editor.session,
        main,
        Tick::ZERO,
        range(0, 10)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([35, 45, 65, 255]),
        }),
    )?;
    editor.selection = [first, second, background].into_iter().collect();
    // Setup edits use Session directly, so refresh the preview snapshot before
    // accepting a frame from the populated-editor fixture.
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn inspector_mixed(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_mixed_locale(theme, scale, Locale::En)
}

fn inspector_mixed_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_mixed_locale(theme, scale, Locale::Vi)
}

fn inspector_blend_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let mut data = Project::empty(ProjectId(44), "Blend".into())?.into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.background = Color([120, 160, 200, 255]);
    let mut session = Session::new(Project::new(data)?, 10)?;
    let main = session.project().data().scene().timeline.main.id;
    let item = insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 10)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([200, 80, 120, 128]),
        }),
    )?;
    session.apply(Command::SetItemField {
        item,
        patch: ItemParamPatch::BlendMode(project::BlendMode::Multiply),
    })?;
    session.apply(Command::SetItemField {
        item,
        patch: ItemParamPatch::Opacity(0.5),
    })?;
    let mut playback = Playback::new(&session, ByteBudget::new(64 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    let mut prefs = preferences(theme, locale);
    prefs.set(
        "properties_tabs",
        serde_json::json!({"graphic": "blending"}),
    )?;
    prefs.set("assets_tab", serde_json::json!("effects"))?;
    let mut editor = Editor::new(session, playback, prefs);
    editor.selection.insert(item);
    wait_for_frame(&mut editor)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn inspector_blend(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_blend_locale(theme, scale, Locale::En)
}

fn inspector_blend_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_blend_locale(theme, scale, Locale::Vi)
}
/// Properties scenes render at the reference window size so crops align.
const CAPTURE_WINDOW: [f32; 2] = [1728.0, 1030.0];

/// The capture's column split (34 / 28 %), so the panel is 486 px wide.
fn capture_columns(editor: &mut Editor) -> Result<(), Failure> {
    editor
        .preferences
        .set("panels", serde_json::json!([0.337, 0.28125, 0.45]))?;
    Ok(())
}

fn inspector_video_tab(theme: Theme, scale: f32, tab: &str) -> Result<AppHarness, Failure> {
    let (mut editor, item) = media_editor(
        theme,
        if tab.ends_with("-vi") {
            Locale::Vi
        } else {
            Locale::En
        },
    )?;
    capture_columns(&mut editor)?;
    let tab = tab.strip_suffix("-vi").unwrap_or(tab);
    let kinds_spatial = matches!(
        tab,
        "effects-streak" | "effects-edge-lines" | "effects-relief"
    );
    let colour_picker = tab == "effects-colours-picker";
    let mixer_bottom = tab == "effects-mixer-bottom";
    let tab = if tab.starts_with("effects") {
        let kinds: &[&str] = if tab == "effects-streak" {
            &["directional_blur"]
        } else if tab == "effects-edge-lines" {
            &["find_edges"]
        } else if tab == "effects-relief" {
            &["color_emboss"]
        } else if tab == "effects-threshold" {
            &["extract"]
        } else if tab.starts_with("effects-mixer") {
            &["channel_mix"]
        } else if tab == "effects-tonal" {
            &["black_white", "levels", "gamma_correction", "posterize"]
        } else if tab.starts_with("effects-colours") {
            &["tint", "strobe"]
        } else {
            &["invert", "color_pass", "color_replace"]
        };
        for kind in kinds {
            editor.session.apply(Command::AddEffect {
                item,
                effect: if *kind == "directional_blur" {
                    project::effect_definition(kind)?
                        .instance(*kind)
                        .with("length", project::EffectValue::Scalar(125.0))?
                        .with("direction", project::EffectValue::Scalar(45.0))?
                } else {
                    project::effect_definition(kind)?.instance(*kind)
                },
            })?;
        }
        "effects"
    } else {
        tab
    };
    editor.selection = [item].into_iter().collect();
    editor
        .preferences
        .set("properties_tabs", serde_json::json!({"video": tab}))?;
    let size = if kinds_spatial {
        [1727.0, 1029.0]
    } else {
        CAPTURE_WINDOW
    };
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, size);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    if mixer_bottom {
        app.get(
            By::new().predicate(|node| node.author_id() == Some("effects.channel_mix.param.bc")),
        )
        .scroll_to_me();
        app.run_steps(3);
    }
    if colour_picker {
        app.get(By::new().predicate(|node| node.author_id() == Some("effects.tint.param.black")))
            .click();
        app.run_steps(3);
    }
    Ok(app)
}

fn inspector_video(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_video_tab(theme, scale, "transform")
}

fn inspector_video_blend(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_video_tab(theme, scale, "blending")
}

fn inspector_project(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    capture_columns(&mut editor)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, CAPTURE_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn inspector_text(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    let item = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .ok_or("text item")?
        .id;
    capture_columns(&mut editor)?;
    editor.selection = [item].into_iter().collect();
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, CAPTURE_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn inspector_transform(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = inspector_blend(theme, scale)?;
    if let App::Editor(editor) = &mut app.state_mut().app {
        editor.preferences.set(
            "properties_tabs",
            serde_json::json!({"graphic":"transform"}),
        )?;
    }
    app.run_steps(3);
    Ok(app)
}
fn inspector_color(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = inspector_blend(theme, scale)?;
    if let App::Editor(editor) = &mut app.state_mut().app {
        editor
            .preferences
            .set("properties_tabs", serde_json::json!({"graphic":"graphic"}))?;
        let item = editor
            .selection
            .first()
            .copied()
            .ok_or("colour audit item")?;
        editor.open_color_picker(vibecut::ColorTarget::GraphicFill(item))?;
        editor.set_eyedropper(vibecut::services::color_picker::NativeEyedropper::unavailable())?;
    }
    app.run_steps(3);
    Ok(app)
}

fn import_summary(asset: SourceAsset) -> media::ImportedSource {
    let streams: Vec<_> = asset
        .probe()
        .streams
        .iter()
        .filter(|stream| stream.kind != StreamKind::Other)
        .map(|stream| (stream.index, stream.kind, stream.codec.clone()))
        .collect();
    media::ImportedSource {
        asset,
        capabilities: media::ProbeCapabilities::decodable(streams),
    }
}

fn text_item_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, locale)?;
    let item = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .ok_or("text item")?
        .id;
    editor.selection = [item].into_iter().collect();
    for (field, value) in [
        ("text_content", "VibeCut · Tiếng Việt"),
        ("text_font", "Noto Serif"),
        ("text_background", "#000000A0"),
        ("text_position_y", "70"),
    ] {
        editor.set_shared_field(field, value)?;
    }
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1440.0, 1440.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}

fn text_item(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    text_item_locale(theme, scale, Locale::En)
}

fn text_item_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    text_item_locale(theme, scale, Locale::Vi)
}

/// Returns a deterministic library asset for every path except `bad`, which
/// exercises the real partial-import failure state.
struct CompletedProbe;
impl ImportProbe for CompletedProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<media::ImportedSource> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.contains("bad") {
            return Err(MediaError::Unsupported("audit import refusal"));
        }
        let kind = if name.contains("audio") {
            StreamKind::Audio
        } else {
            StreamKind::Video
        };
        source_asset(id.0, kind, path)
            .map(import_summary)
            .map_err(|_| MediaError::Unsupported("fixture"))
    }
}

fn import_complete(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, locale)?;
    editor.set_import_probe(Arc::new(CompletedProbe))?;
    editor.drop_files(vec![
        std::env::temp_dir().join("video-a.mp4"),
        std::env::temp_dir().join("bad-clip.mp4"),
        std::env::temp_dir().join("audio-a.wav"),
    ])?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.importing() {
        editor.tick();
        if Instant::now() >= deadline {
            return Err("import did not finish".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    editor.select_media([AssetId(1)]);
    let mut app = app_editor(editor, scale);
    app.run_steps(3);
    Ok(app)
}

fn import_complete_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    import_complete(theme, scale, Locale::Vi)
}

fn close_prompt_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, locale)?;
    let directory = tempfile::tempdir()?;
    let mut workspace = Workspace::new(
        directory.path().join("recovery"),
        preferences(theme, locale),
        None,
    )?;
    let id = workspace.register_editor(Box::new(editor));
    workspace
        .windows
        .get_mut(&id)
        .ok_or("close-prompt project window")?
        .request_close();
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    app.run_steps(4);
    Ok(app)
}

fn close_prompt(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    close_prompt_locale(theme, scale, Locale::En)
}

fn close_prompt_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    close_prompt_locale(theme, scale, Locale::Vi)
}

const TRACKS_WINDOW: [f32; 2] = [1728.0, 1200.0];
/// The measured reference editor window, so crops compare at equal logical size.
const EDITOR_WINDOW: [f32; 2] = [1728.0, 1030.0];

/// One long clip on the main track, like the single-track reference capture.
fn timeline_main(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_main_sized(theme, scale, EDITOR_WINDOW)
}

fn timeline_main_sized(theme: Theme, scale: f32, window: [f32; 2]) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Timeline")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(
                id,
                kind,
                std::env::temp_dir().join(format!("source-{id}.mp4")),
            )?,
        })?;
    }
    let main = session.project().data().scene().timeline.main.id;
    for index in 0..4 {
        insert(
            &mut session,
            main,
            Tick(index * 10 * SECOND),
            range(0, 10)?,
            Some(AssetId(1)),
            ItemParams::Video(VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
                crop: None,
            }),
        )?;
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    // 11.24 s puts the playhead 54 px left of the 12 s label, as in the capture.
    let playhead = Tick((11.24 * SECOND as f64) as i64);
    session.apply(Command::SetViewState {
        zoom: 1.7875,
        scroll: 575.0,
        playhead,
    })?;
    let mut editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let project = editor.session.project().clone();
    editor.playback.seek(playhead, false, project)?;
    wait_for_frame(&mut editor)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, window);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

/// The preview axis holding a pointer time over the lanes.
fn preview_axis(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = timeline_main(theme, scale)?;
    hover_preview_axis(&mut app)?;
    Ok(app)
}

/// The same state at the cited web capture's 1440×900 logical size.
fn preview_axis_reference(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = timeline_main_sized(theme, scale, WINDOW)?;
    hover_preview_axis(&mut app)?;
    Ok(app)
}

fn hover_preview_axis(app: &mut AppHarness) -> Result<(), Failure> {
    let ruler = widget_rect(app, "timeline.ruler")?;
    let zoom = app.state().inspect()?.zoom;
    app.hover_at(egui::pos2(ruler.left() + 4.0 * zoom, ruler.center().y));
    app.run_steps(2);
    wait_for_presented(app)?;
    Ok(())
}

/// Layer-end and global-tail evidence at the web reference's logical size.
fn preview_axis_ends(theme: Theme, scale: f32, at: Tick) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Preview axis")?;
    let main = session.project().data().scene().timeline.main.id;
    insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 2)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([255, 0, 0, 255]),
        }),
    )?;
    let overlay = track(&mut session, "Overlay", TrackKind::Video)?;
    let blue = insert(
        &mut session,
        overlay,
        Tick::ZERO,
        range(0, 4)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([0, 0, 255, 255]),
        }),
    )?;
    let playhead = Tick(SECOND);
    session.apply(Command::SetViewState {
        zoom: 1.0,
        scroll: 0.0,
        playhead,
    })?;
    let mut playback = Playback::new(&session, ByteBudget::new(64 << 20)?, Arc::new(Files))?;
    let mut settings = render::RenderSettings::from_project(session.project());
    settings.placements.insert(
        blue,
        render::Placement {
            destination: render::Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            ..Default::default()
        },
    );
    playback.set_render_settings(settings)?;
    playback.seek(playhead, false, session.project().clone())?;
    let editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, WINDOW);
    app.run_steps(3);
    let ruler = widget_rect(&app, "timeline.ruler")?;
    let zoom = app.state().inspect()?.zoom;
    app.hover_at(egui::pos2(
        ruler.left() + at.seconds() as f32 * zoom,
        ruler.center().y,
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.run_steps(1);
        if app.state().presented_frame() == Some(at) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("axis audit frame timeout".into());
        }
        std::thread::yield_now();
    }
    Ok(app)
}

/// Populated multi-track timeline (video, audio, text) at the reference size.
fn timeline_multi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, Locale::En)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, EDITOR_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

// New bottom-corner design: the reference captures only panel duration fields.
fn timeline_fades(theme: Theme, scale: f32, state: &str) -> Result<AppHarness, Failure> {
    let locale = if state == "menu" {
        Locale::Vi
    } else {
        Locale::En
    };
    let (mut editor, _) = media_editor(theme, locale)?;
    let audio = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Audio(_)))
        .ok_or("missing audio clip")?
        .id;
    for (edge, seconds, curve) in [
        (project::FadeEdge::Out, 3, project::FadeCurve::Exponential),
        (project::FadeEdge::In, 20, project::FadeCurve::EqualPower),
    ] {
        editor.session.apply(Command::SetFade {
            item: audio,
            edge,
            duration: Tick(seconds * SECOND),
            curve,
        })?;
    }
    if state == "split" {
        editor.session.apply(Command::SplitAt {
            item: audio,
            at: Tick(2 * SECOND),
        })?;
    }
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, EDITOR_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    let handle = format!("timeline.item.{}.fade_in", audio.0);
    if state == "menu" {
        app.get(By::new().predicate(|node| node.author_id() == Some(handle.as_str())))
            .click_secondary();
    } else if state == "cancelled" {
        let entry = app
            .state()
            .session()
            .ok_or("missing editor")?
            .project()
            .clone();
        let from = widget_rect(&app, &handle)?.center();
        let zoom = app.state().inspect()?.zoom;
        app.drag_at(from);
        app.run_steps(1);
        app.hover_at(egui::pos2(from.x - zoom, from.y));
        app.run_steps(2);
        app.key_press(egui::Key::Escape);
        app.run_steps(2);
        app.drop_at(from);
        app.run_steps(2);
        assert_eq!(
            app.state().session().ok_or("missing editor")?.project(),
            &entry
        );
    }
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn tracks(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Tracks")?;
    for kind in [
        TrackKind::Video,
        TrackKind::Text,
        TrackKind::Sticker,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
        TrackKind::Audio,
    ] {
        track(&mut session, &format!("{kind:?}"), kind)?;
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, TRACKS_WINDOW);
    app.run_steps(4);
    Ok(app)
}

fn scene_create(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = app_editor(empty_editor(theme, Locale::En)?, scale);
    app.run_steps(3);
    for id in ["scene.open", "scene.create"] {
        app.query_all(By::new().predicate(|node| node.author_id() == Some(id)))
            .next()
            .ok_or_else(|| format!("missing automation id {id}"))?
            .click();
        app.run_steps(3);
    }
    Ok(app)
}

fn scene_rename(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = scene_create(theme, scale)?;
    let created = app
        .state()
        .session()
        .ok_or("scene rename requires an editor")?
        .project()
        .data()
        .current_scene_id;
    let row = format!("scene.row.{}", created.0);
    app.query_all(By::new().predicate(|node| node.author_id() == Some(row.as_str())))
        .next()
        .ok_or("missing created scene row")?
        .click_secondary();
    app.run_steps(3);
    let rename = format!("scene.rename.{}", created.0);
    app.query_all(By::new().predicate(|node| node.author_id() == Some(rename.as_str())))
        .next()
        .ok_or("missing scene rename action")?
        .click();
    app.run_steps(3);
    Ok(app)
}

fn scene_delete(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Scenes")?;
    let mut ids = Vec::new();
    for name in ["Opening", "Closing"] {
        let created = session.apply(Command::CreateScene { name: name.into() })?;
        if let Some(CreatedId::Scene(id)) = created.created.first() {
            ids.push(*id);
        }
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, TRACKS_WINDOW);
    app.run_steps(3);
    for id in ["scene.open", "scene.select"] {
        app.query_all(By::new().predicate(|node| node.author_id() == Some(id)))
            .next()
            .ok_or_else(|| format!("missing automation id {id}"))?
            .click();
        app.run_steps(3);
    }
    for id in ids {
        let row = format!("scene.row.{}", id.0);
        app.query_all(By::new().predicate(|node| node.author_id() == Some(row.as_str())))
            .next()
            .ok_or_else(|| format!("missing automation id {row}"))?
            .click();
        app.run_steps(2);
    }
    app.query_all(By::new().predicate(|node| node.author_id() == Some("scene.delete")))
        .next()
        .ok_or("missing automation id scene.delete")?
        .click();
    app.run_steps(3);
    if app
        .query_by_label("Delete selected scenes? You can undo this action.")
        .is_none()
    {
        return Err("scene delete confirmation did not open".into());
    }
    Ok(app)
}

fn placement(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Placement")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(
                id,
                kind,
                std::env::temp_dir().join(format!("source-{id}.mp4")),
            )?,
        })?;
    }
    let video = track(&mut session, "Video", TrackKind::Video)?;
    let moving = insert(
        &mut session,
        video,
        Tick::ZERO,
        range(0, 6)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            crop: None,
            muted: false,
        }),
    )?;
    insert(
        &mut session,
        video,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            crop: None,
            volume: 0.0,
            muted: false,
        }),
    )?;
    let audio = track(&mut session, "Audio", TrackKind::Audio)?;
    insert(
        &mut session,
        audio,
        Tick::ZERO,
        range(0, 10)?,
        Some(AssetId(2)),
        ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
        }),
    )?;
    let text = track(&mut session, "Text", TrackKind::Text)?;
    insert(
        &mut session,
        text,
        Tick(2 * SECOND),
        range(2, 5)?,
        None,
        ItemParams::Text(TextParams {
            content: "Chapter one".into(),
            font_size: 48.0,
            color: Color([255, 255, 255, 255]),
            ..Default::default()
        }),
    )?;
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, TRACKS_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    let body = widget_rect(&app, &format!("timeline.item.{}", moving.0))?;
    let header = widget_rect(&app, &format!("track.{}.header", video.0))?;
    let before = app
        .state()
        .session()
        .ok_or("editor session")?
        .project()
        .data()
        .scene()
        .timeline
        .track_count();
    let target = egui::pos2(body.center().x, header.bottom() - 1.0);
    app.drag_at(body.center());
    app.run_steps(1);
    app.hover_at(target);
    app.run_steps(2);
    app.drop_at(target);
    app.run_steps(2);
    let after = app
        .state()
        .session()
        .ok_or("editor session")?
        .project()
        .data()
        .scene()
        .timeline
        .track_count();
    if after <= before {
        return Err("placement drag did not create a track".into());
    }
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}

/// Blocks every file after the first until the batch is cancelled, so the
/// audit samples the real progress overlay while the job is genuinely running.
struct ProgressProbe {
    calls: AtomicUsize,
    release: (Mutex<bool>, Condvar),
}
impl ProgressProbe {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            release: (Mutex::new(false), Condvar::new()),
        }
    }
}
impl ImportProbe for ProgressProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<media::ImportedSource> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            let mut released = match self.release.0.lock() {
                Ok(released) => released,
                Err(_) => return Err(MediaError::Cancelled),
            };
            while !*released {
                if cancelled.load(Ordering::Acquire) {
                    return Err(MediaError::Cancelled);
                }
                released = match self
                    .release
                    .1
                    .wait_timeout(released, Duration::from_millis(5))
                {
                    Ok((released, _)) => released,
                    Err(_) => return Err(MediaError::Cancelled),
                };
            }
        }
        source_asset(id.0, StreamKind::Video, path)
            .map(import_summary)
            .map_err(|_| MediaError::Unsupported("fixture"))
    }
}

fn export_dialog(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.apply(Command::SetCanvas {
        width: 360,
        height: 640,
        preset: None,
    })?;
    editor.activate("file.export")?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(4);
    Ok(app)
}

fn export_resolution(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = export_dialog(theme, scale)?;
    if let App::Editor(editor) = &mut app.state_mut().app {
        editor.preferences.export.output.resolution = Some([1440, 2560]);
    }
    app.run_steps(2);
    app.get(By::new().predicate(|node| node.author_id() == Some("export.resolution")))
        .click();
    app.run_steps(20);
    Ok(app)
}

fn import_progress(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, Locale::En)?;
    editor.set_import_probe(Arc::new(ProgressProbe::new()))?;
    editor.drop_files(vec![
        std::env::temp_dir().join("one.mp4"),
        std::env::temp_dir().join("two.mp4"),
        std::env::temp_dir().join("three.mp4"),
    ])?;
    let mut app = app_editor(editor, scale);
    for _ in 0..500 {
        app.run_steps(1);
        let inspection = app.state().inspect()?;
        if inspection.import_completed > 0 {
            break;
        }
        if !inspection.importing {
            return Err("import finished before the audit could sample progress".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    let inspection = app.state().inspect()?;
    if !inspection.importing {
        return Err("import finished before the audit could render progress".into());
    }
    Ok(app)
}

const README_WINDOW: [f32; 2] = [1440.0, 900.0];
const README_GIF_FRAMES: usize = 80;
const README_MAX_GIF: u64 = 5_000_000;
const README_MAX_JPG: u64 = 512 * 1024;
const README_SCRATCH: u64 = 8 << 30;
const BBB_SOURCE_URL: &str = "https://www.peach.themazzone.com/big_buck_bunny_720p_h264.mov";
const BBB_SOURCE_BYTES: u64 = 416_751_190;
const BBB_SOURCE_SHA256: &str = "45c8bafeb9a53df7f491198d2e71529701bcf1cd51805782089fac1d32869f9b";
const BBB_EXCERPT_BYTES: u64 = 4_544_843;
const BBB_EXCERPT_SHA256: &str = "66518061bbabbde0c7d48b639675ce8c46a549182d32c3cc1203f2cded7c21a1";
const BBB_CREDIT: &str = "(c) copyright 2008, Blender Foundation / www.bigbuckbunny.org";
const TOOL_METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const TOOL_WORK_TIMEOUT: Duration = Duration::from_secs(120);
const TOOL_STDOUT_CAP: usize = 16 << 20;
const TOOL_STDERR_CAP: usize = 64 << 10;

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").ok();
    }
    out
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

struct ReadmeClock(AtomicU64);

impl engine::jobs::Clock for ReadmeClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn stream_sha256(path: &Path) -> Result<String, Failure> {
    let metadata = fs::metadata(path)?;
    if metadata.len() != BBB_EXCERPT_BYTES {
        return Err(format!(
            "README source is not the admitted Big Buck Bunny excerpt: expected {BBB_EXCERPT_BYTES} bytes, found {}",
            metadata.len()
        )
        .into());
    }
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 1 << 16];
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > BBB_EXCERPT_BYTES {
            return Err("README source grew beyond the admitted excerpt".into());
        }
        hasher.update(&buffer[..read]);
    }
    if total != BBB_EXCERPT_BYTES {
        return Err("README source ended before the admitted excerpt".into());
    }
    let digest = hex(&hasher.finalize());
    if digest != BBB_EXCERPT_SHA256 {
        return Err(format!(
            "README source SHA-256 mismatch: expected {BBB_EXCERPT_SHA256}, found {digest}"
        )
        .into());
    }
    Ok(digest)
}

#[derive(Debug)]
struct SupervisedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn child(&mut self) -> Result<&mut Child, Failure> {
        self.0
            .as_mut()
            .ok_or_else(|| "supervised child missing".into())
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

fn read_capped(path: &Path, cap: usize, label: &str) -> Result<Vec<u8>, Failure> {
    let mut file = fs::File::open(path)?;
    let mut out = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let room = (cap + 1).saturating_sub(out.len());
        if room > 0 {
            out.extend_from_slice(&buffer[..read.min(room)]);
        }
        if out.len() > cap {
            return Err(format!("{label} output exceeded its bounded cap").into());
        }
    }
    Ok(out)
}

fn tool_command(program: &str) -> ProcessCommand {
    let mut command = ProcessCommand::new(program);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    if let (Some(prefix), Some(paths)) = (std::env::var_os("FFMPEG_DIR"), std::env::var_os("PATH"))
    {
        let production = PathBuf::from(prefix).join("bin");
        let paths: Vec<_> = std::env::split_paths(&paths)
            .filter(|path| path != &production)
            .collect();
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    command
}

fn run_tool(
    program: &str,
    args: &[&str],
    timeout: Duration,
    stdout_cap: usize,
) -> Result<SupervisedOutput, Failure> {
    let scratch = tempfile::tempdir()?;
    let out_path = scratch.path().join("stdout");
    let err_path = scratch.path().join("stderr");
    let out_file = fs::File::create(&out_path)?;
    let err_file = fs::File::create(&err_path)?;
    let child = tool_command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|error| format!("spawn {program}: {error}"))?;
    let mut guard = ChildGuard(Some(child));
    let deadline = Instant::now() + timeout;
    let mut status = None;
    while Instant::now() < deadline {
        if let Some(exit) = guard.child()?.try_wait()? {
            status = Some(exit);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let timed_out = status.is_none();
    if timed_out {
        guard.kill();
    }
    let stdout = read_capped(&out_path, stdout_cap, program)?;
    let stderr = read_capped(&err_path, TOOL_STDERR_CAP, program)?;
    if timed_out {
        return Err(format!("{program} timed out after {timeout:?}").into());
    }
    guard.0.take();
    Ok(SupervisedOutput {
        status: status.ok_or("supervised child status")?,
        stdout,
        stderr,
    })
}

fn tool_identity(tool: &str) -> Result<String, Failure> {
    let output = run_tool(tool, &["-version"], TOOL_METADATA_TIMEOUT, TOOL_STDERR_CAP)?;
    if !output.status.success() {
        return Err(format!("{tool} -version failed").into());
    }
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .next()
        .unwrap_or("")
        .to_owned())
}

fn git_head(root: &Path) -> Result<String, Failure> {
    let root = root.to_string_lossy().into_owned();
    let output = run_tool(
        "git",
        &["-C", &root, "rev-parse", "HEAD"],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn free_bytes(path: &Path) -> Result<u64, Failure> {
    let path = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    let output = run_tool(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "(Get-Item -LiteralPath '{}').PSDrive.Free",
                path.replace('\'', "''")
            ),
        ],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    #[cfg(unix)]
    let output = run_tool(
        "df",
        &["-k", "-P", &path],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("scratch headroom probe failed".into());
    }
    let text = String::from_utf8(output.stdout)?;
    #[cfg(windows)]
    let bytes = text.trim().parse::<u64>()?;
    #[cfg(unix)]
    let bytes = text
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|field| field.parse::<u64>().ok())
        .map(|kib| kib.saturating_mul(1024))
        .ok_or("df free space parse")?;
    Ok(bytes)
}

fn ffmpeg(args: &[&str]) -> Result<(), Failure> {
    let mut all = vec!["-hide_banner", "-loglevel", "error", "-y"];
    all.extend_from_slice(args);
    let output = run_tool("ffmpeg", &all, TOOL_WORK_TIMEOUT, TOOL_STDOUT_CAP)?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        )
        .into());
    }
    Ok(())
}

fn decode_rgb(path: &Path, filter: Option<&str>) -> Result<Vec<u8>, Failure> {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["-hide_banner", "-loglevel", "error", "-i", path.as_str()];
    if let Some(filter) = filter {
        args.extend(["-vf", filter]);
    }
    args.extend(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    let output = run_tool("ffmpeg", &args, TOOL_WORK_TIMEOUT, TOOL_STDOUT_CAP)?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg decode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

fn probe_field(path: &Path, select: &str, entry: &str) -> Result<String, Failure> {
    let path_value = path.to_string_lossy().into_owned();
    let entry = format!("stream={entry}");
    let output = run_tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            select,
            "-show_entries",
            &entry,
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            &path_value,
        ],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("ffprobe field failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn decoded_frame_count(path: &Path) -> Result<u64, Failure> {
    let path_value = path.to_string_lossy().into_owned();
    let output = run_tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            &path_value,
        ],
        TOOL_WORK_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("ffprobe frame count failed".into());
    }
    String::from_utf8(output.stdout)?
        .trim()
        .parse::<u64>()
        .map_err(|_| "ffprobe frame count parse".into())
}

fn image_stats(pixels: &[u8]) -> serde_json::Value {
    let channels = pixels.len() / 3;
    if channels == 0 {
        return serde_json::json!({"mean": 0.0, "min": 0, "max": 0, "spread": 0});
    }
    let sum: u64 = pixels.iter().map(|v| u64::from(*v)).sum();
    let min = pixels.iter().copied().min().unwrap_or(0);
    let max = pixels.iter().copied().max().unwrap_or(0);
    serde_json::json!({
        "mean": sum as f64 / pixels.len() as f64,
        "min": min,
        "max": max,
        "spread": max - min
    })
}

fn difference(left: &[u8], right: &[u8]) -> serde_json::Value {
    let mut max = 0u8;
    let mut sum = 0u64;
    for (a, b) in left.iter().zip(right) {
        let delta = a.abs_diff(*b);
        max = max.max(delta);
        sum += u64::from(delta);
    }
    serde_json::json!({
        "mean_abs": sum as f64 / left.len().max(1) as f64,
        "max_abs": max
    })
}

fn publish(src: &Path, dst: &Path) -> Result<(), Failure> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    fs::remove_file(src).ok();
    Ok(())
}

fn owned_by_runner(outputs: &Path, name: &str, slot: &str) -> Result<bool, Failure> {
    let media = outputs.join(name);
    let sidecar = outputs.join(format!("{name}.attribution"));
    if !media.exists() && !sidecar.exists() {
        return Ok(false);
    }
    let text = fs::read_to_string(&sidecar).map_err(|_| {
        format!("refusing unrelated existing README output `{name}`: no runner sidecar")
    })?;
    let marker = format!("ui_audit.rs --readme {slot}");
    Ok(text
        .lines()
        .any(|line| line.trim() == format!("asset: {name}"))
        && text.lines().any(|line| line.trim() == "added: p0-core/22")
        && text
            .lines()
            .any(|line| line.starts_with("source:") && line.contains(&marker)))
}

#[derive(Debug)]
struct Publication {
    outputs: PathBuf,
    backup: PathBuf,
    published: Vec<String>,
    backed: Vec<String>,
    run: PathBuf,
}

impl Publication {
    fn commit(self) {
        fs::remove_dir_all(&self.backup).ok();
    }

    fn rollback(self, error: Failure) -> Failure {
        let mut rollback = Vec::new();
        for name in self.published.iter().rev() {
            let path = self.outputs.join(name);
            if path.exists() && fs::remove_file(&path).is_err() {
                rollback.push(format!("remove {name}"));
            }
        }
        for name in self.backed.iter().rev() {
            if fs::rename(self.backup.join(name), self.outputs.join(name)).is_err() {
                rollback.push(format!("restore {name}"));
            }
        }
        if rollback.is_empty() {
            fs::remove_dir_all(&self.backup).ok();
            error
        } else {
            fs::write(
                self.run.join("preserve-staging"),
                format!("rollback incomplete: {}", rollback.join(", ")),
            )
            .ok();
            format!(
                "{error}; rollback incomplete ({}) with backups at {}",
                rollback.join(", "),
                self.backup.display()
            )
            .into()
        }
    }
}

fn publish_outputs(
    stage: &Path,
    outputs: &Path,
    slot: &str,
    media_names: &[&str],
    all_names: &[&str],
    run: &Path,
    run_id: &str,
) -> Result<Publication, Failure> {
    for name in media_names {
        let exists =
            outputs.join(name).exists() || outputs.join(format!("{name}.attribution")).exists();
        if exists && !owned_by_runner(outputs, name, slot)? {
            return Err(format!(
                "refusing unrelated existing README output `{name}`: sidecar does not prove runner ownership"
            )
            .into());
        }
    }
    let backup = outputs.join(format!(".readme-media-backup-{run_id}"));
    if backup.exists() {
        return Err("README publication backup already exists".into());
    }
    fs::create_dir_all(&backup)?;
    let mut publication = Publication {
        outputs: outputs.to_owned(),
        backup: backup.clone(),
        published: Vec::new(),
        backed: Vec::new(),
        run: run.to_owned(),
    };
    let mut apply = || -> Result<(), Failure> {
        for name in all_names {
            let src = stage.join(name);
            if !src.is_file() {
                return Err(format!("staged README output `{name}` is missing").into());
            }
            let dst = outputs.join(name);
            if dst.exists() {
                fs::rename(&dst, backup.join(name))?;
                publication.backed.push((*name).to_owned());
            }
            let temp = outputs.join(format!(".{name}.tmp-{run_id}"));
            if temp.exists() {
                fs::remove_file(&temp)?;
            }
            fs::copy(&src, &temp)?;
            fs::rename(&temp, &dst)?;
            publication.published.push((*name).to_owned());
        }
        Ok(())
    };
    match apply() {
        Ok(()) => Ok(publication),
        Err(error) => Err(publication.rollback(error)),
    }
}

fn finish_publication(
    publication: Publication,
    manifest_path: &Path,
    manifest: &[u8],
) -> Result<(), Failure> {
    if let Err(error) = fs::write(manifest_path, manifest) {
        return Err(
            publication.rollback(format!("publication manifest write failed: {error}").into())
        );
    }
    publication.commit();
    Ok(())
}

fn encode_jpg(raw: &Path, out: &Path) -> Result<serde_json::Value, Failure> {
    let name = out.file_name().ok_or("jpg filename")?;
    let temp = out.with_file_name(format!(".{}.tmp.jpg", name.to_string_lossy()));
    let mut bytes = 0;
    for quality in ["2", "4", "6", "8", "12"] {
        ffmpeg(&[
            "-i",
            &raw.to_string_lossy(),
            "-vf",
            "scale=960:-2:flags=lanczos",
            "-frames:v",
            "1",
            "-q:v",
            quality,
            &temp.to_string_lossy(),
        ])?;
        bytes = fs::metadata(&temp)?.len();
        if bytes <= README_MAX_JPG {
            break;
        }
    }
    if bytes > README_MAX_JPG {
        return Err(format!("jpg exceeds 512 KiB: {bytes}").into());
    }
    let width = probe_field(&temp, "v:0", "width")?.parse::<u32>()?;
    let height = probe_field(&temp, "v:0", "height")?.parse::<u32>()?;
    if (width, height) != (960, 600) {
        return Err(format!("jpg dimensions {width}x{height}").into());
    }
    let decoded = decode_rgb(&temp, None)?;
    let source = decode_rgb(raw, Some("scale=960:-2:flags=lanczos"))?;
    let stats = image_stats(&decoded);
    if stats["spread"].as_u64().unwrap_or(0) < 16 {
        return Err("jpg is blank".into());
    }
    let loss = difference(&source, &decoded);
    publish(&temp, out)?;
    Ok(serde_json::json!({
        "bytes": bytes,
        "width": width,
        "height": height,
        "decoded_pixels": decoded.len() / 3,
        "stats": stats,
        "source_vs_decoded": loss
    }))
}

fn gif_timing(path: &Path) -> Result<(usize, f64, f64), Failure> {
    let path_value = path.to_string_lossy().into_owned();
    let output = run_tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=pts_time,duration_time",
            "-of",
            "csv=p=0",
            &path_value,
        ],
        TOOL_METADATA_TIMEOUT,
        1 << 20,
    )?;
    if !output.status.success() {
        return Err("gif timing probe failed".into());
    }
    let mut frames = 0;
    let mut max_delay_error = 0.0f64;
    let mut end = 0.0f64;
    for line in String::from_utf8(output.stdout)?.lines() {
        let mut fields = line.split(',');
        let Some(pts) = fields.next().and_then(|v| v.trim().parse::<f64>().ok()) else {
            continue;
        };
        let delay = fields
            .next()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
        frames += 1;
        max_delay_error = max_delay_error.max((delay - 0.1).abs());
        end = pts + delay;
    }
    Ok((frames, max_delay_error, end))
}

fn encode_gif(run: &Path, out: &Path) -> Result<serde_json::Value, Failure> {
    let pattern = run.join("frame-%05d.png");
    let pattern = pattern.to_str().ok_or("frame pattern path")?;
    let temp = run.join("timeline.gif");
    let scale = "scale=960:-2:flags=lanczos";
    ffmpeg(&[
        "-framerate",
        "10",
        "-i",
        pattern,
        "-frames:v",
        "80",
        "-vf",
        scale,
        "-loop",
        "0",
        &temp.to_string_lossy(),
    ])?;
    let mut palette = false;
    if fs::metadata(&temp)?.len() > README_MAX_GIF {
        palette = true;
        let palette_file = run.join("palette.png");
        ffmpeg(&[
            "-framerate",
            "10",
            "-i",
            pattern,
            "-vf",
            &format!("{scale},palettegen=stats_mode=diff"),
            &palette_file.to_string_lossy(),
        ])?;
        ffmpeg(&[
            "-framerate",
            "10",
            "-i",
            pattern,
            "-i",
            &palette_file.to_string_lossy(),
            "-lavfi",
            &format!("{scale}[x];[x][1:v]paletteuse=dither=sierra2_4a"),
            "-loop",
            "0",
            &temp.to_string_lossy(),
        ])?;
    }
    let bytes = fs::metadata(&temp)?.len();
    if bytes > README_MAX_GIF {
        return Err(format!("gif exceeds 5,000,000 bytes: {bytes}").into());
    }
    let width = probe_field(&temp, "v:0", "width")?.parse::<u32>()?;
    let height = probe_field(&temp, "v:0", "height")?.parse::<u32>()?;
    let frames = decoded_frame_count(&temp)?;
    let (_, delay_error, end) = gif_timing(&temp)?;
    if (width, height) != (960, 600) || frames != README_GIF_FRAMES as u64 {
        return Err(format!("gif geometry/frames {width}x{height} {frames}").into());
    }
    if delay_error > 0.001 || (end - 8.0).abs() > 0.05 {
        return Err(format!("gif timing delay_error={delay_error} end={end}").into());
    }
    let decoded = decode_rgb(&temp, Some("select=eq(n\\,40)"))?;
    let source = decode_rgb(
        &run.join("boundary-2.png"),
        Some("scale=960:-2:flags=lanczos"),
    )?;
    let stats = image_stats(&decoded);
    if stats["spread"].as_u64().unwrap_or(0) < 16 {
        return Err("gif frame is blank".into());
    }
    let loss = difference(&source, &decoded);
    ffmpeg(&["-i", &temp.to_string_lossy(), "-f", "null", "-"])?;
    publish(&temp, out)?;
    Ok(serde_json::json!({
        "bytes": bytes,
        "width": width,
        "height": height,
        "frames": frames,
        "frame_delay_seconds": 0.1,
        "duration_seconds": end,
        "palette_optimized": palette,
        "max_delay_error": delay_error,
        "decoded_pixels": decoded.len() / 3,
        "stats": stats,
        "source_vs_decoded": loss
    }))
}

fn readme_editor(app: &mut AppHarness) -> Result<&mut Editor, Failure> {
    match &mut app.state_mut().app {
        App::Editor(editor) => Ok(editor),
        _ => Err("README capture requires an editor".into()),
    }
}

fn readme_frame(app: &mut AppHarness, at: Tick) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = {
            let editor = readme_editor(app)?;
            editor.playback.poll(Instant::now())?;
            editor.playback.frame().is_some_and(|frame| {
                frame.generation == editor.playback.generation() && frame.at == at
            })
        };
        if ready {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("README preview frame {} timed out", at.0).into());
        }
        app.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.run_steps(1);
        if app.state().presented_frame() == Some(at) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("README preview frame {} not presented", at.0).into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(app)?;
    if app.state().presented_frame() != Some(at) {
        return Err("settled README frame changed".into());
    }
    Ok(())
}

fn readme_seek(app: &mut AppHarness, at: Tick) -> Result<(), Failure> {
    {
        let editor = readme_editor(app)?;
        let project = editor.session.project().clone();
        editor.playback.seek(at, false, project)?;
    }
    readme_frame(app, at)
}

fn readme_capture(app: &mut AppHarness, path: &Path) -> Result<(), Failure> {
    app.run_steps(2);
    let image = app.render().map_err(std::io::Error::other)?;
    let image = opaque(image, vibecut::window_color(true));
    image.save(path)?;
    Ok(())
}

fn readme_frame_digest(app: &mut AppHarness) -> Result<String, Failure> {
    let editor = readme_editor(app)?;
    let frame = editor.playback.frame().ok_or("missing README frame")?;
    let mut hasher = Sha256::new();
    for pixel in &frame.image.pixels {
        hasher.update(pixel.to_array());
    }
    Ok(hex(&hasher.finalize()))
}

fn readme_project(source: &Path) -> Result<(Editor, ItemId, ItemId), Failure> {
    let mut data = Project::empty(ProjectId(245), "README media".into())?.into_data();
    data.canvas.width = 1280;
    data.canvas.height = 720;
    data.canvas.fps = FrameRate::FPS_30;
    let session = Session::new(Project::new(data)?, 100)?;
    let mut preferences = preferences(Theme::Dark, Locale::En);
    preferences.assets_tab = vibecut::panels::AssetsTab::Media;
    preferences.subtitles.source = "vi".into();
    preferences.subtitles.target = "en".into();
    preferences.subtitles.bilingual = true;
    let playback = Playback::new(&session, ByteBudget::new(1 << 30)?, Arc::new(Files))?;
    let mut editor = Editor::new(session, playback, preferences);
    let clock = Arc::new(ReadmeClock(AtomicU64::new(0)));
    editor.set_clock(clock.clone());
    editor.drop_files(vec![source.to_owned()])?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while editor.importing() {
        editor.tick();
        if Instant::now() >= deadline {
            return Err("README media import timed out".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    clock.0.store(6_000, Ordering::SeqCst);
    editor.tick();
    let canonical = std::fs::canonicalize(source)?;
    let asset = editor
        .session
        .project()
        .data()
        .assets
        .iter()
        .find(|asset| asset.path() == canonical)
        .cloned()
        .ok_or("README media import produced no asset")?;
    let probe = asset.probe();
    let video = probe
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())
        .ok_or("README media has no video probe")?;
    if (video.width, video.height) != (1280, 720) {
        return Err(format!("README source probe {}x{}", video.width, video.height).into());
    }
    let asset_id = asset.id();
    let main = editor.session.project().data().scene().timeline.main.id;
    let first = insert(
        &mut editor.session,
        main,
        Tick::ZERO,
        range(0, 6)?,
        Some(asset_id),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    )?;
    let second = insert(
        &mut editor.session,
        main,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(asset_id),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    )?;
    let audio = track(&mut editor.session, "Audio", TrackKind::Audio)?;
    insert(
        &mut editor.session,
        audio,
        Tick::ZERO,
        range(0, 10)?,
        Some(asset_id),
        ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
        }),
    )?;
    let text = track(&mut editor.session, "Text", TrackKind::Text)?;
    insert(
        &mut editor.session,
        text,
        Tick(2 * SECOND),
        range(2, 6)?,
        None,
        ItemParams::Text(TextParams {
            content: "Chapter one".into(),
            font_size: 52.0,
            color: Color([255, 255, 255, 255]),
            position: project::TextPosition { x: 50.0, y: 70.0 },
            ..Default::default()
        }),
    )?;
    let caption_text = |text: &str| CueText {
        text: text.into(),
        edited: false,
        translation_ref: None,
    };
    let cue = Cue {
        id: "readme-cue".into(),
        range: range(2, 6)?,
        source_lang: LanguageTag::parse("vi")?,
        texts: [
            (LanguageTag::parse("vi")?, caption_text("Chào bạn")),
            (LanguageTag::parse("en")?, caption_text("Hello friend")),
        ]
        .into_iter()
        .collect(),
        word_ranges: Vec::new(),
        style: None,
    };
    let captions = track(&mut editor.session, "Captions", TrackKind::Text)?;
    insert(
        &mut editor.session,
        captions,
        Tick::ZERO,
        range(0, 10)?,
        Some(asset_id),
        ItemParams::Caption(CaptionParams {
            cues: CueTrack {
                styles: Vec::new(),
                cues: vec![cue],
            },
        }),
    )?;
    let mut settings = render::RenderSettings::from_project(editor.session.project());
    settings.caption_languages = vec![LanguageTag::parse("vi")?, LanguageTag::parse("en")?];
    editor.playback.set_render_settings(settings)?;
    editor.selection = [first].into_iter().collect();
    Ok((editor, first, second))
}

fn readme_boundary(
    app: &mut AppHarness,
    run: &Path,
    index: usize,
    label: &str,
) -> Result<serde_json::Value, Failure> {
    readme_seek(app, Tick(3 * SECOND))?;
    let raw = run.join(format!("boundary-{index}.png"));
    readme_capture(app, &raw)?;
    let editor = readme_editor(app)?;
    let project = editor.session.project();
    let snapshot = project::encode(project, false)?;
    let project_hash = sha256_bytes(&snapshot);
    let snapshot_name = format!("boundary-{index}.project");
    fs::write(run.join(&snapshot_name), &snapshot)?;
    let revision = serde_json::to_string(&editor.session.revision())?;
    let generation = editor.playback.generation();
    let frame_at = editor.playback.frame().map(|frame| frame.at.0);
    let presented = editor.presented_frame().map(|tick| tick.0);
    Ok(serde_json::json!({
        "boundary": index,
        "label": label,
        "raw": raw.file_name().map(|name| name.to_string_lossy().into_owned()),
        "snapshot": snapshot_name,
        "snapshot_sha256": project_hash,
        "revision": revision,
        "project_sha256": project_hash,
        "generation": generation,
        "frame_at": frame_at,
        "presented_at": presented
    }))
}

fn verify_snapshots(
    run: &Path,
    first: ItemId,
    split: ItemId,
) -> Result<serde_json::Value, Failure> {
    let load = |index: usize| -> Result<project::ProjectData, Failure> {
        let bytes = fs::read(run.join(format!("boundary-{index}.project")))?;
        Ok(project::decode(&bytes)?.project.into_data())
    };
    let initial = load(0)?;
    let split_state = load(1)?;
    let trimmed_state = load(2)?;
    let undone_state = load(3)?;
    if undone_state != split_state {
        return Err("undo snapshot does not equal the split snapshot".into());
    }
    let split_right = split_state
        .scene()
        .timeline
        .main
        .items
        .iter()
        .find(|item| item.id == split)
        .ok_or("split snapshot has no right item")?
        .clone();
    let mut trimmed_reverted = trimmed_state.clone();
    {
        let right = trimmed_reverted
            .scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|item| item.id == split)
            .ok_or("trim snapshot has no right item")?;
        right.source_range = split_right.source_range;
        right.retime = split_right.retime;
    }
    if trimmed_reverted != split_state {
        return Err("trim snapshot changed more than the right item range/retime".into());
    }
    let initial_first = initial
        .scene()
        .timeline
        .main
        .items
        .iter()
        .find(|item| item.id == first)
        .ok_or("initial snapshot has no first item")?
        .clone();
    let mut split_reverted = split_state.clone();
    split_reverted
        .scene_mut()
        .timeline
        .main
        .items
        .retain(|item| item.id != split);
    {
        let left = split_reverted
            .scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|item| item.id == first)
            .ok_or("split snapshot has no first item")?;
        left.source_range = initial_first.source_range;
        left.retime = initial_first.retime;
    }
    if split_reverted != initial {
        return Err(
            "split snapshot changed more than the first item range/retime and new right item"
                .into(),
        );
    }
    Ok(serde_json::json!({
        "undo_matches_split": true,
        "trim_only_right_range": true,
        "split_only_first_range_and_new_item": true
    }))
}

fn unique_run_dir(target: &Path, slot: &str) -> Result<PathBuf, Failure> {
    let root = target.join("readme-media").join(slot);
    fs::create_dir_all(&root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock before epoch")?
        .as_millis();
    for attempt in 0..1000 {
        let run = root.join(format!("{stamp}-{}-{attempt}", std::process::id()));
        if !run.exists() {
            fs::create_dir(&run)?;
            return Ok(run);
        }
    }
    Err("could not allocate a unique README run directory".into())
}

fn readme_media(root: &Path, target: &Path, slot: &str, source: &Path) -> Result<(), Failure> {
    if !matches!(slot, "hero" | "timeline") {
        return Err(format!("unknown README slot `{slot}`; expected hero or timeline").into());
    }
    let source = std::fs::canonicalize(source).map_err(|error| {
        format!(
            "README source `{}` is not readable: {error}",
            source.display()
        )
    })?;
    if !source.is_file() {
        return Err("README source must be a local file".into());
    }
    let source_hash = stream_sha256(&source)?;
    let tools = [tool_identity("ffmpeg")?, tool_identity("ffprobe")?];
    let free = free_bytes(target)?;
    if free < README_SCRATCH {
        return Err(format!(
            "README capture needs at least 8 GiB scratch headroom; found {free} bytes"
        )
        .into());
    }
    let run = unique_run_dir(target, slot)?;
    let result = readme_media_run(root, slot, &source, &run, &source_hash, &tools);
    if result.is_err() && !run.join("preserve-staging").exists() {
        fs::remove_dir_all(&run).ok();
    }
    result
}

fn readme_media_run(
    root: &Path,
    slot: &str,
    source: &Path,
    run: &Path,
    source_hash: &str,
    tools: &[String; 2],
) -> Result<(), Failure> {
    let commit = git_head(root)?;
    let (editor, first, _second) = readme_project(source)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), 1.0, README_WINDOW);
    readme_seek(&mut app, Tick(3 * SECOND))?;
    let at_three = readme_frame_digest(&mut app)?;
    readme_seek(&mut app, Tick(SECOND))?;
    let at_one = readme_frame_digest(&mut app)?;
    if at_three == at_one {
        return Err("decoded README source frames are identical".into());
    }
    let editor = readme_editor(&mut app)?;
    let project = editor.session.project();
    let asset = project
        .data()
        .assets
        .first()
        .ok_or("README project lost its asset")?;
    let probe = serde_json::to_value(asset.probe())?;
    let source_relative = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "excerpt.mp4".into());
    let outputs = root.join("docs/assets/readme");
    fs::create_dir_all(&outputs)?;
    let stage = run.join("stage");
    fs::create_dir_all(&stage)?;
    let mut manifest = serde_json::json!({
        "slot": slot,
        "command": format!("cargo xtask build -- cargo xtask ui-audit --readme {slot} --source <local Big Buck Bunny excerpt>"),
        "source_commit": commit,
        "source": {
            "kind": "Big Buck Bunny (2008, Blender Foundation)",
            "credit": BBB_CREDIT,
            "source_page": "https://peach.blender.org/download/",
            "license": "CC-BY-3.0",
            "license_url": "https://creativecommons.org/licenses/by/3.0/",
            "license_file": "docs/licenses/CC-BY-3.0.txt",
            "original": {
                "url": BBB_SOURCE_URL,
                "bytes": BBB_SOURCE_BYTES,
                "sha256": BBB_SOURCE_SHA256
            },
            "path": source_relative,
            "sha256": source_hash,
            "probe": probe,
            "excerpt": {
                "start_seconds": 50.0,
                "duration_seconds": 12.0,
                "transform": "fps=30,scale=1280:720:flags=lanczos,setsar=1,libx264 medium crf20 yuv420p gop30 BT.709 TV,movflags faststart,map_metadata -1,no data/subtitles; AAC 160k 48k stereo; original audio retained"
            }
        },
        "tools": tools,
        "decoded_frame_sha256": { "at_1s": at_one, "at_3s": at_three }
    });
    if slot == "hero" {
        readme_seek(&mut app, Tick(3 * SECOND))?;
        let raw = run.join("hero-frame.png");
        readme_capture(&mut app, &raw)?;
        let jpg = stage.join("hero.jpg");
        let jpg_report = encode_jpg(&raw, &jpg)?;
        let details = readme_boundary(&mut app, &run, 0, "settled multitrack project")?;
        manifest["boundary"] = details;
        manifest["outputs"] = serde_json::json!({"hero.jpg": jpg_report});
    } else {
        let boundaries = vec![readme_boundary(
            &mut app,
            &run,
            0,
            "initial multitrack project",
        )?];
        let mut commands = Vec::new();
        let split = {
            let editor = readme_editor(&mut app)?;
            let command = Command::SplitAt {
                item: first,
                at: Tick(4 * SECOND),
            };
            let command_json = serde_json::to_value(&command)?;
            let outcome = editor.session.apply(command)?;
            commands.push(serde_json::json!({
                "boundary": 1,
                "command": command_json,
                "changed": outcome.changed,
                "created": serde_json::to_value(&outcome.created)?,
                "placements": serde_json::to_value(&outcome.placements)?
            }));
            outcome
                .created
                .into_iter()
                .find_map(|created| match created {
                    CreatedId::Item(id) => Some(id),
                    _ => None,
                })
                .ok_or("split created no right item")?
        };
        let other_ranges: Vec<_> = {
            let editor = readme_editor(&mut app)?;
            let first_item = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == first)
                .ok_or("split lost the first item")?;
            if first_item.source_range.start() != Tick::ZERO
                || first_item.source_range.end() != Tick(4 * SECOND)
            {
                return Err("split first source range is not 0..4s".into());
            }
            let right = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == split)
                .ok_or("split right item missing")?;
            if right.start != Tick(4 * SECOND)
                || right.source_range.start() != Tick(4 * SECOND)
                || right.source_range.end() != Tick(6 * SECOND)
            {
                return Err("split right source range is not 4..6s".into());
            }
            editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| track.items.iter())
                .filter(|item| item.id != first && item.id != split)
                .map(|item| (item.id, item.source_range, item.start))
                .collect()
        };
        let boundaries = {
            let mut all = boundaries;
            all.push(readme_boundary(&mut app, &run, 1, "split at 4s")?);
            all
        };
        let boundaries = {
            let editor = readme_editor(&mut app)?;
            let command = Command::TimelineTrim {
                items: vec![split],
                edge: TrimEdge::End,
                delta: Tick(-SECOND / 2),
            };
            let command_json = serde_json::to_value(&command)?;
            let outcome = editor.session.apply(command)?;
            commands.push(serde_json::json!({
                "boundary": 2,
                "command": command_json,
                "changed": outcome.changed,
                "created": serde_json::to_value(&outcome.created)?,
                "placements": serde_json::to_value(&outcome.placements)?
            }));
            let right = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == split)
                .ok_or("trim lost the right item")?;
            if right.source_range.end() != Tick(5 * SECOND + SECOND / 2) {
                return Err("trim did not remove 0.5s from the right end".into());
            }
            for (id, source_range, start) in &other_ranges {
                let item = editor
                    .session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| track.items.iter())
                    .find(|item| item.id == *id)
                    .ok_or("trim lost another item")?;
                if item.source_range != *source_range || item.start != *start {
                    return Err("trim changed another track/item".into());
                }
            }
            let mut all = boundaries;
            all.push(readme_boundary(
                &mut app,
                &run,
                2,
                "right end trimmed by 0.5s",
            )?);
            all
        };
        let boundaries = {
            let editor = readme_editor(&mut app)?;
            let command = Command::Undo {};
            let command_json = serde_json::to_value(&command)?;
            let outcome = editor.session.apply(command)?;
            commands.push(serde_json::json!({
                "boundary": 3,
                "command": command_json,
                "changed": outcome.changed,
                "created": serde_json::to_value(&outcome.created)?,
                "placements": serde_json::to_value(&outcome.placements)?
            }));
            let first_item = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == first)
                .ok_or("undo lost the first item")?;
            let right = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .find(|item| item.id == split)
                .ok_or("undo lost the split item")?;
            if first_item.source_range.end() != Tick(4 * SECOND)
                || right.source_range.start() != Tick(4 * SECOND)
                || right.source_range.end() != Tick(6 * SECOND)
            {
                return Err("undo did not restore the split state".into());
            }
            let mut all = boundaries;
            all.push(readme_boundary(
                &mut app,
                &run,
                3,
                "undo restores the split",
            )?);
            all
        };
        let mut frames = Vec::with_capacity(README_GIF_FRAMES);
        for index in 0..README_GIF_FRAMES {
            let boundary = index / 20;
            let source = run.join(format!("boundary-{boundary}.png"));
            let frame = run.join(format!("frame-{index:05}.png"));
            fs::copy(&source, &frame)?;
            let mut entry = boundaries.get(boundary).cloned().unwrap_or_default();
            entry["frame"] = serde_json::json!(index);
            entry["boundary"] = serde_json::json!(boundary);
            frames.push(entry);
        }
        let gif_report = encode_gif(&run, &stage.join("timeline.gif"))?;
        let jpg_report = encode_jpg(&run.join("boundary-2.png"), &stage.join("timeline.jpg"))?;
        manifest["snapshot_verification"] = verify_snapshots(&run, first, split)?;
        manifest["commands"] = serde_json::json!(commands);
        manifest["boundaries"] = serde_json::json!(boundaries);
        manifest["frames"] = serde_json::json!(frames);
        manifest["outputs"] = serde_json::json!({
            "timeline.gif": gif_report,
            "timeline.jpg": jpg_report
        });
    }
    let source_note = format!(
        "source: Big Buck Bunny (2008, Blender Foundation), {BBB_CREDIT}; https://peach.blender.org/download/; 12s excerpt at 50s, excerpt SHA256 {}; captured by apps/vibecut/examples/ui_audit.rs --readme {slot} at {commit}",
        manifest["source"]["sha256"].as_str().unwrap_or("")
    );
    let notes = "notes: VibeCut original offscreen capture (GPL-3.0-only code/capture) over a CC-BY-3.0 Big Buck Bunny excerpt; authored Vietnamese/English caption is a demo edit, not recognition or translation; bundled Inter/Noto font and icon licences are recorded in docs/assets/readme/README.md; film credit retained";
    let files = if slot == "hero" {
        vec![(
            "hero.jpg",
            "VibeCut editor with a multitrack project",
            "The editor with file-backed video, audio, an authored title and bilingual caption.",
        )]
    } else {
        vec![
            (
                "timeline.gif",
                "Split, trim and undo on the VibeCut timeline",
                "An actual offscreen edit: split at 4 s, trim the right end by 0.5 s, undo.",
            ),
            (
                "timeline.jpg",
                "Split, trim and undo on the VibeCut timeline",
                "Poster frame for the timeline sequence: the right end after the 0.5 s trim.",
            ),
        ]
    };
    for (asset, title, description) in files {
        let sidecar = format!(
            "asset: {asset}\ntitle: {title}\nauthor: Blender Foundation; VibeCut contributors\n{source_note}\nlicense: CC-BY-3.0 (film excerpt); GPL-3.0-only (VibeCut capture)\nlicense-file: ../../licenses/CC-BY-3.0.txt\nadded: p0-core/22\n{notes}; capture: {description}\n"
        );
        fs::write(stage.join(format!("{asset}.attribution")), sidecar)?;
    }
    manifest["publication"] = serde_json::json!("staged");
    fs::write(
        run.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let (media, all): (&[&str], &[&str]) = if slot == "hero" {
        (&["hero.jpg"], &["hero.jpg", "hero.jpg.attribution"])
    } else {
        (
            &["timeline.gif", "timeline.jpg"],
            &[
                "timeline.gif",
                "timeline.gif.attribution",
                "timeline.jpg",
                "timeline.jpg.attribution",
            ],
        )
    };
    let run_id = run
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or("README run id")?;
    let publication = publish_outputs(&stage, &outputs, slot, media, all, run, &run_id)?;
    manifest["publication"] = serde_json::json!("published");
    let manifest = serde_json::to_vec_pretty(&manifest)?;
    finish_publication(publication, &run.join("manifest.json"), &manifest)?;
    println!("readme-media: {} {}", slot, run.display());
    Ok(())
}

fn main() -> Result<(), Failure> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize()?;
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("target"),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--readme") {
        if args.len() != 4 || args.get(2).is_none_or(|arg| arg != "--source") {
            return Err("usage: cargo xtask ui-audit --readme hero|timeline --source PATH".into());
        }
        return readme_media(&root, &target, &args[1], Path::new(&args[3]));
    }
    let output = target.join("ui-audit");
    std::fs::create_dir_all(&output)?;
    let references = reference_root(&root);
    let selected = select(args.into_iter())?;
    let mut rows = Vec::new();
    for (scene, theme) in &selected {
        let variant = format!("{}-{}", scene.name, theme.key());
        let mut images = Vec::new();
        for scale in SCALES {
            let mut app = (scene.build)(*theme, scale)?;
            app.run_steps(2);
            settle(&mut app)?;
            let image = app.render().map_err(std::io::Error::other)?;
            let image = opaque(image, vibecut::window_color(theme.dark()));
            let file = format!("{variant}@{}x.png", scale as u32);
            image.save(output.join(&file))?;
            if (scene.name.starts_with("assets-")
                || scene.name.starts_with("export-")
                || scene.name.starts_with("inspector-effects"))
                && scale == 1.0
            {
                let geometry:Vec<_>=app.state().inspect()?.widgets.into_iter().map(|widget|serde_json::json!({
                    "id":widget.id,"enabled":widget.enabled,
                    "rect":[widget.rect.left(),widget.rect.top(),widget.rect.width(),widget.rect.height()]
                })).collect();
                std::fs::write(
                    output.join(format!("{variant}-geometry.json")),
                    serde_json::to_vec_pretty(&geometry)?,
                )?;
            }
            images.push((file, scale));
        }
        let reference = scene.reference(*theme).and_then(|relative| {
            copy_reference(references.as_deref(), &output, &variant, relative)
        });
        rows.push(Row {
            variant,
            theme: theme.key(),
            images,
            reference,
        });
        eprintln!("ui-audit: {} rendered", scene.name);
    }
    let index = output.join("index.html");
    std::fs::write(&index, index_html(&rows))?;
    println!("ui-audit: {}", index.display());
    Ok(())
}

fn select(names: impl Iterator<Item = String>) -> Result<Vec<(&'static Scene, Theme)>, Failure> {
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
        if unknown.iter().any(|name| name == "settings") {
            return Err(
                "scene `settings` is unavailable: main exposes language/theme in the editor header; the settings dialog and its audit scene belong to p1-parity/75"
                    .into(),
            );
        }
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

fn reference_root(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VIBECUT_UI_REFERENCE") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    let local = root.join(".spikes/ui-reference");
    if local.is_dir() {
        return Some(local);
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .map(|worktree| worktree.join(".spikes/ui-reference"))
        .filter(|path| path.is_dir())
}

fn copy_reference(
    root: Option<&Path>,
    output: &Path,
    variant: &str,
    relative: &str,
) -> Option<String> {
    let source = root?.join(relative);
    let extension = source.extension()?.to_str()?;
    let name = format!("{variant}.{extension}");
    let directory = output.join("reference");
    std::fs::create_dir_all(&directory).ok()?;
    std::fs::copy(&source, directory.join(&name)).ok()?;
    Some(format!("reference/{name}"))
}

/// egui renders premultiplied alpha over a transparent clear; the audit PNG is
/// opaque so it can be compared and reviewed outside a compositor.
fn opaque(image: image::RgbaImage, background: egui::Color32) -> image::RgbaImage {
    let [red, green, blue, _] = background.to_array();
    let mut output = image::RgbaImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let [source_red, source_green, source_blue, source_alpha] = pixel.0;
        let inverse = 255 - u32::from(source_alpha);
        let blend = |source: u8, destination: u8| {
            (u32::from(source) + u32::from(destination) * inverse / 255).min(255) as u8
        };
        output.put_pixel(
            x,
            y,
            image::Rgba([
                blend(source_red, red),
                blend(source_green, green),
                blend(source_blue, blue),
                255,
            ]),
        );
    }
    output
}

struct Row {
    variant: String,
    theme: &'static str,
    images: Vec<(String, f32)>,
    reference: Option<String>,
}

fn index_html(rows: &[Row]) -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>VibeCut UI audit</title>\n<style>\n\
         body{font:14px/1.4 system-ui,sans-serif;margin:24px;background:#0f1113;color:#fff}\n\
         h1{font-size:24px} h2{font-size:18px;margin-top:32px}\n\
         table{border-collapse:collapse;width:100%}\n\
         th,td{text-align:left;vertical-align:top;padding:8px;border-top:1px solid #26272a}\n\
         img{max-width:100%;display:block;border:1px solid #26272a}\n\
         .missing{color:#898a8b;font-style:italic}\n\
         </style>\n</head>\n<body>\n<h1>VibeCut UI audit</h1>\n\
         <p>Offscreen renders at 1x and 2x. Reference images are copied from \
         <code>.spikes/ui-reference/</code> when present.</p>\n",
    );
    let mut current = "";
    for Row {
        variant,
        theme,
        images,
        reference,
    } in rows
    {
        let scene = variant
            .strip_suffix("-dark")
            .or_else(|| variant.strip_suffix("-light"))
            .unwrap_or(variant);
        if scene != current {
            if !current.is_empty() {
                let _ = writeln!(html, "</table>");
            }
            let _ = writeln!(html, "<h2>{scene}</h2>\n<table>");
            let _ = writeln!(
                html,
                "<tr><th>theme</th><th>1x</th><th>2x</th><th>reference</th></tr>"
            );
            current = scene;
        }
        let _ = writeln!(html, "<tr><th>{theme}</th>");
        for (file, _) in images {
            let _ = writeln!(
                html,
                "<td><a href=\"{file}\"><img src=\"{file}\" alt=\"{variant}\"></a></td>"
            );
        }
        match reference {
            Some(reference) => {
                let _ = writeln!(
                    html,
                    "<td><img src=\"{reference}\" alt=\"reference for {variant}\"></td></tr>"
                );
            }
            None => {
                let _ = writeln!(
                    html,
                    "<td class=\"missing\">no matching reference found</td></tr>"
                );
            }
        }
    }
    if !rows.is_empty() {
        html.push_str("</table>\n");
    }
    html.push_str("</body>\n</html>\n");
    html
}

/// A generation that never finishes, so the audit can draw the working state.
struct Hold;
impl vibecut::shorts::Generator for Hold {
    fn run(
        &self,
        _request: vibecut::shorts::GenerationRequest,
        _budget: ByteBudget,
        _events: &vibecut::shorts::TaskEvents,
        cancel: &engine::transcript::Cancellation,
    ) -> engine::Result<vibecut::shorts::ReviewSource> {
        while !cancel.cancelled() {
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(engine::EditError::Cancelled)
    }
}

/// Shared fixture, real widget path; no provider calls or visible native window.
fn shorts_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
    scene: &str,
) -> Result<AppHarness, Failure> {
    use vibecut::{
        services::playback::Sources,
        shorts::{Decision, State as Shorts},
    };
    static ASSET: std::sync::OnceLock<SourceAsset> = std::sync::OnceLock::new();
    let mut source = shorts_fixture::source();
    let preview: Arc<dyn Sources> = if let Some(path) = std::env::var_os("SHORTS_AUDIT_VIDEO") {
        let asset = match ASSET.get() {
            Some(asset) => asset.clone(),
            None => {
                let asset = SourceAsset::import(AssetId(1), PathBuf::from(path))?;
                let _ = ASSET.set(asset.clone());
                asset
            }
        };
        let mut data = source.project.into_data();
        data.assets = vec![asset];
        source.project = Project::new(data)?;
        Arc::new(Files)
    } else {
        Arc::new(shorts_fixture::Pixels)
    };
    let prefs = preferences(theme, locale);
    let generator: Arc<dyn vibecut::shorts::Generator> = if scene == "generating" {
        Arc::new(Hold)
    } else {
        Arc::new(shorts_fixture::Once(Mutex::new(Some(source))))
    };
    let mut shorts = Shorts::with_generator(
        prefs.look(),
        ByteBudget::new(768 << 20)?,
        generator,
        preview,
    )?;
    shorts.captions = false;
    if scene != "pick" {
        shorts.pick_video(std::env::temp_dir().join("Interview.mp4"))?;
    }
    let mut app = Harness::builder()
        .with_size(egui::vec2(1718.0, 989.0))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp {
            app: App::Shorts(Box::new(shorts)),
            _model_cache: None,
            _sound_library: None,
        });
    app.run_steps(3);
    if scene == "generating" {
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.generate")))
            .click();
        app.run_steps(4);
    }
    if matches!(scene, "review" | "layout" | "trim" | "results") {
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.generate")))
            .click();
        app.run_steps(1);
        let App::Shorts(shorts) = &mut app.state_mut().app else {
            return Err("missing Shorts audit app".into());
        };
        shorts_fixture::wait(shorts);
        app.run_steps(3);
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.candidate.0")))
            .click();
        app.run_steps(2);
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.accept")))
            .click();
        app.run_steps(2);
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let App::Shorts(shorts) = &mut app.state_mut().app else {
                return Err("missing Shorts audit app".into());
            };
            shorts.tick();
            if let Some(error) = shorts.error() {
                return Err(std::io::Error::other(error.to_string()).into());
            }
            if let Some(error) = shorts.preview().and_then(|p| p.error()) {
                return Err(std::io::Error::other(error).into());
            }
            if shorts.preview().is_some_and(|p| p.frame().is_some()) {
                if shorts
                    .reviews()
                    .first()
                    .is_none_or(|r| r.decision != Decision::Accepted)
                {
                    return Err("Shorts review action did not apply".into());
                }
                break;
            }
            if std::time::Instant::now() > deadline {
                return Err("Shorts preview timeout".into());
            }
            std::thread::yield_now();
        }
        app.run_steps(3);
        if scene == "results" {
            app.get(By::new().predicate(|n| n.author_id() == Some("shorts.grid")))
                .click();
            app.run_steps(3);
        }
        let tool = match scene {
            "layout" => Some("shorts.tool.layout"),
            "trim" => Some("shorts.tool.trim"),
            _ => None,
        };
        if let Some(tool) = tool {
            app.get(By::new().predicate(move |n| n.author_id() == Some(tool)))
                .click();
            app.run_steps(3);
        }
    }
    Ok(app)
}

/// Uses the area slot at the measured editor size, independent of sibling work.
struct EffectsClock(std::sync::atomic::AtomicU64);
impl engine::jobs::Clock for EffectsClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}
fn assets_scene(theme: Theme, scale: f32, state: &str) -> Result<AppHarness, Failure> {
    let locale = if state.ends_with("-vi") {
        Locale::Vi
    } else {
        Locale::En
    };
    let state = state.strip_suffix("-vi").unwrap_or(state);
    let phase3 = state.ends_with("-phase3");
    let state = state.strip_suffix("-phase3").unwrap_or(state);
    let mut sound_library = None;
    let mut editor = empty_editor(theme, locale)?;
    let clock = Arc::new(EffectsClock(std::sync::atomic::AtomicU64::new(0)));
    editor.set_clock(clock.clone());
    if matches!(state, "media" | "offline") {
        editor.set_import_probe(Arc::new(CompletedProbe))?;
        editor.set_thumbnailer(Arc::new(AssetsThumbnail))?;
        editor.drop_files(vec![
            std::env::temp_dir().join("Interview.mp4"),
            std::env::temp_dir().join("audio-music.wav"),
        ])?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while editor.importing() {
            editor.tick();
            if Instant::now() >= deadline {
                return Err("assets audit import timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    editor.preferences.assets_tab = match state {
        "effects" => vibecut::panels::AssetsTab::Effects,
        "transitions" => vibecut::panels::AssetsTab::Transitions,
        "text" => vibecut::panels::AssetsTab::Text,
        "captions" => vibecut::panels::AssetsTab::Captions,
        "offline" => vibecut::panels::AssetsTab::Sounds,
        _ => vibecut::panels::AssetsTab::Media,
    };
    if state == "offline" {
        let directory = tempfile::tempdir()?;
        let library = Arc::new(libraries::sounds::SoundLibrary::open(directory.path())?);
        library.save(&libraries::sounds::SoundResult {
            id: 1,
            name: "Night Drive".into(),
            author: "BCD Studio".into(),
            licence: "Creative Commons 0".into(),
            attribution: "Night Drive by BCD Studio (Creative Commons 0)".into(),
            source_url: "https://freesound.org/s/1/".into(),
            audio_url: "https://cdn.example/1.mp3".into(),
            duration_ms: 31_000,
            downloads: 100,
        })?;
        editor.set_sound_library(library)?;
        sound_library = Some(directory);
    }
    if state == "loading" {
        editor.set_import_probe(Arc::new(ProgressProbe::new()))?;
        editor.drop_files(vec![
            std::env::temp_dir().join("one.mp4"),
            std::env::temp_dir().join("two.mp4"),
        ])?;
    }
    if state == "error" {
        editor.start_import(vec![std::env::temp_dir().join("missing-audit-media.mp4")])?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while editor.importing() {
            editor.tick();
            if Instant::now() >= deadline {
                return Err("assets audit refusal timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let mut app = harness_sized(App::Assets(Box::new(editor)), scale, [1727.0, 1029.0]);
    app.state_mut()._sound_library = sound_library;
    app.run_steps(4);
    if phase3 {
        if let App::Assets(editor) = &mut app.state_mut().app {
            editor.settle_previews(Duration::from_secs(10))?;
        }
        clock.0.store(100, Ordering::SeqCst);
        app.run_steps(1);
    }
    Ok(app)
}

/// Own generated pixels, injected through the real thumbnail publication seam.
struct AssetsThumbnail;
impl vibecut::import::ThumbnailSource for AssetsThumbnail {
    fn thumbnail(
        &self,
        _: &SourceAsset,
        _: Tick,
        _: u32,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<media::Thumbnail> {
        if cancelled.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        Ok(media::Thumbnail {
            width: 120,
            height: 73,
            frame: Tick::ZERO,
            pixels: [80, 103, 126, 255].repeat(120 * 73),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_capture_refuses_wrong_source_before_staging_and_keeps_history() -> Result<(), Failure>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let media = root.join("docs/assets/readme");
        fs::create_dir_all(&media)?;
        fs::write(media.join("hero.jpg"), b"keep")?;
        let old_run = root.join("target/readme-media/hero/old-run");
        fs::create_dir_all(&old_run)?;
        fs::write(old_run.join("manifest.json"), b"history")?;
        let source = root.join("wrong.mp4");
        fs::write(&source, b"not the admitted excerpt")?;
        let error = readme_media(root, &root.join("target"), "hero", &source).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not the admitted Big Buck Bunny")
        );
        assert_eq!(fs::read(media.join("hero.jpg"))?, b"keep");
        assert_eq!(fs::read(old_run.join("manifest.json"))?, b"history");
        assert_eq!(
            fs::read_dir(root.join("target/readme-media/hero"))
                .map_err(|error| error.to_string())?
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn publish_rolls_back_after_a_real_file_failure() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        let owned = "asset: hero.jpg\ntitle: old\nauthor: VibeCut contributors\nsource: apps/vibecut/examples/ui_audit.rs --readme hero at old\nlicense: CC-BY-3.0 (film excerpt); GPL-3.0-only (VibeCut capture)\nlicense-file: ../../licenses/CC-BY-3.0.txt\nadded: p0-core/22\nnotes: old\n";
        fs::write(finals.join("hero.jpg"), b"old-media")?;
        fs::write(finals.join("hero.jpg.attribution"), owned)?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), owned)?;
        let error = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg", "hero.jpg.attribution", "timeline.jpg"],
            &run,
            "run1",
        )
        .unwrap_err();
        assert!(error.to_string().contains("staged README output"));
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"old-media");
        assert_eq!(
            fs::read_to_string(finals.join("hero.jpg.attribution"))?,
            owned
        );
        assert!(!finals.join(".readme-media-backup-run1").exists());
        assert!(!run.join("preserve-staging").exists());
        Ok(())
    }

    #[test]
    fn publish_refuses_an_unrelated_existing_final() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        fs::write(finals.join("hero.jpg"), b"unrelated")?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), b"asset: hero.jpg\n")?;
        let error = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg"],
            &run,
            "run2",
        )
        .unwrap_err();
        assert!(error.to_string().contains("refusing unrelated"));
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"unrelated");
        Ok(())
    }

    #[test]
    fn publish_refuses_readable_but_incomplete_ownership_sidecar() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        fs::write(finals.join("hero.jpg"), b"unrelated")?;
        fs::write(finals.join("hero.jpg.attribution"), b"asset: hero.jpg\n")?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), b"asset: hero.jpg\n")?;
        let error = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg", "hero.jpg.attribution"],
            &run,
            "run3",
        )
        .unwrap_err();
        assert!(error.to_string().contains("refusing unrelated"));
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"unrelated");
        Ok(())
    }

    #[test]
    fn publication_manifest_failure_rolls_back_finals() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let stage = root.join("stage");
        let finals = root.join("finals");
        let run = root.join("run");
        fs::create_dir_all(&stage)?;
        fs::create_dir_all(&finals)?;
        fs::create_dir_all(&run)?;
        let owned = "asset: hero.jpg\ntitle: old\nauthor: VibeCut contributors\nsource: apps/vibecut/examples/ui_audit.rs --readme hero at old\nlicense: CC-BY-3.0 (film excerpt); GPL-3.0-only (VibeCut capture)\nlicense-file: ../../licenses/CC-BY-3.0.txt\nadded: p0-core/22\nnotes: old\n";
        fs::write(finals.join("hero.jpg"), b"old-media")?;
        fs::write(finals.join("hero.jpg.attribution"), owned)?;
        fs::write(stage.join("hero.jpg"), b"new-media")?;
        fs::write(stage.join("hero.jpg.attribution"), owned)?;
        let publication = publish_outputs(
            &stage,
            &finals,
            "hero",
            &["hero.jpg"],
            &["hero.jpg", "hero.jpg.attribution"],
            &run,
            "run4",
        )?;
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"new-media");
        let manifest_directory = root.join("manifest-dir");
        fs::create_dir_all(&manifest_directory)?;
        let error = finish_publication(publication, &manifest_directory, b"manifest").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("publication manifest write failed")
        );
        assert_eq!(fs::read(finals.join("hero.jpg"))?, b"old-media");
        assert_eq!(
            fs::read_to_string(finals.join("hero.jpg.attribution"))?,
            owned
        );
        assert!(!finals.join(".readme-media-backup-run4").exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn supervised_runner_does_not_wait_for_inherited_output_handles() {
        let start = Instant::now();
        let result = run_tool(
            "cmd",
            &[
                "/c",
                "start",
                "/b",
                "powershell",
                "-NoProfile",
                "-Command",
                "Start-Sleep -Seconds 5",
            ],
            Duration::from_secs(2),
            1024,
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        if let Err(error) = result {
            assert!(!error.to_string().contains("timed out"), "{error}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn supervised_runner_times_out_and_reaps_a_stalled_process() {
        let start = Instant::now();
        let error = run_tool(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 5",
            ],
            Duration::from_millis(150),
            1024,
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[cfg(windows)]
    #[test]
    fn supervised_runner_refuses_bounded_output_overflow() {
        let error = run_tool(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.Write('x' * 10000)",
            ],
            Duration::from_secs(5),
            100,
        )
        .unwrap_err();
        assert!(error.to_string().contains("bounded cap"), "{error}");
    }

    #[test]
    fn readme_capture_refuses_unknown_slots_before_touching_final_or_staging() -> Result<(), Failure>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let media = root.join("docs/assets/readme");
        fs::create_dir_all(&media)?;
        fs::write(media.join("hero.jpg"), b"keep")?;
        let error = readme_media(
            root,
            &root.join("target"),
            "unknown",
            &media.join("hero.jpg"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown README slot"));
        assert_eq!(fs::read(media.join("hero.jpg"))?, b"keep");
        assert!(!root.join("target/readme-media").exists());
        Ok(())
    }

    #[test]
    fn mixed_selection_preview_matches_its_edited_timeline() -> Result<(), Failure> {
        let app = inspector_mixed_vi(Theme::Light, 1.0)?;
        let App::Editor(editor) = &app.state().app else {
            return Err("expected editor scene".into());
        };
        let frame = editor.playback.frame().ok_or("missing preview frame")?;
        // This corner lies outside the text. It must show the scene's solid
        // overlay, rather than the synthetic video deleted during setup.
        let corner = frame.image.pixels.first().ok_or("empty preview frame")?;
        assert_eq!(*corner, egui::Color32::from_rgb(35, 45, 65));
        Ok(())
    }
}
