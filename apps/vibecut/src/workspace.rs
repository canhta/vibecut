//! Nonblocking desktop lifecycle and recovery presentation over engine persistence.
use crate::{
    Editor, Error, Result,
    catalog::{self, Preferences, Text},
    egui,
    home::{Action as HomeAction, State as HomeState},
    kit::{self as icons, Icon},
    lifecycle::EditorAction,
    playback::{Files, Playback},
};
use engine::{
    ByteBudget, Command, EditError, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
        SystemClock,
    },
    recovery::{self, Autosave, Candidate, Discovery, Schedule},
};
use std::{
    collections::BTreeMap,
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
    home: EditorWindow,
    focused: Option<project::ProjectId>,
    focus_requested: Option<project::ProjectId>,
    home_open: bool,
    shorts_window: bool,
    models: Option<crate::model_manager::ModelManager>,
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
        );
        home.startup_path = path;
        home.submit(Work::Discover, Text::Discovering)?;
        Ok(Self {
            windows: BTreeMap::new(),
            projects: HomeState::new(projects_root, budget),
            shorts: None,
            preferences,
            home,
            focused: None,
            focus_requested: None,
            home_open: true,
            shorts_window: false,
            models: None,
        })
    }
    pub fn control_budget(&self) -> ByteBudget {
        self.home.budget.clone()
    }
    /// Register an already-composed editor, retaining its writer lease and
    /// resource policy. Repeated project identities focus their existing window.
    pub fn register_editor(&mut self, editor: Box<Editor>) -> project::ProjectId {
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
    pub fn viewport(id: project::ProjectId) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("editor.window", id))
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
                self.preferences.clone(),
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
            if let Some(models) = window.retained_models.take() {
                self.models = Some(models);
            } else if let Some(editor) = &mut window.editor
                && let Some(models) = editor.take_model_manager()
            {
                self.models = Some(models);
            }
        }
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
                if let Some(action) = self.projects.show(ui, &mut self.preferences, status) {
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
            let title = self
                .windows
                .get(&id)
                .map(|window| window.title().to_owned())
                .unwrap_or_else(|| {
                    catalog::text(self.preferences.locale, Text::AppName).to_owned()
                });
            let geometry = self
                .preferences
                .windows
                .get(&format!("editor.{}", id.0))
                .cloned()
                .unwrap_or_default();
            ctx.show_viewport_immediate(
                Self::viewport(id),
                geometry.builder().with_title(title),
                |ui, _| {
                    if ui.input(|i| i.focused) {
                        self.focused = Some(id);
                    }
                    self.window_ui(id, ui);
                    if let Some(geometry) = crate::catalog::WindowGeometry::read(ui) {
                        remember_geometry(
                            &mut self.preferences,
                            format!("editor.{}", id.0),
                            geometry,
                        );
                    }
                },
            );
        }
        if let Some(id) = self.focus_requested.take() {
            ctx.send_viewport_cmd_to(Self::viewport(id), egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(Self::viewport(id), egui::ViewportCommand::Focus);
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
                    .unwrap_or(crate::catalog::WindowGeometry {
                        size: crate::shorts::WINDOW,
                        ..Default::default()
                    })
                    .builder()
                    .with_title(catalog::text(shorts.preferences.locale, Text::Shorts)),
                |ui, _| {
                    actions = crate::shorts::show(shorts, &cx, ui);
                    close = ui.input(|i| i.viewport().close_requested());
                    if let Some(geometry) = crate::catalog::WindowGeometry::read(ui) {
                        remember_geometry(&mut self.preferences, "shorts".into(), geometry);
                    }
                },
            );
            if close {
                if let Some(p) = shorts.preview_mut() {
                    let _ = p.activate("transport.pause");
                }
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
        if let Some(geometry) = crate::catalog::WindowGeometry::read(ui) {
            remember_geometry(&mut self.preferences, "home".into(), geometry);
        }
        if self.ready_to_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

fn remember_geometry(
    preferences: &mut Preferences,
    key: String,
    mut geometry: crate::catalog::WindowGeometry,
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
    pending: Option<Pending>,
    picker: Option<Picker>,
    candidates: Vec<Candidate>,
    diagnostics: Vec<EditError>,
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
}
impl EditorWindow {
    fn new(
        root: PathBuf,
        preferences: Preferences,
        budget: ByteBudget,
        persistence: ByteBudget,
        runner: std::rc::Rc<std::cell::RefCell<JobRunner>>,
    ) -> Self {
        Self {
            editor: None,
            title: catalog::text(preferences.locale, Text::AppName).to_owned(),
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
        editor.session.execute(
            "project.set_view_state",
            serde_json::json!({"zoom":view.zoom,"scroll":view.scroll,"playhead":at}),
        )?;
        self.retry_save = Some((path.clone(), leave, close, journal_only));
        self.retained_models = editor.take_model_manager();
        let editor = self.editor.take().ok_or(Error::Invalid("no project"))?;
        self.submit(
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
        )
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
                    .set_title(catalog::text(editor.preferences.locale, Text::ExportTitle))
                    .add_filter(
                        catalog::text(editor.preferences.locale, Text::ExportFormat),
                        &[format.extension()],
                    )
                    .set_file_name(file_name);
                if let Some(directory) = directory {
                    dialog = dialog.set_directory(directory);
                }
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
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
                    .set_title(catalog::text(editor.preferences.locale, Text::SnapshotSave))
                    .add_filter(
                        catalog::text(editor.preferences.locale, Text::SnapshotFormat),
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
            std::task::Poll::Ready(None) => {}
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
        let submitted = self.runner.borrow_mut().submit(
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
                    .map_err(|_| EditError::JobPoisoned)?
                    .take()
                    .ok_or(EditError::RecoveryBusy)?;
                let result = run(
                    work,
                    &root,
                    budget.clone(),
                    persistence.clone(),
                    preferences.clone(),
                );
                send.send(result).map_err(|_| EditError::Cancelled)?;
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
        let result = pending.receive.try_recv().unwrap_or_else(|_| {
            Err(snapshot
                .error
                .map(Error::Worker)
                .unwrap_or(Error::Engine(EditError::WorkerPanicked)))
        });
        if pending.cancel.cancelled() {
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
        catalog::text(self.preferences.locale, text)
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
        } else if let Some(pending) = &self.pending {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading(egui::RichText::new(self.tr(Text::AppName)).size(28.0));
                ui.add_space(28.0);
                ui.spinner();
                ui.label(self.tr(pending.text));
                if matches!(pending.text, Text::LoadingProject)
                    && self.button(ui, "project.operation.cancel", Icon::X, Text::Cancel)
                {
                    pending.cancel.cancel();
                }
            });
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
                        Some(Error::Engine(EditError::Project(project::Error::TooNew {
                            ..
                        })))
                        | Some(Error::Project(project::Error::TooNew { .. })) => Text::TooNew,
                        Some(Error::Engine(EditError::Project(project::Error::TooOld {
                            ..
                        })))
                        | Some(Error::Project(project::Error::TooOld { .. })) => Text::TooOld,
                        Some(Error::Engine(EditError::Project(project::Error::Busy)))
                        | Some(Error::Project(project::Error::Busy)) => Text::Busy,
                        Some(Error::Engine(EditError::Project(project::Error::Conflict)))
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
pub(crate) fn fresh(name: &str) -> Result<Session> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid("system time"))?;
    let id = u64::try_from(epoch.as_nanos())
        .map_err(|_| Error::Invalid("project identity"))?
        .wrapping_add(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    Ok(Session::create(
        Command::CreateProject {
            id: project::ProjectId(id),
            name: name.into(),
        },
        100,
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
            fresh(catalog::text(preferences.locale, Text::NewProject))?,
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
            Err(EditError::Project(project::Error::Io(e)))
                if e.kind() == std::io::ErrorKind::NotFound
                    && path.parent().is_some_and(|parent| parent.is_dir()) =>
            {
                session(
                    root,
                    fresh(catalog::text(preferences.locale, Text::UntitledProject))?,
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
                        autosave.execute("project.save", serde_json::json!({}))?;
                    }
                    autosave.flush()?;
                    if autosave.saving() || autosave.is_dirty() {
                        return Err(EditError::StaleResult.into());
                    }
                }
                if leave || close {
                    autosave.execute("project.thumbnail", serde_json::json!({}))?;
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
        let geometry = crate::catalog::WindowGeometry {
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
        let maximized = crate::catalog::WindowGeometry {
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
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let editor = worker_slot.lock().unwrap().take();
            let _active = WorkingEditor {
                slot: worker_slot,
                editor,
            };
            panic!("injected while the worker owns the editor");
        }));
        assert!(outcome.is_err());
        assert_eq!(
            slot.lock()
                .unwrap()
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
}
