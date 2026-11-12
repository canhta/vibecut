//! Nonblocking desktop lifecycle and recovery presentation over engine persistence.
use crate::i18n;
use crate::{
    Editor, Error, Result, egui,
    home::{Action as HomeAction, State as HomeState},
    i18n::Text,
    kit::{self as icons, Icon},
    lifecycle::EditorAction,
    lifecycle::fresh,
    preferences::Preferences,
    services::playback::{Files, Playback},
};
use engine::{
    ByteBudget, Command, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    control::Error as EngineError,
    jobs::{
        JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
        SystemClock,
    },
    recovery::{self, Autosave, Candidate, Discovery, Schedule},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

/// Application lifetime. Home owns catalog/discovery; each registered project
/// retains its own editor, writer lease, pending save and close prompt.
pub struct Workspace {
    pub windows: BTreeMap<project::ProjectId, EditorWindow>,
    pub projects: HomeState,
    pub shorts: Option<crate::shorts::State>,
    pub preferences: Preferences,
    /// Injected native reader shared by Home and immediate child composition.
    pub text_clipboard: crate::services::text_clipboard::TextClipboard,
    home: EditorWindow,
    focused: Option<project::ProjectId>,
    focus_requested: Option<project::ProjectId>,
    home_open: bool,
    shorts_window: bool,
    models: Option<crate::model_manager::ModelManager>,
    /// Project files seen this session (catalog rows and open windows), so
    /// lane memory is dropped only on proof the file is gone.
    lane_paths: BTreeMap<project::ProjectId, PathBuf>,
    lane_prune_key: Vec<project::ProjectId>,
    sounds: Option<Arc<libraries::sounds::SoundLibrary>>,
    /// One process music library and session credential shared by every window.
    music: Option<Arc<crate::services::music::MusicSession>>,
    /// One process media cache instance, lent to every editor's timeline and to
    /// the desktop MCP inbox. A second open of the same root is never attempted.
    media: Option<Arc<Mutex<engine::control::MediaAccess>>>,
}

impl Workspace {
    pub fn new(root: PathBuf, preferences: Preferences, path: Option<PathBuf>) -> Result<Self> {
        let projects = root.with_file_name("projects");
        Self::with_projects(root, projects, preferences, path)
    }
    pub fn with_projects(
        root: PathBuf,
        projects_root: PathBuf,
        preferences: Preferences,
        path: Option<PathBuf>,
    ) -> Result<Self> {
        let budget = ByteBudget::new(768 << 20)?;
        let persistence = ByteBudget::new(256 << 20)?;
        // One saved-sound library is shared by every project window; the
        // adjacent writer lease refuses a second in-process owner.
        let sounds = libraries::sounds::SoundLibrary::open(&root.with_file_name("sounds"))
            .ok()
            .map(Arc::new);
        // One music library and in-memory session key are shared by every
        // project window; the adjacent writer lease refuses a second owner.
        let music = libraries::music::MusicLibrary::open(&root.with_file_name("music"))
            .ok()
            .map(|library| Arc::new(crate::services::music::MusicSession::new(Arc::new(library))));
        // One media cache owner for the whole desktop process. Its workspace
        // root keeps test runs isolated and never races the CLI's cache root.
        let media =
            engine::control::MediaAccess::open_in(&root.with_file_name("media"), budget.clone())
                .ok()
                .map(|access| Arc::new(Mutex::new(access)));
        let runner = std::rc::Rc::new(std::cell::RefCell::new(JobRunner::new(
            RunnerConfig {
                concurrency: [2, 1, 1],
                queue: 128,
                retained: 128,
                network_attempts: 1,
                backoff: Duration::from_secs(1),
                progress_interval: Duration::from_millis(10),
            },
            persistence.clone(),
            Arc::new(SystemClock::default()),
        )?));
        let mut home = EditorWindow::new(
            root,
            preferences.clone(),
            budget.clone(),
            persistence,
            runner,
            sounds.clone(),
            music.clone(),
            media.clone(),
        );
        home.startup_path = path;
        home.submit(Work::Discover, Text::Discovering)?;
        Ok(Self {
            windows: BTreeMap::new(),
            projects: HomeState::new(projects_root, budget),
            shorts: None,
            preferences,
            text_clipboard: Default::default(),
            home,
            focused: None,
            focus_requested: None,
            home_open: true,
            shorts_window: false,
            models: None,
            lane_paths: BTreeMap::new(),
            lane_prune_key: Vec::new(),
            sounds,
            music,
            media,
        })
    }
    pub fn control_budget(&self) -> ByteBudget {
        self.home.budget.clone()
    }
    /// The one process media cache instance the timeline and MCP inbox share.
    pub fn media(&self) -> Option<Arc<Mutex<engine::control::MediaAccess>>> {
        self.media.clone()
    }
    /// The copied appearance value every window reads this frame.
    fn look(&self) -> crate::appearance::Look {
        self.preferences.look()
    }
    /// Register an already-composed editor, retaining its writer lease and
    /// resource policy. Repeated project identities focus their existing window.
    pub fn register_editor(&mut self, mut editor: Box<Editor>) -> project::ProjectId {
        self.rekey_windows();
        if let Some(sounds) = &self.sounds {
            let _ = editor.set_sound_library(sounds.clone());
        }
        if let Some(music) = &self.music {
            let _ = editor.set_music_session(music.clone());
        }
        if let Some(media) = &self.media {
            editor.set_media(media.clone());
        }
        let id = editor.session.project().data().id;
        let path = match &editor.session {
            crate::lifecycle::EditorSession::Persistent(s) => s.saved_path().map(ToOwned::to_owned),
            crate::lifecycle::EditorSession::Memory(_) => None,
        };
        self.windows.entry(id).or_insert_with(|| {
            let mut window = EditorWindow::new(
                self.home.root.clone(),
                self.preferences.clone(),
                editor.playback.budget().clone(),
                self.home.persistence.clone(),
                self.home.runner.clone(),
                self.sounds.clone(),
                self.music.clone(),
                self.media.clone(),
            );
            window.project_window = true;
            window.title = editor.session.project().data().name.as_str().to_owned();
            window.editor = Some(editor);
            window.path = path;
            window.device_available = self.home.device_available;
            window
        });
        self.focused = Some(id);
        self.focus_requested = Some(id);
        id
    }
    pub fn focused_project(&self) -> Option<project::ProjectId> {
        self.focused
    }
    pub fn home_open(&self) -> bool {
        self.home_open
    }
    pub fn viewport(&self, id: project::ProjectId) -> Option<egui::ViewportId> {
        Some(self.windows.get(&id)?.viewport)
    }
    pub fn focus(&mut self, id: project::ProjectId) -> Result<()> {
        if !self.windows.contains_key(&id) {
            return Err(Error::Invalid("unknown project window"));
        }
        self.focused = Some(id);
        self.focus_requested = Some(id);
        Ok(())
    }
    pub fn editor_mut(&mut self, id: project::ProjectId) -> Option<&mut Editor> {
        self.windows.get_mut(&id)?.editor.as_deref_mut()
    }
    /// The focused project's window, including its pending lifecycle operation.
    pub fn focused_window_mut(&mut self) -> Option<&mut EditorWindow> {
        self.focused.and_then(|id| self.windows.get_mut(&id))
    }
    pub fn focused_editor(&self) -> Option<&Editor> {
        self.focused
            .and_then(|id| self.windows.get(&id))?
            .editor
            .as_deref()
    }
    pub fn focused_editor_mut(&mut self) -> Option<&mut Editor> {
        self.focused_window_mut()?.editor.as_deref_mut()
    }
    /// Save the focused project without coupling its operation to Home or peers.
    pub fn persist(
        &mut self,
        path: Option<PathBuf>,
        leave: bool,
        close: bool,
        journal_only: bool,
    ) -> Result<()> {
        self.focused_window_mut()
            .ok_or(Error::Invalid("no project"))?
            .persist(path, leave, close, journal_only)
    }
    pub fn busy(&self) -> bool {
        self.home.busy() || self.projects.busy() || self.windows.values().any(EditorWindow::busy)
    }
    pub fn error(&self) -> Option<&Error> {
        self.home.error().or_else(|| {
            self.focused
                .and_then(|id| self.windows.get(&id))
                .and_then(EditorWindow::error)
        })
    }
    pub fn open(&mut self, path: PathBuf) -> Result<()> {
        self.rekey_windows();
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if let Some(id) = self.windows.iter().find_map(|(id, window)| {
            let saving = window
                .retry_save
                .as_ref()
                .and_then(|s| s.0.as_ref())
                .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
            (window.path.as_ref() == Some(&canonical) || saving.as_ref() == Some(&canonical))
                .then_some(*id)
        }) {
            return self.focus(id);
        }
        self.home.preferences = self.preferences.clone();
        self.home.open(path)
    }
    pub fn create(&mut self) -> Result<()> {
        self.home.preferences = self.preferences.clone();
        self.home.create()
    }
    pub fn open_shorts(&mut self) -> Result<()> {
        if self.shorts.is_none() {
            self.shorts = Some(crate::shorts::State::new(
                self.look(),
                self.home.budget.clone(),
            )?);
        }
        self.shorts_window = true;
        Ok(())
    }
    pub fn warn_preferences_reset(&mut self) {
        self.home.warn_preferences_reset();
    }
    /// Closing Home hides only Home. The root event loop outlives child windows.
    pub fn request_close(&mut self) {
        self.home_open = false;
    }
    pub fn ready_to_close(&self) -> bool {
        !self.home_open && self.windows.is_empty() && !self.shorts_window && !self.home.busy()
    }
    pub fn set_device_available(&mut self, available: bool) {
        self.home.device_available = available;
        for window in self.windows.values_mut() {
            window.set_device_available(available);
        }
    }
    pub fn tick(&mut self) {
        self.projects.tick();
        if let Some(action) = self.projects.take_action() {
            self.project_action(action);
        }
        // Catalog admission precedes startup open; discovery must not race it.
        self.home.tick();
        if let Some(path) = self.home.requested_open.take()
            && let Err(error) = self.open(path)
        {
            self.home.error = Some(error);
        }
        if !self.projects.busy()
            && !self.home.busy()
            && self.home.candidates.is_empty()
            && self.home.error.is_none()
            && let Some(path) = self.home.startup_path.take()
            && let Err(error) = self.open(path)
        {
            self.home.error = Some(error);
        }
        if let Some(editor) = self.home.editor.take() {
            self.register_editor(editor);
        }
        for window in self.windows.values_mut() {
            // Background workflow completions can request the model service
            // before the next UI pass. They must reuse the app owner too.
            if let Some(editor) = &mut window.editor
                && let Some(models) = self.models.take()
            {
                editor.set_model_manager(models);
            }
            window.tick();
            if let Some((from, to)) = window.saved_as.take() {
                self.preferences.copy_timeline_lanes(from, to);
            }
            if let Some(models) = window.retained_models.take() {
                self.models = Some(models);
            } else if let Some(editor) = &mut window.editor
                && let Some(models) = editor.take_model_manager()
            {
                self.models = Some(models);
            }
        }
        self.rekey_windows();
        let count = self.windows.len();
        self.windows
            .retain(|_, w| !(w.editor.is_none() && !w.busy() && w.error.is_none()));
        if count != self.windows.len() {
            self.projects.refresh();
        }
        if self
            .focused
            .is_some_and(|id| !self.windows.contains_key(&id))
        {
            self.focused = self.windows.keys().next_back().copied();
        }
        self.prune_lane_preferences();
        if let Some(shorts) = &mut self.shorts {
            shorts.tick();
        }
        self.home.runner.borrow_mut().drain_events();
        if let Some(models) = &mut self.models
            && let Err(error) = models.tick()
        {
            self.home.error.get_or_insert(error);
        }
    }
    fn rekey_windows(&mut self) {
        // Save As changes a document's identity after its publication is acknowledged.
        // Move the existing window and all its local state to that registry key.
        let changed: Vec<_> = self
            .windows
            .iter()
            .filter_map(|(id, window)| {
                let next = window.editor.as_ref()?.session.project().data().id;
                (*id != next).then_some((*id, next))
            })
            .collect();
        for (old, next) in changed {
            if let Some(window) = self.windows.remove(&old) {
                if let Some(geometry) = self
                    .preferences
                    .windows
                    .get(&format!("editor.{}", old.0))
                    .cloned()
                {
                    remember_geometry(
                        &mut self.preferences,
                        format!("editor.{}", next.0),
                        geometry,
                    );
                }
                self.windows.insert(next, window);
                if self.focused == Some(old) {
                    self.focused = Some(next);
                }
                if self.focus_requested == Some(old) {
                    self.focus_requested = Some(next);
                }
            }
        }
    }
    /// Lane preferences outlive project windows. An entry goes only when its
    /// project no longer exists: not open, not listed by a settled and
    /// its project file (seen this session) is gone. A project opened from a
    /// file keeps its lanes while the file exists; one never seen this session
    /// has no evidence of deletion and is kept.
    fn prune_lane_preferences(&mut self) {
        for entry in self.projects.entries() {
            self.lane_paths.insert(entry.id, entry.path.clone());
        }
        for (id, window) in &self.windows {
            if let Some(path) = &window.path {
                self.lane_paths.insert(*id, path.clone());
            }
        }
        let listed: BTreeSet<project::ProjectId> = self
            .projects
            .settled_ids()
            .map(Iterator::collect)
            .unwrap_or_default();
        let mut key: Vec<_> = self
            .windows
            .keys()
            .copied()
            .chain(listed.iter().copied())
            .collect();
        key.sort_unstable();
        key.dedup();
        if key == self.lane_prune_key {
            return;
        }
        self.lane_prune_key = key;
        let paths = &self.lane_paths;
        let windows = &self.windows;
        self.preferences.retain_timeline_lanes(|id| {
            windows.contains_key(&id)
                || listed.contains(&id)
                || paths.get(&id).is_none_or(|path| path.exists())
        });
    }
    fn project_action(&mut self, action: HomeAction) {
        let result = match action {
            HomeAction::Open(path) => self.open(path),
            HomeAction::OpenFile => {
                self.home.open_file_requested = true;
                Ok(())
            }
            HomeAction::Shorts => self.open_shorts(),
        };
        if let Err(error) = result {
            self.home.error = Some(error);
        }
    }
    /// Draw a specific project through the same path as its native viewport.
    /// Kittest callers select the project explicitly, preserving automation ids.
    pub fn window_ui(&mut self, id: project::ProjectId, ui: &mut egui::Ui) {
        let owner = self
            .windows
            .get(&id)
            .and_then(|w| w.editor.as_ref())
            .map(|e| {
                egui::Id::unique((
                    e.session.session_id(),
                    e.session.project().data().id,
                    e.session.project().data().current_scene_id,
                    &e.selection,
                ))
            })
            .unwrap_or_else(|| egui::Id::unique(("loading", id)));
        self.text_clipboard.bridge(ui.ctx(), owner);
        if let Some(window) = self.windows.get_mut(&id) {
            window.preferences = self.preferences.clone();
            if let Some(editor) = &mut window.editor {
                editor.preferences = self.preferences.clone();
                editor.set_models_visible(self.focused == Some(id));
                if let Some(models) = self.models.take() {
                    editor.set_model_manager(models);
                }
            }
            window.ui(ui);
            if let Some(models) = window.retained_models.take() {
                self.models = Some(models);
            } else if let Some(editor) = &mut window.editor
                && let Some(models) = editor.take_model_manager()
            {
                self.models = Some(models);
            }
            self.preferences = window.preferences.clone();
            if let Some((from, to)) = window.saved_as.take() {
                self.preferences.copy_timeline_lanes(from, to);
            }
            if std::mem::take(&mut window.focus_home) {
                self.home_open = true;
                ui.ctx().send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Visible(true),
                );
                ui.ctx().send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Minimized(false),
                );
                ui.ctx()
                    .send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.tick();
        if !self.home.configured {
            Editor::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.home.configured = true;
        }
        Editor::theme(ui.ctx(), self.preferences.dark);
        let ctx = ui.ctx().clone();
        if self.home_open {
            self.text_clipboard.bridge(
                ui.ctx(),
                egui::Id::unique(("home.text", self.projects.selection())),
            );
        }
        // Our own final `Close` returns here as `close_requested`; cancelling it
        // would leave a hidden process running after the last window closed.
        if ui.input(|i| i.viewport().close_requested()) && !self.ready_to_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close();
        }
        if self.home_open {
            self.home.preferences = self.preferences.clone();
            self.home.ui(ui);
            if !self.home.busy() && self.home.candidates.is_empty() && self.home.error.is_none() {
                let status = self.shorts.as_ref().and_then(|shorts| {
                    if shorts.cancelling() {
                        Some(Text::ShortsCancelling)
                    } else if shorts.awaiting_consent() {
                        Some(Text::ShortsApproval)
                    } else if shorts.exporting() {
                        Some(Text::ShortsExporting)
                    } else if shorts.busy() {
                        Some(Text::ShortsCreating)
                    } else if shorts.source().is_some() {
                        Some(Text::ShortsReview)
                    } else {
                        None
                    }
                });
                if let Some(action) = self.projects.show(
                    ui,
                    self.preferences.look(),
                    &mut self.preferences.projects,
                    status,
                ) {
                    self.project_action(action);
                }
            }
            self.home.preferences = self.preferences.clone();
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }
        let ids: Vec<_> = self.windows.keys().copied().collect();
        for id in ids {
            let Some(viewport) = self.viewport(id) else {
                continue;
            };
            let title = self
                .windows
                .get(&id)
                .map(|window| window.title().to_owned())
                .unwrap_or_else(|| i18n::text(self.preferences.locale, Text::AppName).to_owned());
            let geometry = self
                .preferences
                .windows
                .get(&format!("editor.{}", id.0))
                .cloned()
                .unwrap_or_default();
            ctx.show_viewport_immediate(viewport, geometry.builder().with_title(title), |ui, _| {
                if ui.input(|i| i.focused) {
                    self.focused = Some(id);
                }
                self.window_ui(id, ui);
                if let Some(geometry) = crate::preferences::WindowGeometry::read(ui) {
                    remember_geometry(&mut self.preferences, format!("editor.{}", id.0), geometry);
                }
            });
        }
        if let Some(id) = self.focus_requested.take()
            && let Some(viewport) = self.viewport(id)
        {
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
        }
        if self.shorts_window
            && let Some(shorts) = &mut self.shorts
        {
            let cx = crate::shorts::Context {
                editor_available: !self.home.busy(),
            };
            let mut close = false;
            let mut actions = Vec::new();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("shorts.window"),
                self.preferences
                    .windows
                    .get("shorts")
                    .cloned()
                    .unwrap_or(crate::preferences::WindowGeometry {
                        size: crate::shorts::WINDOW,
                        ..Default::default()
                    })
                    .builder()
                    .with_title(i18n::text(shorts.preferences.locale, Text::Shorts)),
                |ui, _| {
                    shorts.apply_model_preferences(&self.preferences.ai_models);
                    self.text_clipboard.bridge(
                        ui.ctx(),
                        egui::Id::unique((
                            "shorts.text",
                            shorts.source().map(|s| s.project.data().id),
                            shorts.preview_session().map(|s| s.session_id()),
                            shorts.selected(),
                        )),
                    );
                    actions = crate::shorts::show(shorts, &cx, ui);
                    close = ui.input(|i| i.viewport().close_requested());
                    if let Some(geometry) = crate::preferences::WindowGeometry::read(ui) {
                        remember_geometry(&mut self.preferences, "shorts".into(), geometry);
                    }
                },
            );
            if close {
                shorts.pause_preview();
                self.shorts_window = false;
            }
            if actions.contains(&crate::shorts::Action::OpenEditor) && !self.home.busy() {
                let result = shorts.editor_session().and_then(|(s, settings)| {
                    self.home
                        .submit(Work::Short(Box::new(s), settings), Text::LoadingProject)
                });
                if let Err(e) = result {
                    self.home.error = Some(e);
                }
            }
        }
        if let Some(geometry) = crate::preferences::WindowGeometry::read(ui) {
            remember_geometry(&mut self.preferences, "home".into(), geometry);
        }
        let mut text_viewports: Vec<_> = self
            .windows
            .keys()
            .filter_map(|id| self.viewport(*id))
            .collect();
        if self.home_open {
            text_viewports.push(egui::ViewportId::ROOT);
        }
        if self.shorts_window {
            text_viewports.push(egui::ViewportId::from_hash_of("shorts.window"));
        }
        if ctx.embed_viewports() && !text_viewports.is_empty() {
            text_viewports = vec![egui::ViewportId::ROOT];
        }
        self.text_clipboard.retain_viewports(&ctx, &text_viewports);
        if self.ready_to_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

fn remember_geometry(
    preferences: &mut Preferences,
    key: String,
    mut geometry: crate::preferences::WindowGeometry,
) {
    if geometry.maximized
        && let Some(previous) = preferences.windows.get(&key)
    {
        geometry.size = previous.size;
        geometry.position = previous.position;
    }
    if preferences.windows.get(&key) == Some(&geometry) {
        return;
    }
    let previous = preferences.windows.clone();
    if !preferences.windows.contains_key(&key)
        && preferences.windows.len() >= 64
        && let Some(oldest) = preferences
            .windows
            .keys()
            .find(|k| *k != "home" && *k != "shorts")
            .cloned()
    {
        preferences.windows.remove(&oldest);
    }
    preferences.windows.insert(key.clone(), geometry);
    // Geometry is presentation history: trim it before allowing a resize to
    // make the bounded preference document unreadable or block later edits.
    while preferences.validate().is_err() {
        let evict = preferences
            .windows
            .keys()
            .find(|k| **k != key && *k != "home" && *k != "shorts")
            .or_else(|| preferences.windows.keys().find(|k| **k != key))
            .cloned();
        if let Some(evict) = evict {
            preferences.windows.remove(&evict);
        } else {
            preferences.windows = previous;
            break;
        }
    }
}

#[derive(Clone, Copy)]
enum PickerAction {
    Open,
    Save { leave: bool, close: bool },
    ImportMedia,
    ImportLut,
    ExportSave,
    SnapshotSave,
}
struct Picker {
    future: std::pin::Pin<Box<dyn std::future::Future<Output = Option<Vec<rfd::FileHandle>>>>>,
    action: PickerAction,
    waker: std::task::Waker,
}
struct Repaint(egui::Context);
impl std::task::Wake for Repaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }
}
type EditorSlot = Arc<Mutex<Option<Box<Editor>>>>;
struct WorkingEditor {
    slot: EditorSlot,
    editor: Option<Box<Editor>>,
}
impl Drop for WorkingEditor {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = self.editor.take();
        }
    }
}
enum Work {
    Discover,
    Open(PathBuf),
    New,
    Short(Box<Session>, render::RenderSettings),
    Recover(Box<Candidate>),
    Discard(Box<Candidate>),
    Persist {
        editor: EditorSlot,
        path: Option<PathBuf>,
        leave: bool,
        close: bool,
        journal_only: bool,
    },
}
enum Completed {
    Discovery(Discovery),
    Opened(Box<Editor>),
    Discarded,
    Persisted {
        result: Result<()>,
        leave: bool,
        close: bool,
    },
}
struct Pending {
    id: JobId,
    receive: mpsc::Receiver<Result<Completed>>,
    text: Text,
    cancel: engine::jobs::CancelToken,
    editor: Option<EditorSlot>,
}
/// The editor is absent while its owner is performing an IO boundary on an engine
/// worker. Loading/exiting remains visible; no second Session or serializer exists.
pub struct EditorWindow {
    viewport: egui::ViewportId,
    pub editor: Option<Box<Editor>>,
    title: String,
    retained_models: Option<crate::model_manager::ModelManager>,
    focus_home: bool,
    project_window: bool,
    path: Option<PathBuf>,
    requested_open: Option<PathBuf>,
    pub preferences: Preferences,
    root: PathBuf,
    budget: ByteBudget,
    persistence: ByteBudget,
    runner: std::rc::Rc<std::cell::RefCell<JobRunner>>,
    sounds: Option<Arc<libraries::sounds::SoundLibrary>>,
    /// The process music session lent to this window's editor.
    music: Option<Arc<crate::services::music::MusicSession>>,
    /// The process media cache instance lent to this window's editor.
    media: Option<Arc<Mutex<engine::control::MediaAccess>>>,
    pending: Option<Pending>,
    picker: Option<Picker>,
    candidates: Vec<Candidate>,
    diagnostics: Vec<recovery::Error>,
    error: Option<Error>,
    retry_path: Option<PathBuf>,
    startup_path: Option<PathBuf>,
    close_prompt: bool,
    ready_to_close: bool,
    configured: bool,
    device_available: bool,
    retry_save: Option<(Option<PathBuf>, bool, bool, bool)>,
    retry_discovery: bool,
    preference_warning: bool,
    open_file_requested: bool,
    /// Identity the pending operation was issued under, when it is a Save As.
    save_as_from: Option<project::ProjectId>,
    /// Identity pair a successful Save As produced, waiting for the workspace
    /// owner to carry lane memory from the issuing project to the copy.
    saved_as: Option<(project::ProjectId, project::ProjectId)>,
}
impl EditorWindow {
    #[allow(clippy::too_many_arguments)]
    fn new(
        root: PathBuf,
        preferences: Preferences,
        budget: ByteBudget,
        persistence: ByteBudget,
        runner: std::rc::Rc<std::cell::RefCell<JobRunner>>,
        sounds: Option<Arc<libraries::sounds::SoundLibrary>>,
        music: Option<Arc<crate::services::music::MusicSession>>,
        media: Option<Arc<Mutex<engine::control::MediaAccess>>>,
    ) -> Self {
        static NEXT_WINDOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self {
            viewport: egui::ViewportId::from_hash_of((
                "editor.window",
                NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            )),
            editor: None,
            title: i18n::text(preferences.locale, Text::AppName).to_owned(),
            retained_models: None,
            focus_home: false,
            project_window: false,
            path: None,
            requested_open: None,
            preferences,
            root,
            budget,
            persistence,
            runner,
            sounds,
            music,
            media,
            pending: None,
            picker: None,
            candidates: Vec::new(),
            diagnostics: Vec::new(),
            error: None,
            retry_path: None,
            startup_path: None,
            close_prompt: false,
            ready_to_close: false,
            configured: false,
            device_available: false,
            retry_save: None,
            retry_discovery: false,
            preference_warning: false,
            open_file_requested: false,
            save_as_from: None,
            saved_as: None,
        }
    }
    /// A damaged layout preference document was replaced at startup.
    pub fn warn_preferences_reset(&mut self) {
        self.preference_warning = true;
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.picker.is_some()
    }
    /// Ask the pending lifecycle operation to stop. Publication may already have
    /// won the race; its completion decides whether the operation succeeded.
    pub fn cancel_pending(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.cancel();
        }
    }
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }
    pub fn ready_to_close(&self) -> bool {
        self.ready_to_close
    }
    pub fn request_close(&mut self) {
        if self.editor.is_some() || self.busy() {
            self.close_prompt = true;
        } else {
            self.ready_to_close = true;
        }
    }
    /// Device lifetime stays in the native adapter; replacement projects use the
    /// same admitted 48 kHz device contract as the shell.
    pub fn set_device_available(&mut self, available: bool) {
        self.device_available = available;
        if let Some(editor) = &mut self.editor {
            editor.playback.set_device_available(available);
        }
    }
    pub fn open(&mut self, path: PathBuf) -> Result<()> {
        if self.editor.is_some() || self.busy() {
            return Err(Error::Invalid("leave active project before opening"));
        }
        self.retry_path = Some(path.clone());
        self.submit(Work::Open(path), Text::LoadingProject)
    }
    pub fn create(&mut self) -> Result<()> {
        if self.editor.is_some() || self.busy() {
            return Err(Error::Invalid("leave active project before creating"));
        }
        self.retry_path = None;
        self.submit(Work::New, Text::LoadingProject)
    }
    /// A destination is supplied by the native picker, or the existing saved path
    /// is reused. Move the sole owner to a worker until its revision is acknowledged.
    pub fn persist(
        &mut self,
        path: Option<PathBuf>,
        leave: bool,
        close: bool,
        journal_only: bool,
    ) -> Result<()> {
        if self.busy() {
            return Err(Error::Invalid("project operation in progress"));
        }
        let editor = self.editor.as_mut().ok_or(Error::Invalid("no project"))?;
        self.title = editor.session.project().data().name.as_str().to_owned();
        self.preferences = editor.preferences.clone();
        editor.activate("transport.pause")?;
        let at = editor.playback.position()?;
        let view = editor.session.metadata().view;
        editor.session.apply(Command::SetViewState {
            zoom: view.zoom,
            scroll: view.scroll,
            playhead: at,
        })?;
        let id = editor.session.project().data().id;
        self.retry_save = Some((path.clone(), leave, close, journal_only));
        self.save_as_from = path.as_ref().map(|_| id);
        self.retained_models = editor.take_model_manager();
        let editor = self.editor.take().ok_or(Error::Invalid("no project"))?;
        let submitted = self.submit(
            Work::Persist {
                editor: Arc::new(Mutex::new(Some(editor))),
                path,
                leave,
                close,
                journal_only,
            },
            if leave || close {
                Text::ExitingProject
            } else {
                Text::SavingProject
            },
        );
        if submitted.is_err() {
            self.save_as_from = None;
        }
        submitted
    }
    fn pick(&mut self, ctx: &egui::Context, action: PickerAction) -> Result<()> {
        if self.busy() {
            return Err(Error::Invalid("project operation in progress"));
        }
        let future: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<Vec<rfd::FileHandle>>>>,
        > = match action {
            PickerAction::Open => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::AppName), &["vibecut", "json"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::Save { .. } => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::AppName), &["vibecut", "json"]);
                let file_name = self.tr(Text::DefaultProjectFileName);
                Box::pin(async move {
                    dialog
                        .set_file_name(file_name)
                        .save_file()
                        .await
                        .map(|file| vec![file])
                })
            }
            PickerAction::ExportSave => {
                let editor = self.editor.as_ref().ok_or(Error::Invalid("no project"))?;
                let format = editor.export_save_format();
                let (file_name, directory) = editor.export_save_defaults();
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title(i18n::text(editor.preferences.locale, Text::ExportTitle))
                    .add_filter(
                        i18n::text(editor.preferences.locale, Text::ExportFormat),
                        &[format.extension()],
                    )
                    .set_file_name(file_name);
                if let Some(directory) = directory {
                    dialog = dialog.set_directory(directory);
                }
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
            PickerAction::ImportLut => {
                let dialog = rfd::AsyncFileDialog::new().add_filter(self.tr(Text::Lut), &["cube"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::ImportMedia => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::Media), crate::import::MEDIA_EXTENSIONS);
                Box::pin(dialog.pick_files())
            }
            PickerAction::SnapshotSave => {
                let editor = self.editor.as_ref().ok_or(Error::Invalid("no project"))?;
                let (file_name, directory) = editor.snapshot_save_defaults();
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title(i18n::text(editor.preferences.locale, Text::SnapshotSave))
                    .add_filter(
                        i18n::text(editor.preferences.locale, Text::SnapshotFormat),
                        &["png"],
                    )
                    .set_file_name(file_name);
                if let Some(directory) = directory {
                    dialog = dialog.set_directory(directory);
                }
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
        };
        self.picker = Some(Picker {
            future,
            action,
            waker: std::task::Waker::from(Arc::new(Repaint(ctx.clone()))),
        });
        Ok(())
    }
    fn save_from_ui(
        &mut self,
        ctx: &egui::Context,
        leave: bool,
        close: bool,
        force_as: bool,
    ) -> Result<()> {
        let editor = self.editor.as_mut().ok_or(Error::Invalid("no project"))?;
        let needs_path = editor
            .session
            .autosave()
            .is_some_and(|s| s.saved_path().is_none());
        if force_as || needs_path {
            self.pick(ctx, PickerAction::Save { leave, close })
        } else {
            self.persist(None, leave, close, false)
        }
    }
    /// Complete the real LUT picker without opening a native window in tests.
    pub fn lut_picked_file(&mut self, path: Option<PathBuf>) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .lut_picked_file(path)
    }
    /// Apply native picker result paths through the same import path as drops.
    /// This OS boundary accepts injected results without opening a native dialog.
    pub fn import_picked_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .start_import(paths)
    }
    /// Apply a save-picker destination to the existing export dialog.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .export_picked_file(path)
    }
    pub fn export_save_cancelled(&mut self) {
        if let Some(editor) = &mut self.editor {
            editor.export_save_cancelled();
        }
    }
    /// Apply the native snapshot picker's destination through `preview.snapshot`.
    pub fn snapshot_picked_file(&mut self, path: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .snapshot_picked_file(path)
    }
    /// Cancelling the native snapshot dialog issues no command.
    pub fn snapshot_save_cancelled(&mut self) {
        self.picker = self
            .picker
            .take()
            .filter(|picker| !matches!(picker.action, PickerAction::SnapshotSave));
        if let Some(editor) = &mut self.editor {
            editor.snapshot_save_cancelled();
        }
    }
    fn poll_picker(&mut self) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        match picker
            .future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&picker.waker))
        {
            std::task::Poll::Pending => self.picker = Some(picker),
            std::task::Poll::Ready(None) => {
                if matches!(picker.action, PickerAction::ImportLut)
                    && let Err(error) = self.lut_picked_file(None)
                {
                    self.error = Some(error);
                }
            }
            std::task::Poll::Ready(Some(mut files)) => {
                let result = match picker.action {
                    PickerAction::Open => match files.pop() {
                        Some(file) => {
                            self.requested_open = Some(file.path().to_owned());
                            Ok(())
                        }
                        None => Ok(()),
                    },
                    PickerAction::Save { leave, close } => match files.pop() {
                        Some(file) => {
                            self.persist(Some(file.path().to_owned()), leave, close, false)
                        }
                        None => Ok(()),
                    },
                    PickerAction::ExportSave => match files.pop() {
                        Some(file) => self.export_picked_file(file.path().to_owned()),
                        None => {
                            self.export_save_cancelled();
                            Ok(())
                        }
                    },
                    PickerAction::SnapshotSave => match files.pop() {
                        Some(file) => self.snapshot_picked_file(file.path().to_owned()),
                        None => {
                            self.snapshot_save_cancelled();
                            Ok(())
                        }
                    },
                    PickerAction::ImportLut => {
                        self.lut_picked_file(files.pop().map(|file| file.path().to_owned()))
                    }
                    PickerAction::ImportMedia => {
                        let paths = files
                            .into_iter()
                            .map(|file| file.path().to_owned())
                            .collect();
                        self.import_picked_files(paths)
                    }
                };
                if let Err(e) = result {
                    self.error = Some(e);
                }
            }
        }
    }
    fn submit(&mut self, work: Work, text: Text) -> Result<()> {
        if self.editor.is_some() && matches!(&work, Work::Recover(_) | Work::Open(_) | Work::New) {
            return Err(Error::Invalid("leave active project before replacing it"));
        }
        self.retry_discovery =
            matches!(&work, Work::Discover | Work::Discard(_) | Work::Recover(_));
        let editor = match &work {
            Work::Persist { editor, .. } => Some(editor.clone()),
            _ => None,
        };
        let (send, receive) = mpsc::channel();
        let root = self.root.clone();
        let budget = if matches!(
            &work,
            Work::Open(_) | Work::New | Work::Recover(_) | Work::Short(..)
        ) {
            self.budget.child(512 << 20)?
        } else {
            self.budget.clone()
        };
        let persistence = self.persistence.child(64 << 20)?;
        let input = Arc::new(Mutex::new(Some(work)));
        let preferences = self.preferences.clone();
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "desktop",
            model: "project-lifecycle",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let worker_input = input.clone();
        let submitted = self.runner.borrow_mut().submit::<engine::control::Error>(
            JobSpec {
                kind: JobKind::Persist,
                class: ResourceClass::Cpu,
                input: key,
                bytes: JobBytes {
                    compositor: 48 << 20,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                let work = worker_input
                    .lock()
                    .map_err(|_| engine::RuntimeError::Poisoned)?
                    .take()
                    .ok_or(EngineError::Edit(engine::EditError::Invalid(
                        "missing lifecycle work",
                    )))?;
                let result = run(
                    work,
                    &root,
                    budget.clone(),
                    persistence.clone(),
                    preferences.clone(),
                );
                send.send(result)
                    .map_err(|_| EngineError::Runtime(engine::RuntimeError::Cancelled))?;
                Ok(Vec::new())
            },
        );
        let (id, cancel) = match submitted {
            Ok(job) => job,
            Err(e) => {
                if let Some(editor) = &editor {
                    self.editor = editor.lock().map_err(|_| Error::Poisoned)?.take();
                }
                return Err(e.into());
            }
        };
        self.pending = Some(Pending {
            id,
            receive,
            text,
            cancel,
            editor,
        });
        self.error = None;
        Ok(())
    }
    pub fn tick(&mut self) {
        self.poll_picker();
        if let Some(editor) = &mut self.editor {
            editor.tick();
            self.title = editor.session.project().data().name.as_str().to_owned();
            if self.error.is_none()
                && let Some(error) = &editor.persistence_error
            {
                self.error = Some(Error::Worker(error.clone()));
            }
        }
        self.poll_pending();
    }
    fn poll_pending(&mut self) {
        if let Err(e) = self.runner.borrow_mut().tick() {
            self.error = Some(e.into());
            return;
        }
        // Completion snapshots remain until their owning window consumes them.
        let Some(pending) = &self.pending else {
            return;
        };
        let Some(snapshot) = self.runner.borrow().snapshot(pending.id) else {
            return;
        };
        if !matches!(
            snapshot.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let save_as_from = self.save_as_from.take();
        let _ = self.runner.borrow_mut().take_output(pending.id);
        if let Some(editor) = &pending.editor {
            match editor.lock() {
                Ok(mut editor) => self.editor = editor.take(),
                Err(_) => {
                    self.error = Some(Error::Poisoned);
                    return;
                }
            }
        }
        let result =
            pending.receive.try_recv().unwrap_or_else(|_| {
                Err(snapshot.error.map(Error::Worker).unwrap_or(Error::Engine(
                    EngineError::Runtime(engine::RuntimeError::WorkerPanicked),
                )))
            });
        // A Save As whose publication already completed is a successful Save As
        // even when the cancel arrives afterwards: the engine has rekeyed the
        // editor to the copy, so dropping the completion would leave the window
        // on the new id without carrying its lanes. A cancel before publication
        // produced no completed Persist result and still changes nothing.
        let published_save_as = save_as_from.is_some()
            && matches!(&result, Ok(Completed::Persisted { result: Ok(()), .. }));
        if pending.cancel.cancelled() && !published_save_as {
            self.startup_path = None;
            self.retry_path = None;
            return;
        }
        match result {
            Ok(Completed::Discovery(discovery)) => {
                self.candidates = discovery.candidates;
                self.diagnostics = discovery.diagnostics;
            }
            Ok(Completed::Opened(mut editor)) => {
                editor.playback.set_device_available(self.device_available);
                if let Some(sounds) = &self.sounds {
                    let _ = editor.set_sound_library(sounds.clone());
                }
                if let Some(music) = &self.music {
                    let _ = editor.set_music_session(music.clone());
                }
                if let Some(media) = &self.media {
                    editor.set_media(media.clone());
                }
                self.editor = Some(editor);
                self.retry_path = None;
                self.startup_path = None;
            }
            Ok(Completed::Discarded) => {}
            Ok(Completed::Persisted {
                result,
                leave,
                close,
            }) => match result {
                Ok(()) => {
                    if let Some(from) = save_as_from
                        && let Some(editor) = &self.editor
                    {
                        let to = editor.session.project().data().id;
                        if from != to {
                            self.saved_as = Some((from, to));
                        }
                    }
                    if let Some(editor) = &self.editor
                        && let crate::lifecycle::EditorSession::Persistent(autosave) =
                            &editor.session
                    {
                        self.path = autosave.saved_path().map(ToOwned::to_owned);
                    }
                    self.retry_save = None;
                    if leave || close {
                        self.editor = None;
                    }
                    if close {
                        self.ready_to_close = true;
                    }
                    if leave || close {
                        self.close_prompt = false;
                    }
                }
                Err(e) => self.error = Some(e),
            },
            Err(e) => self.error = Some(e),
        }
    }
    fn tr(&self, text: Text) -> &'static str {
        i18n::text(self.preferences.locale, text)
    }
    fn button(&self, ui: &mut egui::Ui, id: &'static str, icon: Icon, text: Text) -> bool {
        let image = icons::tinted(icon, 16.0, ui.visuals().text_color());
        let response = ui
            .push_id(id, |ui| {
                ui.add_sized(
                    [160.0, 34.0],
                    egui::Button::image_and_text(image, self.tr(text)),
                )
            })
            .inner;
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(id));
        response.clicked()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            Editor::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.configured = true;
        }
        Editor::theme(ui.ctx(), self.preferences.dark);
        if self.project_window {
            self.tick();
        }
        if self.open_file_requested && !self.busy() {
            self.open_file_requested = false;
            if let Err(error) = self.pick(ui.ctx(), PickerAction::Open) {
                self.error = Some(error);
            }
        }
        if self.project_window
            && ui.input(|i| i.viewport().close_requested())
            && !self.ready_to_close
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close();
        }
        if let Some(editor) = &mut self.editor {
            editor.overlay_open = self.close_prompt
                || self.error.is_some()
                || self.picker.is_some()
                || self.pending.is_some()
                || self.preference_warning
                || !self.diagnostics.is_empty();
            editor.ui(ui);
            self.title = editor.session.project().data().name.as_str().to_owned();
            self.preferences = editor.preferences.clone();
            if let Some(action) = editor.lifecycle_action.take() {
                let result = match action {
                    EditorAction::ExportSave => self.pick(ui.ctx(), PickerAction::ExportSave),
                    EditorAction::SnapshotSave => self.pick(ui.ctx(), PickerAction::SnapshotSave),
                    EditorAction::ImportMedia => self.pick(ui.ctx(), PickerAction::ImportMedia),
                    EditorAction::ImportLut => self.pick(ui.ctx(), PickerAction::ImportLut),
                    EditorAction::FocusHome => {
                        self.focus_home = true;
                        Ok(())
                    }
                    EditorAction::SaveAs => self.save_from_ui(ui.ctx(), false, false, true),
                    EditorAction::Save => self.save_from_ui(ui.ctx(), false, false, false),
                };
                if let Err(e) = result {
                    self.error = Some(e);
                }
            }
        } else if let Some(pending) = self.pending.as_ref() {
            let text = pending.text;
            let mut cancel = false;
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading(egui::RichText::new(self.tr(Text::AppName)).size(28.0));
                ui.add_space(28.0);
                ui.spinner();
                ui.label(self.tr(text));
                if matches!(text, Text::LoadingProject)
                    && self.button(ui, "project.operation.cancel", Icon::X, Text::Cancel)
                {
                    cancel = true;
                }
            });
            if cancel {
                self.cancel_pending();
            }
        } else if self.picker.is_some() {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading(egui::RichText::new(self.tr(Text::AppName)).size(28.0));
                ui.add_space(28.0);
                ui.label(self.tr(Text::ChooseProject));
            });
        }
        if self.editor.is_none() && !self.busy() && !self.candidates.is_empty() {
            egui::Modal::new(egui::Id::unique("recovery.prompt")).show(ui.ctx(), |ui| {
                ui.heading(self.tr(Text::Recovery));
                if let Some(candidate) = self.candidates.first() {
                    ui.label(&candidate.journal.name);
                }
                ui.label(self.tr(Text::SaveAsNeeded));
                ui.horizontal(|ui| {
                    if self.button(
                        ui,
                        "recovery.recover",
                        Icon::ArrowCounterClockwise,
                        Text::Recover,
                    ) {
                        let candidate = self.candidates.remove(0);
                        if let Err(e) =
                            self.submit(Work::Recover(Box::new(candidate)), Text::LoadingProject)
                        {
                            self.error = Some(e);
                        }
                    }
                    if self.button(ui, "recovery.discard", Icon::Trash, Text::Discard) {
                        let candidate = self.candidates.remove(0);
                        if let Err(e) =
                            self.submit(Work::Discard(Box::new(candidate)), Text::Discovering)
                        {
                            self.error = Some(e);
                        }
                    }
                });
            });
        }
        if !self.diagnostics.is_empty() {
            egui::Window::new(self.tr(Text::RecoveryDiagnostic))
                .id(egui::Id::unique("recovery.diagnostic"))
                .show(ui.ctx(), |ui| {
                    for e in &self.diagnostics {
                        ui.label(e.to_string());
                    }
                    if self.button(ui, "recovery.diagnostic.close", Icon::X, Text::Cancel) {
                        self.diagnostics.clear();
                    }
                });
        }
        if self.preference_warning {
            egui::Window::new(self.tr(Text::PreferencesWarning))
                .id(egui::Id::unique("preferences.warning"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(self.tr(Text::PreferencesReset));
                    if self.button(ui, "preferences.warning.close", Icon::X, Text::Ok) {
                        self.preference_warning = false;
                    }
                });
        }
        if self.error.is_some() {
            self.close_prompt = false;
            egui::Window::new(self.tr(Text::ProjectError))
                .id(egui::Id::unique("project.error"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    let text = match self.error.as_ref() {
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::TooNew { .. },
                        ))))
                        | Some(Error::Project(project::Error::TooNew { .. })) => Text::TooNew,
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::TooOld { .. },
                        ))))
                        | Some(Error::Project(project::Error::TooOld { .. })) => Text::TooOld,
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::Busy,
                        ))))
                        | Some(Error::Project(project::Error::Busy)) => Text::Busy,
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::Conflict,
                        ))))
                        | Some(Error::Project(project::Error::Conflict)) => Text::Conflict,
                        _ => Text::ProjectError,
                    };
                    ui.label(self.tr(text));
                    if let Some(e) = &self.error {
                        ui.label(e.to_string());
                        let mut source = std::error::Error::source(e);
                        for _ in 0..8 {
                            let Some(error) = source else {
                                break;
                            };
                            ui.label(error.to_string());
                            source = error.source();
                        }
                    }
                    ui.horizontal(|ui| {
                        if self.button(
                            ui,
                            "project.retry",
                            Icon::ArrowClockwise,
                            Text::RetryProject,
                        ) {
                            self.error = None;
                            let result = if let Some((path, leave, close, journal_only)) =
                                self.retry_save.take()
                            {
                                self.persist(path, leave, close, journal_only)
                            } else if self.retry_discovery {
                                self.candidates.clear();
                                self.submit(Work::Discover, Text::Discovering)
                            } else if let Some(path) = self.retry_path.clone() {
                                self.open(path)
                            } else if self.editor.is_none() {
                                self.create()
                            } else {
                                Ok(())
                            };
                            if let Err(e) = result {
                                self.error = Some(e);
                            }
                        }
                        if self.button(
                            ui,
                            "project.error.back",
                            Icon::ArrowLeft,
                            if self.editor.is_some() {
                                Text::Cancel
                            } else {
                                Text::BackProjects
                            },
                        ) {
                            self.error = None;
                            self.retry_path = None;
                            self.startup_path = None;
                        }
                    });
                });
        }
        if self.close_prompt && !self.busy() {
            egui::Modal::new(egui::Id::unique("project.close.prompt")).show(ui.ctx(), |ui| {
                ui.heading(self.tr(Text::ClosePrompt));
                if self.button(ui, "project.close.save", Icon::FloppyDisk, Text::SaveClose)
                    && let Err(e) = self.save_from_ui(ui.ctx(), true, true, false)
                {
                    self.error = Some(e);
                }
                if self.button(ui, "project.close.recover", Icon::File, Text::Close)
                    && let Err(e) = self.persist(None, true, true, true)
                {
                    self.error = Some(e);
                }
                if self.button(ui, "project.close.cancel", Icon::X, Text::Cancel) {
                    self.close_prompt = false;
                }
            });
        }
        if self.ready_to_close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}
fn session(root: &std::path::Path, session: Session, budget: ByteBudget) -> Result<Autosave> {
    Ok(Autosave::new(
        root,
        session,
        budget,
        Arc::new(SystemClock::default()),
        Schedule::default(),
    )?)
}
fn run(
    work: Work,
    root: &std::path::Path,
    budget: ByteBudget,
    persistence: ByteBudget,
    preferences: Preferences,
) -> Result<Completed> {
    let autosave = match work {
        Work::Discover => {
            std::fs::create_dir_all(root)?;
            return Ok(Completed::Discovery(recovery::discover(root)?));
        }
        Work::Discard(candidate) => {
            candidate.discard()?;
            return Ok(Completed::Discarded);
        }
        Work::Recover(candidate) => {
            let mut autosave = session(root, candidate.recover(100)?, persistence.clone())?;
            // Publish the recovered copy before retiring its source journal.
            autosave.flush()?;
            candidate.discard()?;
            autosave
        }
        Work::New => session(
            root,
            fresh(i18n::text(preferences.locale, Text::NewProject))?,
            persistence.clone(),
        )?,
        Work::Short(short, settings) => {
            let autosave = session(root, *short, persistence.clone())?;
            let mut playback = Playback::new(autosave.session(), budget, Arc::new(Files))?;
            playback.set_render_settings(settings)?;
            return Ok(Completed::Opened(Box::new(Editor::persisted(
                autosave,
                playback,
                preferences,
            ))));
        }
        Work::Open(path) => match Autosave::open(
            root,
            &path,
            100,
            persistence.clone(),
            Arc::new(SystemClock::default()),
            Schedule::default(),
        ) {
            Ok(open) => open,
            Err(recovery::Error::Edit(engine::EditError::Project(project::Error::Io(e))))
                if e.kind() == std::io::ErrorKind::NotFound
                    && path.parent().is_some_and(|parent| parent.is_dir()) =>
            {
                session(
                    root,
                    fresh(i18n::text(preferences.locale, Text::UntitledProject))?,
                    persistence.clone(),
                )?
            }
            Err(e) => return Err(e.into()),
        },
        Work::Persist {
            editor,
            path,
            leave,
            close,
            journal_only,
        } => {
            let owned = editor
                .lock()
                .map_err(|_| Error::Poisoned)?
                .take()
                .ok_or(Error::Invalid("missing editor"))?;
            let mut active = WorkingEditor {
                slot: editor,
                editor: Some(owned),
            };
            let result = (|| -> Result<()> {
                let editor = active
                    .editor
                    .as_mut()
                    .ok_or(Error::Invalid("missing editor"))?;
                let autosave = editor
                    .session
                    .autosave()
                    .ok_or(Error::Unsupported("nonpersistent editor"))?;
                autosave.flush()?;
                if !journal_only {
                    if let Some(path) = path {
                        autosave.save_as(path)?;
                    } else {
                        autosave.apply(Command::SaveProject {})?;
                    }
                    autosave.flush()?;
                    if autosave.saving() || autosave.is_dirty() {
                        return Err(EngineError::Session(engine::SessionError::StaleResult).into());
                    }
                }
                if leave || close {
                    autosave.apply(Command::ThumbnailProject {})?;
                }
                Ok(())
            })();
            if result.is_ok() && (leave || close) {
                drop(active.editor.take());
            }
            return Ok(Completed::Persisted {
                result,
                leave,
                close,
            });
        }
    };
    let view = autosave.session().metadata().view;
    let mut playback = Playback::new(autosave.session(), budget, Arc::new(Files))?;
    playback.seek(
        view.playhead.min(autosave.session().project().duration()),
        false,
        autosave.session().project().clone(),
    )?;
    let mut editor = Editor::persisted(autosave, playback, preferences);
    editor.set_thumbnail_root(root.parent().unwrap_or(root).join("thumbnails"));
    Ok(Completed::Opened(Box::new(editor)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::Instant;
    #[test]
    fn geometry_history_yields_to_the_preference_byte_bound_and_preserves_normal_size() {
        let mut preferences = Preferences::default();
        for index in 0..128 {
            preferences.sections.insert(format!("{index:064}"), false);
        }
        for index in 0..64 {
            let key = format!("{index:064}");
            preferences.windows.insert(key.clone(), Default::default());
            if preferences.validate().is_err() {
                preferences.windows.remove(&key);
                break;
            }
        }
        let sections = preferences.sections.clone();
        let before = preferences.windows.len();
        let key = "x".repeat(64);
        let geometry = crate::preferences::WindowGeometry {
            size: [12345.678, 12345.678],
            position: Some([-12345.678, -23_456.79]),
            maximized: false,
        };
        remember_geometry(&mut preferences, key.clone(), geometry.clone());
        assert!(preferences.validate().is_ok());
        assert_eq!(preferences.sections, sections);
        assert_eq!(preferences.windows.get(&key), Some(&geometry));
        assert!(
            preferences.windows.len() <= before,
            "history eviction must create room for the new geometry"
        );
        let maximized = crate::preferences::WindowGeometry {
            size: [16000.0, 16000.0],
            position: Some([0.0, 0.0]),
            maximized: true,
        };
        remember_geometry(&mut preferences, key.clone(), maximized);
        let restored = preferences.windows.get(&key).unwrap();
        assert_eq!(restored.size, geometry.size);
        assert_eq!(restored.position, geometry.position);
        assert!(restored.maximized);
    }
    #[test]
    fn startup_opens_once_after_both_discovery_and_catalog_are_ready() {
        for catalog_first in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("startup.json");
            let project =
                project::Project::empty(project::ProjectId(42), "startup".into()).unwrap();
            let bytes = project::encode(&project, true).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            let mut workspace = Workspace::new(
                root.path().join("recovery"),
                Preferences::default(),
                Some(path.clone()),
            )
            .unwrap();
            let release_catalog = workspace.projects.defer_initial_catalog();
            let discovery = workspace.home.pending.as_ref().unwrap().id;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                workspace.home.runner.borrow_mut().tick().unwrap();
                if workspace
                    .home
                    .runner
                    .borrow()
                    .snapshot(discovery)
                    .unwrap()
                    .state
                    == JobState::Completed
                {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            if catalog_first {
                release_catalog();
                workspace.tick();
            } else {
                workspace.tick();
                assert!(workspace.error().is_none(), "{:?}", workspace.error());
                assert_eq!(workspace.home.startup_path.as_ref(), Some(&path));
                assert!(workspace.home.pending.is_none());
                assert!(workspace.focused_editor().is_none());
                release_catalog();
            }
            while workspace.focused_editor().is_none() || workspace.busy() {
                workspace.tick();
                assert!(workspace.error().is_none(), "{:?}", workspace.error());
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(
                workspace
                    .focused_editor()
                    .unwrap()
                    .session
                    .project()
                    .data()
                    .id,
                project::ProjectId(42)
            );
            assert!(workspace.home.startup_path.is_none());
            workspace.tick();
            assert!(workspace.error().is_none(), "{:?}", workspace.error());
            assert!(workspace.home.pending.is_none());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn unwind_returns_the_active_editor_before_reporting_worker_failure() {
        for mode in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let budget = ByteBudget::new(128 << 20).unwrap();
            let runner = JobRunner::new(
                RunnerConfig {
                    concurrency: [1; 3],
                    queue: 2,
                    retained: 2,
                    network_attempts: 1,
                    backoff: Duration::from_millis(1),
                    progress_interval: Duration::from_millis(10),
                },
                budget.clone(),
                Arc::new(SystemClock::default()),
            )
            .unwrap();
            let runner = std::rc::Rc::new(std::cell::RefCell::new(runner));
            let mut window = EditorWindow::new(
                root.path().to_owned(),
                Preferences::default(),
                budget.clone(),
                budget.clone(),
                runner,
                None,
                None,
                None,
            );
            if mode == 2 {
                window
                    .submit(
                        Work::Persist {
                            editor: Arc::new(Mutex::new(None)),
                            path: None,
                            leave: false,
                            close: false,
                            journal_only: false,
                        },
                        Text::Discovering,
                    )
                    .unwrap();
            } else {
                let session = Session::new(
                    project::Project::empty(project::ProjectId(1), "retained".into()).unwrap(),
                    10,
                )
                .unwrap();
                let playback = Playback::new(
                    &session,
                    ByteBudget::new(64 << 20).unwrap(),
                    Arc::new(Files),
                )
                .unwrap();
                let slot = Arc::new(Mutex::new(Some(Box::new(Editor::new(
                    session,
                    playback,
                    Preferences::default(),
                )))));
                let worker_slot = slot.clone();
                let (send, receive) = mpsc::channel::<Result<Completed>>();
                drop(send);
                let key = CacheKey::new(KeyInputs {
                    kind: ArtifactKind::Preview,
                    source: [0; 32],
                    provider: "fixture",
                    model: "window",
                    version: "1",
                    settings: &[],
                    transcript: [0; 32],
                    cues: [0; 32],
                    clip_range: [0; 32],
                    translation: [0; 32],
                })
                .unwrap();
                let (id, cancel) = window
                    .runner
                    .borrow_mut()
                    .submit::<engine::control::Error>(
                        JobSpec {
                            kind: JobKind::Persist,
                            class: ResourceClass::Cpu,
                            input: key,
                            bytes: JobBytes {
                                compositor: 64,
                                result: 16,
                                ..Default::default()
                            },
                        },
                        move |_| {
                            let editor = worker_slot.lock().unwrap().take();
                            let _active = WorkingEditor {
                                slot: worker_slot.clone(),
                                editor,
                            };
                            if mode == 0 {
                                panic!("injected while the worker owns the editor");
                            }
                            Ok(Vec::new())
                        },
                    )
                    .unwrap();
                window.pending = Some(Pending {
                    id,
                    receive,
                    text: Text::Discovering,
                    cancel,
                    editor: Some(slot),
                });
            }
            let id = window.pending.as_ref().unwrap().id;
            let deadline = Instant::now() + Duration::from_secs(10);
            while window.pending.is_some() {
                window.poll_pending();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let snapshot = window.runner.borrow().snapshot(id).unwrap();
            match window.error.as_ref().unwrap() {
                Error::Worker(cause) if mode == 0 => {
                    assert!(Arc::ptr_eq(cause, snapshot.error.as_ref().unwrap()));
                    assert!(matches!(
                        (cause.as_ref() as &(dyn std::error::Error + 'static))
                            .downcast_ref::<engine::RuntimeError>(),
                        Some(engine::RuntimeError::WorkerPanicked)
                    ));
                    assert_eq!(snapshot.state, JobState::Failed);
                }
                Error::Engine(EngineError::Runtime(engine::RuntimeError::WorkerPanicked))
                    if mode == 1 =>
                {
                    assert_eq!(snapshot.state, JobState::Completed);
                    assert!(snapshot.error.is_none());
                }
                Error::Invalid("missing editor") if mode == 2 => {
                    assert_eq!(snapshot.state, JobState::Completed);
                    assert!(window.editor.is_none());
                }
                error => panic!("lost the original completion diagnostic: {error:?}"),
            }
            if mode != 2 {
                assert_eq!(
                    window
                        .editor
                        .as_ref()
                        .unwrap()
                        .session
                        .project()
                        .data()
                        .name
                        .as_str(),
                    "retained"
                );
            }
            assert!(window.runner.borrow_mut().take_output(id).is_none());
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
    }
    #[derive(Default)]
    struct ManualClock(std::sync::atomic::AtomicU64);
    impl engine::jobs::Clock for ManualClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(std::sync::atomic::Ordering::SeqCst))
        }
    }
    #[test]
    fn outstanding_picker_keeps_autosave_ticking_and_cancel_preserves_the_editor() {
        let root = tempfile::tempdir().unwrap();
        let mut workspace =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while workspace.busy() {
            workspace.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let clock = Arc::new(ManualClock::default());
        let session = fresh("picker test").unwrap();
        let autosave = Autosave::new(
            &workspace.home.root,
            session,
            workspace.home.budget.clone(),
            clock.clone(),
            Schedule::default(),
        )
        .unwrap();
        let playback = Playback::new(
            autosave.session(),
            workspace.home.budget.clone(),
            Arc::new(Files),
        )
        .unwrap();
        workspace.home.editor = Some(Box::new(Editor::persisted(
            autosave,
            playback,
            Preferences::default(),
        )));
        let thumbnails = workspace
            .home
            .root
            .parent()
            .unwrap_or(&workspace.home.root)
            .join("thumbnails");
        if let Some(editor) = &mut workspace.home.editor {
            editor.set_thumbnail_root(thumbnails);
        }
        workspace.tick();
        workspace.focused_window_mut().unwrap().picker = Some(Picker {
            future: Box::pin(std::future::pending()),
            action: PickerAction::Save {
                leave: false,
                close: false,
            },
            waker: std::task::Waker::noop().clone(),
        });
        clock.0.store(800, std::sync::atomic::Ordering::SeqCst);
        loop {
            workspace.tick();
            if workspace
                .focused_editor_mut()
                .unwrap()
                .session
                .autosave()
                .unwrap()
                .journaled_revision()
                > 0
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(workspace.busy());
        workspace
            .focused_window_mut()
            .unwrap()
            .picker
            .as_mut()
            .unwrap()
            .future = Box::pin(std::future::ready(None));
        workspace.tick();
        assert!(!workspace.busy());
        assert_eq!(
            workspace
                .focused_editor()
                .unwrap()
                .session
                .project()
                .data()
                .name
                .as_str(),
            "picker test"
        );
    }
    #[test]
    fn cancelled_lut_picker_releases_capture_without_project_edit() {
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 120, 200, 255]))
            .save(&path)
            .unwrap();
        let mut data = project::Project::empty(project::ProjectId(410), "LUT cancel".into())
            .unwrap()
            .into_data();
        data.assets
            .push(media::SourceAsset::import(media::AssetId(1), path).unwrap());
        data.scene_mut().timeline.main.items.push(project::Item {
            id: project::ItemId(2),
            source_asset: Some(media::AssetId(1)),
            source_range: time::SourceRange::new(
                time::Tick::ZERO,
                time::Tick(time::TICKS_PER_SECOND),
            )
            .unwrap(),
            start: time::Tick::ZERO,
            content: project::ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.,
                muted: false,
                crop: None,
            }),
            effects: vec![],
            masks: vec![],
            animations: Default::default(),
            retime: Default::default(),
            blend_mode: Default::default(),
            hidden: false,
        });
        let session = engine::Session::new(project::Project::new(data).unwrap(), 20).unwrap();
        let playback = Playback::new(
            &session,
            ByteBudget::new(768 << 20).unwrap(),
            Arc::new(Files),
        )
        .unwrap();
        let mut editor = Editor::new(session, playback, Preferences::default());
        editor
            .preferences
            .properties_tabs
            .insert("image".into(), "adjust".into());
        editor.selection = [project::ItemId(2)].into_iter().collect();
        let mut workspace =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        let id = workspace.register_editor(Box::new(editor));
        let window = workspace.windows.remove(&id).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440., 900.))
            .build_ui_state(
                |ui, window: &mut EditorWindow| window.editor.as_mut().unwrap().ui(ui),
                window,
            );
        harness.run_steps(3);
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("inspector.tab.adjust")))
            .click();
        harness.run_steps(2);
        let before = harness
            .state()
            .editor
            .as_ref()
            .unwrap()
            .session
            .project()
            .clone();
        let revision = harness.state().editor.as_ref().unwrap().session.revision();
        let history = harness
            .state()
            .editor
            .as_ref()
            .unwrap()
            .session
            .history_len();
        let import =
            || By::new().predicate(|node| node.author_id() == Some("inspector.lut.import"));
        // The grade rows above the LUT push its import button below the
        // inspector's visible area at this window size; wheel-scroll it in.
        for _ in 0..16 {
            let bounds = harness.get(import()).accesskit_node().raw_bounds().unwrap();
            if bounds.y1 < 480.0 {
                break;
            }
            harness.hover_at(egui::pos2(1200.0, 300.0));
            harness.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(8);
        }
        harness.get(import()).click();
        harness.run_steps(2);
        assert_eq!(
            harness
                .state_mut()
                .editor
                .as_mut()
                .unwrap()
                .take_lifecycle_action(),
            Some(EditorAction::ImportLut)
        );
        assert!(harness.get(import()).accesskit_node().is_disabled());
        harness.state_mut().picker = Some(Picker {
            future: Box::pin(std::future::ready(None)),
            action: PickerAction::ImportLut,
            waker: std::task::Waker::noop().clone(),
        });
        harness.state_mut().poll_picker();
        harness.run_steps(2);
        assert!(!harness.get(import()).accesskit_node().is_disabled());
        let editor = harness.state().editor.as_ref().unwrap();
        assert_eq!(editor.session.project(), &before);
        assert_eq!(editor.session.revision(), revision);
        assert_eq!(editor.session.history_len(), history);
    }
    #[test]
    fn lane_preferences_are_dropped_only_for_projects_whose_file_is_gone() {
        let root = tempfile::tempdir().unwrap();
        let mut workspace =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while workspace.busy() {
            workspace.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let kept_file = root.path().join("opened-from-file.vibecut");
        std::fs::write(&kept_file, b"x").unwrap();
        let id = project::ProjectId;
        workspace.lane_paths.insert(id(1), kept_file);
        workspace
            .lane_paths
            .insert(id(2), root.path().join("deleted.vibecut"));
        for project in [1, 2, 3] {
            workspace
                .preferences
                .timeline_lanes
                .insert(id(project), BTreeSet::from([7]));
        }
        workspace.lane_prune_key = vec![id(99)];
        workspace.prune_lane_preferences();
        // 1: file-opened project, file exists, window closed -> kept.
        // 2: file gone -> dropped. 3: never seen this session -> kept.
        let remaining: Vec<_> = workspace
            .preferences
            .timeline_lanes
            .keys()
            .copied()
            .collect();
        assert_eq!(remaining, vec![id(1), id(3)]);
    }
}
