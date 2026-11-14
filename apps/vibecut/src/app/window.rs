//! One native editor viewport's lifecycle state: its editor, writer, pending save and close flow.
use super::{
    picker::Picker,
    work::{Pending, Work},
};
use crate::{
    Editor, Error, Result, egui,
    i18n::{self, Text},
    model_manager::ModelManager,
    preferences::Preferences,
};
use engine::{
    ByteBudget, Command,
    jobs::JobRunner,
    recovery::{self, Candidate},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// The editor is absent while its owner is performing an IO boundary on an engine
/// worker. Loading/exiting remains visible; no second Session or serializer exists.
pub struct EditorWindow {
    pub(super) viewport: egui::ViewportId,
    pub editor: Option<Box<Editor>>,
    pub(super) title: String,
    pub(super) retained_models: Option<ModelManager>,
    pub(super) focus_home: bool,
    pub(super) project_window: bool,
    pub(super) path: Option<PathBuf>,
    pub(super) requested_open: Option<PathBuf>,
    pub preferences: Preferences,
    pub(super) root: PathBuf,
    pub(super) budget: ByteBudget,
    pub(super) persistence: ByteBudget,
    /// The lent footage lookup the stores draw cover images with.
    pub(super) corrections: engine::footage::Lookup,
    pub(super) runner: std::rc::Rc<std::cell::RefCell<JobRunner>>,
    pub(super) pending: Option<Pending>,
    pub(super) picker: Option<Picker>,
    pub(super) candidates: Vec<Candidate>,
    pub(super) diagnostics: Vec<recovery::Error>,
    pub(super) error: Option<Error>,
    pub(super) retry_path: Option<PathBuf>,
    pub(super) startup_path: Option<PathBuf>,
    pub(super) close_prompt: bool,
    pub(super) ready_to_close: bool,
    pub(super) configured: bool,
    pub(super) device_available: bool,
    pub(super) retry_save: Option<(Option<PathBuf>, bool, bool, bool)>,
    pub(super) retry_discovery: bool,
    pub(super) preference_warning: bool,
    pub(super) open_file_requested: bool,
    /// Identity the pending operation was issued under, when it is a Save As.
    pub(super) save_as_from: Option<project::ProjectId>,
    /// Identity pair a successful Save As produced, waiting for the workspace
    /// owner to carry lane memory from the issuing project to the copy.
    pub(super) saved_as: Option<(project::ProjectId, project::ProjectId)>,
    /// The editor confirmed the header Delete; the workspace owns the rest.
    pub(super) delete_requested: bool,
    /// A save-and-leave issued for Delete is in flight. The project moves to Trash
    /// only if that leave completes; any failure or cancel clears the intent.
    pub(super) delete_on_leave: bool,
}
impl EditorWindow {
    pub(super) fn new(
        root: PathBuf,
        preferences: Preferences,
        budget: ByteBudget,
        persistence: ByteBudget,
        corrections: engine::footage::Lookup,
        runner: std::rc::Rc<std::cell::RefCell<JobRunner>>,
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
            corrections,
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
            save_as_from: None,
            saved_as: None,
            delete_requested: false,
            delete_on_leave: false,
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
        // A saved project has nothing to ask about: close it the way "keep backup" does.
        let clean = !self.busy()
            && self
                .editor
                .as_mut()
                .and_then(|editor| editor.session.autosave())
                .is_some_and(|autosave| !autosave.is_dirty() && !autosave.saving());
        if clean {
            if let Err(error) = self.persist(None, true, true, true) {
                self.error = Some(error);
            }
        } else if self.editor.is_some() || self.busy() {
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
}
