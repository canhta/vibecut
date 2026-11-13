//! The window registry: Home's lifecycle worker, the project windows by identity, focus and the per-frame tick.
use super::{geometry::remember_geometry, lending::Lending, window::EditorWindow, work::Work};
use crate::{
    Editor, Error, Result, egui,
    home::{Action as HomeAction, State as HomeState},
    i18n::Text,
    preferences::Preferences,
    services::lifecycle::EditorSession,
};
use engine::{
    ByteBudget,
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
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
    pub(super) home: EditorWindow,
    pub(super) focused: Option<project::ProjectId>,
    pub(super) focus_requested: Option<project::ProjectId>,
    pub(super) home_open: bool,
    pub(super) shorts_window: bool,
    /// The one place stores and the model service are lent from.
    pub(super) lending: Lending,
    /// Project files seen this session (catalog rows and open windows), so
    /// lane memory is dropped only on proof the file is gone.
    pub(super) lane_paths: BTreeMap<project::ProjectId, PathBuf>,
    pub(super) lane_prune_key: Vec<project::ProjectId>,
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
        let lending = Lending::open(&root, &budget);
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
            text_clipboard: Default::default(),
            home,
            focused: None,
            focus_requested: None,
            home_open: true,
            shorts_window: false,
            lending,
            lane_paths: BTreeMap::new(),
            lane_prune_key: Vec::new(),
        })
    }
    pub fn control_budget(&self) -> ByteBudget {
        self.home.budget.clone()
    }
    /// The one process media cache instance the timeline and MCP inbox share.
    pub fn media(&self) -> Option<Arc<Mutex<engine::control::MediaAccess>>> {
        self.lending.media()
    }
    /// The user's brand kits, opened once for the process. No editor or control
    /// request reads them yet; the lender exists so the first consumer borrows it.
    pub fn brand_kit(&self) -> Option<Arc<libraries::brand_kit::BrandKit>> {
        self.lending.brand()
    }
    /// The copied appearance value every window reads this frame.
    fn look(&self) -> crate::kit::Look {
        self.preferences.look()
    }
    /// Register an already-composed editor, retaining its writer lease and
    /// resource policy. Repeated project identities focus their existing window.
    pub fn register_editor(&mut self, mut editor: Box<Editor>) -> project::ProjectId {
        self.rekey_windows();
        self.lending.lend_stores(&mut editor);
        let id = editor.session.project().data().id;
        let path = match &editor.session {
            EditorSession::Persistent(s) => s.saved_path().map(ToOwned::to_owned),
            EditorSession::Memory(_) => None,
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
            if let Some(editor) = &mut window.editor {
                self.lending.lend_models(editor);
            }
            window.tick();
            if let Some((from, to)) = window.saved_as.take() {
                self.preferences.copy_timeline_lanes(from, to);
            }
            self.lending.reclaim_models(window);
        }
        self.run_deletions();
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
        if let Some(error) = self.lending.tick_models() {
            self.home.error.get_or_insert(error);
        }
    }
    /// Header Delete: save and leave the project, and only once its writer lease is
    /// gone move it to Trash through Home's library worker. A project the catalog
    /// does not list (opened from an arbitrary file) is refused; a failed or
    /// cancelled leave clears the intent so a later exit cannot delete it.
    fn run_deletions(&mut self) {
        let mut trash = Vec::new();
        for (id, window) in &mut self.windows {
            if std::mem::take(&mut window.delete_requested) {
                let outcome = if self.projects.entries().iter().any(|entry| entry.id == *id) {
                    window.persist(None, true, false, false)
                } else {
                    Err(Error::Invalid("project is not in the library"))
                };
                match outcome {
                    Ok(()) => window.delete_on_leave = true,
                    Err(error) => window.error = Some(error),
                }
            } else if window.delete_on_leave && !window.busy() {
                window.delete_on_leave = false;
                if window.editor.is_none() && window.error.is_none() {
                    trash.push(*id);
                }
            }
        }
        if !trash.is_empty() {
            self.projects.request_delete(trash);
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
    pub(super) fn project_action(&mut self, action: HomeAction) {
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{
        app::picker::{Picker, PickerAction},
        services::{
            lifecycle::fresh,
            playback::{Files, Playback},
        },
    };
    use engine::{
        jobs::JobState,
        recovery::{Autosave, Schedule},
    };
    use std::time::Instant;
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
    fn header_delete_refuses_a_project_the_library_does_not_list_and_keeps_the_editor() {
        let root = tempfile::tempdir().unwrap();
        let mut workspace =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while workspace.busy() {
            workspace.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let autosave = Autosave::new(
            &workspace.home.root,
            fresh("not listed").unwrap(),
            workspace.home.budget.clone(),
            Arc::new(ManualClock::default()),
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
        workspace.tick();
        let window = workspace.focused_window_mut().unwrap();
        window.delete_requested = true;
        workspace.tick();
        let window = workspace.focused_window_mut().unwrap();
        assert!(window.editor.is_some());
        assert!(!window.delete_on_leave && !window.delete_requested);
        assert!(matches!(window.error(), Some(Error::Invalid(_))));
        assert!(!workspace.busy());
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
