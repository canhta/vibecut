//! Owns audit harness construction, root app state, theme selection, scale constants.
use egui_kittest::Harness;
use engine::Session;
use time::TICKS_PER_SECOND;
use time::Tick;
use vibecut::Editor;
use vibecut::Inspection;
use vibecut::egui;
use vibecut::i18n::Locale;
use vibecut::preferences::Preferences;
use vibecut::workspace::Workspace;

pub(super) const SECOND: i64 = TICKS_PER_SECOND;
pub(super) const WINDOW: [f32; 2] = [1440.0, 900.0];
pub(super) const SCALES: [f32; 2] = [1.0, 2.0];

pub(crate) type Failure = Box<dyn std::error::Error>;
pub(crate) type AppHarness = Harness<'static, AuditApp>;

/// Audit roots: Home, the full editor and the dedicated Shorts window.
pub(crate) enum App {
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
pub(crate) struct AuditApp {
    pub(crate) app: App,
    pub(super) _model_cache: Option<tempfile::TempDir>,
    pub(super) _sound_library: Option<tempfile::TempDir>,
}
impl AuditApp {
    pub(super) fn busy(&self) -> bool {
        self.app.busy()
    }
    pub(super) fn inspect(&self) -> Result<Inspection, Failure> {
        self.app.inspect()
    }
    pub(super) fn presented_frame(&self) -> Option<Tick> {
        self.app.presented_frame()
    }
    pub(super) fn session(&self) -> Option<&Session> {
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
pub(crate) enum Theme {
    Dark,
    Light,
}
impl Theme {
    pub(super) const ALL: [Self; 2] = [Self::Dark, Self::Light];
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    pub(super) fn dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}
pub(super) fn preferences(theme: Theme, locale: Locale) -> Preferences {
    Preferences {
        locale,
        dark: theme.dark(),
        ..Preferences::default()
    }
}

pub(super) fn harness(app: App, scale: f32) -> AppHarness {
    harness_sized(app, scale, WINDOW)
}

pub(super) fn harness_sized(app: App, scale: f32, size: [f32; 2]) -> AppHarness {
    Harness::builder()
        .with_size(egui::vec2(size[0], size[1]))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp {
            app,
            _model_cache: None,
            _sound_library: None,
        })
}
