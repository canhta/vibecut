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
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{
    ByteBudget, Command, CreatedId, ItemParamPatch, NewItem, Placement, Session, TrackPlacement,
};
use media::{AssetData, AssetId, ContentHash, MediaError, SourceAsset, SourceStream, StreamKind};
use project::{
    AudioParams, Color, ItemId, ItemParams, OverlayParams, Project, ProjectId, TextParams, TrackId,
    TrackKind, VideoParams,
};
use render::{AudioBlock, ExecutionHooks, Image, MemoryKind, SourceProvider, UnavailableReason};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
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
    let (mut editor, item) = media_editor(theme, Locale::En)?;
    capture_columns(&mut editor)?;
    editor.selection = [item].into_iter().collect();
    editor
        .preferences
        .set("properties_tabs", serde_json::json!({"video": tab}))?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, CAPTURE_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
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

/// Populated multi-track timeline (video, audio, text) at the reference size.
fn timeline_multi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, Locale::En)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, EDITOR_WINDOW);
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

fn main() -> Result<(), Failure> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize()?;
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("target"),
    };
    let output = target.join("ui-audit");
    std::fs::create_dir_all(&output)?;
    let references = reference_root(&root);
    let selected = select(std::env::args().skip(1))?;
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
            if (scene.name.starts_with("assets-") || scene.name.starts_with("export-"))
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
fn assets_scene(theme: Theme, scale: f32, state: &str) -> Result<AppHarness, Failure> {
    let locale = if state.ends_with("-vi") {
        Locale::Vi
    } else {
        Locale::En
    };
    let state = state.strip_suffix("-vi").unwrap_or(state);
    let mut sound_library = None;
    let mut editor = empty_editor(theme, locale)?;
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
