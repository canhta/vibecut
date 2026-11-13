//! Holds the Home window's project catalog state and applies worker events to it.
//!
//! Every action is a registered engine command sent through the catalog worker;
//! the state never loads or edits project JSON and holds no second project
//! index. Presentation lives in `view`.

use super::workflow::{Event, Request, catalog_worker, next_id};
use crate::{
    egui,
    home::{ProjectsPreferences, ProjectsSortKey},
    i18n::Text,
};
use engine::{ByteBudget, library::ProjectEntry};
use project::ProjectId;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::JoinHandle,
};

/// What Home asks the workspace lifecycle adapter to do after a command.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Open(PathBuf),
    OpenFile,
    Shorts,
}

pub(super) struct RenameDialog {
    pub(super) id: ProjectId,
    pub(super) name: String,
    pub(super) error: Option<String>,
}

pub(super) struct DeleteDialog {
    pub(super) ids: Vec<ProjectId>,
    pub(super) error: Option<String>,
    /// The dialog either moves to Trash or removes the entries for good.
    pub(super) permanent: bool,
}

/// One owner per library root. A catalog worker owns the exclusive
/// [`ProjectLibrary`] handle for the page's lifetime.
pub struct State {
    pub(super) root: PathBuf,
    pub(super) budget: ByteBudget,
    pub(super) request: Option<Sender<Request>>,
    pub(super) events: Receiver<Event>,
    pub(super) worker: Option<JoinHandle<()>>,
    pub(super) entries: Vec<ProjectEntry>,
    pub(super) trash: Vec<ProjectEntry>,
    /// Trash mode replaces the Projects grid; `trash_loaded` keeps a first
    /// listing from rendering the active entries.
    pub(super) trash_mode: bool,
    pub(super) trash_loaded: bool,
    pub(super) textures: BTreeMap<ProjectId, egui::TextureHandle>,
    pub(super) selection: BTreeSet<ProjectId>,
    pub(super) anchor: Option<ProjectId>,
    pub(super) search: String,
    pub(super) ready: bool,
    pub(super) pending: usize,
    pub(super) creating: bool,

    pub(super) default_fps: u32,
    pub(super) duplicate: Option<(usize, usize)>,
    pub(super) rename: Option<RenameDialog>,
    pub(super) info: Option<ProjectId>,
    pub(super) delete: Option<DeleteDialog>,
    pub(super) notice: Option<(Text, Option<String>)>,
    pub(super) list_error: Option<String>,
    pub(super) action: Option<Action>,
}

impl State {
    pub fn new(root: PathBuf, budget: ByteBudget) -> Self {
        let (_, events) = mpsc::channel();
        let mut page = Self {
            root,
            budget,
            request: None,
            events,
            worker: None,
            entries: Vec::new(),
            trash: Vec::new(),
            trash_mode: false,
            trash_loaded: false,
            textures: BTreeMap::new(),
            selection: BTreeSet::new(),
            anchor: None,
            search: String::new(),
            ready: false,
            pending: 0,
            creating: false,

            default_fps: crate::services::lifecycle::DEFAULT_PROJECT_FPS,
            duplicate: None,
            rename: None,
            info: None,
            delete: None,
            notice: None,
            list_error: None,
            action: None,
        };
        page.spawn();
        page
    }

    fn spawn(&mut self) {
        let (request, requests) = mpsc::channel();
        let (sender, events) = mpsc::channel();
        let root = self.root.clone();
        let budget = self.budget.clone();
        self.request = Some(request);
        self.events = events;
        self.ready = false;
        self.worker = Some(std::thread::spawn(move || {
            let failure = sender.clone();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                catalog_worker(root, budget, requests, sender);
            }))
            .is_err()
            {
                let _ = failure.send(Event::Failed(engine::control::Error::Runtime(
                    engine::RuntimeError::WorkerPanicked,
                )));
            }
        }));
    }

    /// Restart after a catalog-open failure; existing rows remain until replaced.
    pub(super) fn retry(&mut self) {
        self.request = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.pending = 0;
        self.list_error = None;
        self.spawn();
        if self.trash_mode {
            self.request_trash_list();
        }
    }

    pub fn busy(&self) -> bool {
        !self.ready || self.pending > 0
    }

    #[cfg(test)]
    pub(crate) fn defer_initial_catalog(&mut self) -> impl Fn() + use<> {
        let (send, events) = mpsc::channel();
        self.events = events;
        self.ready = false;
        move || assert!(send.send(Event::Ready(Ok(Vec::new()))).is_ok())
    }

    pub fn entries(&self) -> &[ProjectEntry] {
        &self.entries
    }

    /// Trashed entries in their newest-first move order after the live search.
    pub fn trash(&self) -> impl Iterator<Item = &ProjectEntry> {
        self.trash_order()
            .into_iter()
            .filter_map(|index| self.trash.get(index))
    }

    pub fn trash_mode(&self) -> bool {
        self.trash_mode
    }

    /// Project identities once the catalog listing is authoritative. `None`
    /// while a refresh is pending or the last listing failed, so callers never
    /// treat an unknown catalog as an empty one.
    pub fn settled_ids(&self) -> Option<impl Iterator<Item = ProjectId> + '_> {
        (!self.busy() && self.list_error.is_none())
            .then(|| self.entries.iter().map(|entry| entry.id))
    }

    pub fn selection(&self) -> &BTreeSet<ProjectId> {
        &self.selection
    }

    pub fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    /// Rows in the persistent sort order after the live search filter.
    pub fn displayed_ids(&self, projects: ProjectsPreferences) -> Vec<ProjectId> {
        self.order(projects)
            .into_iter()
            .filter_map(|index| self.entries.get(index).map(|entry| entry.id))
            .collect()
    }

    pub(super) fn order(&self, projects: ProjectsPreferences) -> Vec<usize> {
        let mut order = self.matching(&self.entries);
        let sort = projects;
        order.sort_by(|left, right| {
            let Some(a) = self.entries.get(*left) else {
                return std::cmp::Ordering::Equal;
            };
            let Some(b) = self.entries.get(*right) else {
                return std::cmp::Ordering::Equal;
            };
            let ordering = match sort.sort {
                ProjectsSortKey::Created => a.created_unix_ms.cmp(&b.created_unix_ms),
                ProjectsSortKey::Modified => a.modified_unix_ms.cmp(&b.modified_unix_ms),
                ProjectsSortKey::Name => a
                    .name
                    .as_str()
                    .to_lowercase()
                    .cmp(&b.name.as_str().to_lowercase()),
                ProjectsSortKey::Duration => a.duration.cmp(&b.duration),
            };
            let ordering = if sort.ascending {
                ordering
            } else {
                ordering.reverse()
            };
            let tie = a.id.cmp(&b.id);
            ordering.then_with(|| if sort.ascending { tie } else { tie.reverse() })
        });
        order
    }

    /// Trash rows follow their own move order, newest first, so the sort menu
    /// never reorders a retention view by a project property.
    pub(super) fn trash_order(&self) -> Vec<usize> {
        let mut order = self.matching(&self.trash);
        order.sort_by(|left, right| {
            let Some(a) = self.trash.get(*left) else {
                return std::cmp::Ordering::Equal;
            };
            let Some(b) = self.trash.get(*right) else {
                return std::cmp::Ordering::Equal;
            };
            b.trashed_unix_ms
                .cmp(&a.trashed_unix_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        order
    }

    fn matching(&self, entries: &[ProjectEntry]) -> Vec<usize> {
        let needle = engine::NfcText::from(self.search.trim())
            .as_str()
            .to_lowercase();
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                needle.is_empty() || entry.name.as_str().to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub(super) fn set_entries(&mut self, entries: Vec<ProjectEntry>) {
        let ids: BTreeSet<ProjectId> = entries.iter().map(|entry| entry.id).collect();
        let before = std::mem::take(&mut self.entries);
        self.retain_textures(&before, &entries);
        self.entries = entries;
        self.selection.retain(|id| ids.contains(id));
        if self.anchor.is_some_and(|id| !ids.contains(&id)) {
            self.anchor = None;
        }
        if self.info.is_some_and(|id| !ids.contains(&id)) {
            self.info = None;
        }
        if self
            .rename
            .as_ref()
            .is_some_and(|dialog| !ids.contains(&dialog.id))
        {
            self.rename = None;
        }
    }

    pub(super) fn set_trash(&mut self, entries: Vec<ProjectEntry>) {
        let ids: BTreeSet<ProjectId> = entries.iter().map(|entry| entry.id).collect();
        let before = std::mem::take(&mut self.trash);
        self.retain_textures(&before, &entries);
        self.trash = entries;
        self.trash_loaded = true;
        self.selection.retain(|id| ids.contains(id));
        if self.anchor.is_some_and(|id| !ids.contains(&id)) {
            self.anchor = None;
        }
    }

    /// Keep one GPU texture per displayed entry whose thumbnail bytes did not change.
    fn retain_textures(&mut self, before: &[ProjectEntry], after: &[ProjectEntry]) {
        self.textures.retain(|id, _| {
            let before = before.iter().find(|entry| entry.id == *id);
            let after = after.iter().find(|entry| entry.id == *id);
            matches!((before, after), (Some(before), Some(after)) if before.thumbnail == after.thumbnail)
        });
    }

    /// Trash keeps the same page; the toggle swaps the listed catalog.
    pub(super) fn toggle_trash(&mut self) {
        self.trash_mode = !self.trash_mode;
        self.selection.clear();
        self.anchor = None;
        self.info = None;
        self.rename = None;
        self.delete = None;
        self.notice = None;
        if self.trash_mode {
            self.request_trash_list();
        } else if self.ready {
            self.request_list();
        }
    }

    fn send(&mut self, request: Request) -> bool {
        let Some(sender) = &self.request else {
            self.notice = Some((Text::ProjectsListError, None));
            return false;
        };
        if sender.send(request).is_err() {
            self.notice = Some((Text::ProjectsListError, None));
            return false;
        }
        true
    }

    fn request_list(&mut self) {
        if self.send(Request::List) {
            self.pending += 1;
        }
    }

    fn request_trash_list(&mut self) {
        if self.send(Request::ListTrash) {
            self.pending += 1;
        }
    }

    /// Re-read the catalog after a workspace edit changed a project's metadata.
    pub(crate) fn refresh(&mut self) {
        if self.ready {
            self.request_list();
        }
    }

    pub(super) fn request_create(&mut self, base_name: String) {
        if self.send(Request::Create {
            id: next_id(),
            base_name,
            fps: self.default_fps,
        }) {
            self.pending += 1;
            self.creating = true;
        }
    }

    pub(super) fn request_rename(&mut self, id: ProjectId, name: String) {
        if self.send(Request::Rename { id, name }) {
            self.pending += 1;
        }
    }

    pub(super) fn request_duplicate(&mut self, ids: Vec<ProjectId>) {
        if ids.is_empty() {
            return;
        }
        if self.send(Request::Duplicate { sources: ids }) {
            self.notice = None;
            self.pending += 1;
            let total = self.selection.len();
            self.duplicate = Some((0, total.max(1)));
        }
    }

    pub fn request_delete(&mut self, ids: Vec<ProjectId>) {
        if ids.is_empty() {
            return;
        }
        if self.send(Request::Delete { ids }) {
            self.notice = None;
            self.pending += 1;
        }
    }

    pub(super) fn request_restore(&mut self, ids: Vec<ProjectId>) {
        if ids.is_empty() {
            return;
        }
        if self.send(Request::Restore { ids }) {
            self.notice = None;
            self.pending += 1;
        }
    }

    pub(super) fn request_purge(&mut self, ids: Vec<ProjectId>) {
        if ids.is_empty() {
            return;
        }
        if self.send(Request::Purge { ids }) {
            self.notice = None;
            self.pending += 1;
        }
    }

    pub(super) fn request_thumbnail(&mut self, id: ProjectId) {
        if self.send(Request::Thumbnail { id }) {
            self.notice = None;
            self.pending += 1;
        }
    }

    pub fn tick(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(event) => self.apply(event),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.ready = true;
                    self.pending = 0;
                    self.duplicate = None;
                    self.creating = false;
                    self.list_error.get_or_insert_with(String::new);
                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                    self.request = None;
                    return;
                }
            }
        }
    }

    /// A successful listing clears its own failure row, leaving unrelated
    /// action notices (rename, duplicate, restore) intact.
    fn clear_listing_notice(&mut self, text: Text) {
        if self
            .notice
            .as_ref()
            .is_some_and(|(shown, _)| *shown == text)
        {
            self.notice = None;
        }
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Failed(error) => self.list_error = Some(error.to_string()),
            Event::Ready(result) => {
                self.ready = true;
                match result {
                    Ok(entries) => {
                        self.list_error = None;
                        self.clear_listing_notice(Text::ProjectsListError);
                        self.set_entries(entries);
                    }
                    Err(error) => self.list_error = Some(error),
                }
            }
            Event::Listed(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(entries) => {
                        self.list_error = None;
                        self.clear_listing_notice(Text::ProjectsListError);
                        self.set_entries(entries);
                    }
                    Err(error) => {
                        self.list_error = Some(error);
                        self.notice = Some((Text::ProjectsListError, None));
                    }
                }
            }
            Event::Created(result) => {
                self.pending = self.pending.saturating_sub(1);
                self.creating = false;
                match result {
                    Ok((path, entries)) => {
                        self.notice = None;
                        self.set_entries(entries);
                        self.action = Some(Action::Open(path));
                    }
                    Err(error) => self.notice = Some((Text::ProjectError, Some(error))),
                }
            }
            Event::Renamed(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(entries) => {
                        self.set_entries(entries);
                        self.rename = None;
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut self.rename {
                            dialog.error = Some(error);
                        }
                    }
                }
            }
            Event::Duplicated { result } => {
                if let Some((done, _)) = &mut self.duplicate {
                    *done += 1;
                }
                if let Err(error) = result {
                    self.notice = Some((Text::ProjectsDuplicateFailed, Some(error)));
                }
            }
            Event::DuplicateFinished { failures, entries } => {
                self.pending = self.pending.saturating_sub(1);
                self.duplicate = None;
                match entries {
                    Ok(entries) => self.set_entries(entries),
                    Err(error) => self.list_error = Some(error),
                }
                if !failures.is_empty() {
                    self.notice = Some((Text::ProjectsDuplicateFailed, Some(failures.join("; "))));
                }
            }
            Event::Deleted { failures, entries } => {
                self.pending = self.pending.saturating_sub(1);
                match entries {
                    Ok(entries) => self.set_entries(entries),
                    Err(error) => self.list_error = Some(error),
                }
                self.selection.clear();
                self.anchor = None;
                self.delete = None;
                if !failures.is_empty() {
                    self.notice = Some((Text::ProjectsDeleteFailed, Some(failures.join("; "))));
                }
            }
            Event::TrashListed(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(entries) => {
                        self.list_error = None;
                        self.clear_listing_notice(Text::ProjectsTrashError);
                        self.set_trash(entries);
                    }
                    Err(error) => {
                        self.list_error = Some(error);
                        self.notice = Some((Text::ProjectsTrashError, None));
                    }
                }
            }
            Event::Restored {
                failures,
                trash,
                entries,
            } => {
                self.pending = self.pending.saturating_sub(1);
                let trash_failed = trash.is_err();
                match trash {
                    Ok(trash) => {
                        self.clear_listing_notice(Text::ProjectsTrashError);
                        self.set_trash(trash);
                    }
                    Err(error) => self.list_error = Some(error),
                }
                match entries {
                    Ok(entries) => {
                        self.clear_listing_notice(Text::ProjectsListError);
                        self.set_entries(entries);
                    }
                    Err(error) => self.list_error = Some(error),
                }
                self.selection.clear();
                self.anchor = None;
                if !failures.is_empty() {
                    self.notice = Some((Text::ProjectsRestoreFailed, Some(failures.join("; "))));
                } else if trash_failed {
                    self.notice = Some((Text::ProjectsTrashError, None));
                }
            }
            Event::Purged { failures, trash } => {
                self.pending = self.pending.saturating_sub(1);
                match trash {
                    Ok(trash) => {
                        self.clear_listing_notice(Text::ProjectsTrashError);
                        self.set_trash(trash);
                    }
                    Err(error) => self.list_error = Some(error),
                }
                self.selection.clear();
                self.anchor = None;
                self.delete = None;
                if !failures.is_empty() {
                    self.notice = Some((Text::ProjectsPurgeFailed, Some(failures.join("; "))));
                }
            }
            Event::Thumbnail(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(entries) => self.set_entries(entries),
                    Err(error) => {
                        self.notice = Some((Text::ProjectsThumbnailFailed, Some(error)));
                    }
                }
            }
        }
    }

    pub(super) fn select(
        &mut self,
        id: ProjectId,
        order: &[ProjectId],
        modifiers: egui::Modifiers,
    ) {
        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(id);
            let start = order.iter().position(|candidate| *candidate == anchor);
            let end = order.iter().position(|candidate| *candidate == id);
            if let (Some(start), Some(end)) = (start, end) {
                let (start, end) = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                self.selection = order
                    .get(start..=end)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .collect();
            } else {
                self.selection.clear();
                self.selection.insert(id);
            }
            if self.anchor.is_none() {
                self.anchor = Some(id);
            }
        } else if modifiers.command || modifiers.ctrl {
            if !self.selection.remove(&id) {
                self.selection.insert(id);
            }
            self.anchor = Some(id);
        } else {
            self.selection.clear();
            self.selection.insert(id);
            self.anchor = Some(id);
        }
        if self.duplicate.is_none() {
            self.notice = None;
        }
    }

    pub(super) fn update_projects(
        &mut self,
        current: &mut ProjectsPreferences,
        projects: ProjectsPreferences,
    ) {
        *current = projects;
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.request = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::kit;
    use engine::{Command, library::ProjectLibrary};
    use project::Thumbnail;

    fn library(root: &std::path::Path) -> ProjectLibrary {
        ProjectLibrary::open(root, ByteBudget::new(64 << 20).expect("budget")).expect("library")
    }

    #[test]
    fn catalog_retry_retains_rows_after_a_failed_refresh() {
        use egui_kittest::{
            Harness,
            kittest::{By, NodeT, Queryable},
        };
        let root = tempfile::tempdir().expect("directory");
        {
            let mut catalog = library(root.path());
            for id in [1, 2] {
                catalog
                    .execute(
                        ProjectId(id),
                        Command::CreateProject {
                            id: ProjectId(id),
                            name: format!("Project {id}").as_str().into(),
                            fps: 30,
                        },
                    )
                    .expect("create");
            }
        }
        let mut page = State::new(root.path().to_owned(), ByteBudget::new(64 << 20).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while page.busy() {
            page.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            page.settled_ids()
                .expect("a completed listing is authoritative")
                .collect::<Vec<_>>(),
            vec![ProjectId(1), ProjectId(2)]
        );
        // Corruption of another row makes the post-delete listing fail. The
        // last successful rows remain, and Retry must stop the live worker.
        let path = root.path().join("0000000000000002/project.vibecut");
        let original = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"invalid project").unwrap();
        page.request_delete(vec![ProjectId(1)]);
        while page.busy() {
            page.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(page.entries.len(), 2);
        assert!(page.list_error.is_some());
        assert!(
            page.settled_ids().is_none(),
            "a failed listing must not be treated as an empty catalog"
        );
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| {
                crate::Editor::fonts(ctx);
                kit::install(ctx);
            })
            .with_size(egui::vec2(1440.0, 900.0))
            .build_ui_state(
                |ui, state: &mut (State, crate::kit::Look, ProjectsPreferences)| {
                    crate::Editor::theme(ui.ctx(), state.1.dark);
                    state.0.tick();
                    crate::home::show(
                        &mut state.0,
                        crate::home::Context {
                            look: state.1,
                            projects: &mut state.2,
                            status: None,

                            default_fps: 30,
                        },
                        ui,
                    );
                },
                (
                    page,
                    crate::kit::Look::default(),
                    ProjectsPreferences::default(),
                ),
            );
        let retry = || By::new().predicate(|node| node.author_id() == Some("projects.retry"));
        for pending in [1, 0] {
            harness.state_mut().0.pending = pending;
            harness.run_steps(2);
            assert_eq!(
                harness.get(retry()).accesskit_node().is_disabled(),
                pending != 0,
                "Retry must not join a worker while an operation is pending"
            );
        }
        std::fs::write(path, original).unwrap();
        harness.get(retry()).click();
        harness.run_steps(2);
        while harness.state().0.busy() {
            harness.run_steps(1);
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(harness.state().0.list_error.is_none());
    }

    #[test]
    fn notice_rows_and_the_loading_state_do_not_shift_project_card_widget_ids() {
        use egui_kittest::{
            Harness,
            kittest::{By, NodeT, Queryable},
        };
        let root = tempfile::tempdir().expect("directory");
        {
            let mut catalog = library(root.path());
            for id in [1, 2] {
                catalog
                    .execute(
                        ProjectId(id),
                        Command::CreateProject {
                            id: ProjectId(id),
                            name: format!("Project {id}").as_str().into(),
                            fps: 30,
                        },
                    )
                    .expect("create");
            }
        }
        let mut page = State::new(root.path().to_owned(), ByteBudget::new(64 << 20).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while page.busy() {
            page.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let entries = page.entries.clone();
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| {
                crate::Editor::fonts(ctx);
                kit::install(ctx);
            })
            .with_size(egui::vec2(1440.0, 900.0))
            .build_ui_state(
                |ui, state: &mut (State, crate::kit::Look, ProjectsPreferences)| {
                    crate::Editor::theme(ui.ctx(), state.1.dark);
                    state.0.tick();
                    crate::home::show(
                        &mut state.0,
                        crate::home::Context {
                            look: state.1,
                            projects: &mut state.2,
                            status: None,

                            default_fps: 30,
                        },
                        ui,
                    );
                },
                (
                    page,
                    crate::kit::Look::default(),
                    ProjectsPreferences::default(),
                ),
            );
        let card = |harness: &Harness<'_, (State, crate::kit::Look, ProjectsPreferences)>| {
            format!(
                "{:?}",
                harness
                    .get(By::new().predicate(|node| {
                        node.author_id() == Some("projects.card.0000000000000001")
                    }))
                    .accesskit_node()
                    .id()
            )
        };
        harness.run_steps(2);
        let before = card(&harness);

        // A notice row joins the section above the grid asynchronously. The
        // cards below must keep their widget ids or a held press is dropped.
        harness.state_mut().0.notice = Some((Text::ProjectError, Some("detail".into())));
        harness.run_steps(2);
        assert_eq!(
            card(&harness),
            before,
            "a notice row above the grid must not move the cards' widget ids"
        );
        harness.state_mut().0.notice = None;
        harness.run_steps(2);

        // The catalog can be held in its loading state. The spinner row that
        // replaces the grid must leave the same ids behind when it is released.
        let release = harness.state_mut().0.defer_initial_catalog();
        harness.run_steps(2);
        let locale = harness.state().1.locale;
        assert!(
            harness
                .query_by_label(crate::i18n::text(locale, Text::LoadingProject))
                .is_some(),
            "the loading row must render while the catalog is pending"
        );
        release();
        harness.run_steps(2);
        harness.state_mut().0.set_entries(entries);
        harness.run_steps(2);
        assert_eq!(
            card(&harness),
            before,
            "loading frames must not leave the grid with new widget ids"
        );
    }

    #[test]
    fn thumbnail_refresh_replaces_only_changed_textures() {
        let root = tempfile::tempdir().expect("directory");
        let mut catalog = library(root.path());
        catalog
            .execute(
                ProjectId(1),
                Command::CreateProject {
                    id: ProjectId(1),
                    name: "Alpha".into(),
                    fps: 30,
                },
            )
            .expect("create");
        let mut entries = catalog.list().expect("list");
        entries[0].thumbnail = Thumbnail::Ready {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        };
        drop(catalog);
        let mut page = State::new(root.path().to_owned(), ByteBudget::new(64 << 20).unwrap());
        page.set_entries(entries);
        let context = egui::Context::default();
        for changed in [false, true] {
            page.textures.insert(
                ProjectId(1),
                context.load_texture(
                    "test thumbnail",
                    egui::ColorImage::new([1, 1], vec![egui::Color32::RED]),
                    egui::TextureOptions::LINEAR,
                ),
            );
            let mut entries = page.entries.clone();
            if changed {
                entries[0].thumbnail = Thumbnail::Ready {
                    width: 1,
                    height: 1,
                    rgba: vec![0, 255, 0, 255],
                };
            }
            page.set_entries(entries);
            assert_eq!(page.textures.contains_key(&ProjectId(1)), !changed);
        }
    }
}
