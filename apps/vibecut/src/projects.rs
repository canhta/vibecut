//! Projects page presentation over the engine's managed project catalog.
//!
//! Every action is a registered engine command sent through
//! [`ProjectLibrary::execute`]; the page never loads or edits project JSON, and
//! it holds no second project index. Listing, duplicate, rename, delete and
//! thumbnail calls run on one catalog worker so no filesystem boundary blocks
//! the UI thread.

use crate::{
    catalog::{
        self, Locale, Preferences, ProjectsPreferences, ProjectsSortKey, ProjectsView, Text,
    },
    egui, kit as theme,
    kit::{self as icons, Icon},
};
use engine::{
    ByteBudget, Command,
    library::{ProjectEntry, ProjectLibrary},
};
use project::{ProjectId, Thumbnail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::JoinHandle,
};
use time::Tick;

const CARD_WIDTH: f32 = 220.0;
const GRID_THUMB: [f32; 2] = [204.0, 115.0];
const LIST_THUMB: [f32; 2] = [96.0, 54.0];
const ICON_BUTTON: f32 = 28.0;

/// What the page asks the workspace lifecycle adapter to do after a command.
#[derive(Debug, PartialEq, Eq)]
pub enum ProjectsAction {
    Open(PathBuf),
    OpenFile,
    Shorts,
}

enum Request {
    List,
    Create { id: ProjectId, base_name: String },
    Rename { id: ProjectId, name: String },
    Duplicate { sources: Vec<ProjectId> },
    Delete { ids: Vec<ProjectId> },
    Thumbnail { id: ProjectId },
}

enum Event {
    Failed(engine::EditError),
    Ready(Result<Vec<ProjectEntry>, String>),
    Listed(Result<Vec<ProjectEntry>, String>),
    Created(Result<(PathBuf, Vec<ProjectEntry>), String>),
    Renamed(Result<Vec<ProjectEntry>, String>),
    Duplicated {
        result: Result<(), String>,
    },
    DuplicateFinished {
        failures: Vec<String>,
        entries: Result<Vec<ProjectEntry>, String>,
    },
    Deleted {
        failures: Vec<String>,
        entries: Result<Vec<ProjectEntry>, String>,
    },
    Thumbnail(Result<Vec<ProjectEntry>, String>),
}

struct RenameDialog {
    id: ProjectId,
    name: String,
    error: Option<String>,
}

struct DeleteDialog {
    ids: Vec<ProjectId>,
    error: Option<String>,
}

/// One owner per library root. A catalog worker owns the exclusive
/// [`ProjectLibrary`] handle for the page's lifetime.
pub struct ProjectsPage {
    root: PathBuf,
    budget: ByteBudget,
    request: Option<Sender<Request>>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
    entries: Vec<ProjectEntry>,
    textures: BTreeMap<ProjectId, egui::TextureHandle>,
    selection: BTreeSet<ProjectId>,
    anchor: Option<ProjectId>,
    search: String,
    ready: bool,
    pending: usize,
    creating: bool,
    duplicate: Option<(usize, usize)>,
    rename: Option<RenameDialog>,
    info: Option<ProjectId>,
    delete: Option<DeleteDialog>,
    notice: Option<(Text, Option<String>)>,
    list_error: Option<String>,
    action: Option<ProjectsAction>,
}

impl ProjectsPage {
    pub fn new(root: PathBuf, budget: ByteBudget) -> Self {
        let (_, events) = mpsc::channel();
        let mut page = Self {
            root,
            budget,
            request: None,
            events,
            worker: None,
            entries: Vec::new(),
            textures: BTreeMap::new(),
            selection: BTreeSet::new(),
            anchor: None,
            search: String::new(),
            ready: false,
            pending: 0,
            creating: false,
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
                let _ = failure.send(Event::Failed(engine::EditError::WorkerPanicked));
            }
        }));
    }

    /// Restart after a catalog-open failure; existing rows remain until replaced.
    fn retry(&mut self) {
        self.request = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.pending = 0;
        self.list_error = None;
        self.spawn();
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

    pub fn selection(&self) -> &BTreeSet<ProjectId> {
        &self.selection
    }

    pub fn take_action(&mut self) -> Option<ProjectsAction> {
        self.action.take()
    }

    /// Rows in the persistent sort order after the live search filter.
    pub fn displayed_ids(&self, preferences: &Preferences) -> Vec<ProjectId> {
        self.order(preferences)
            .into_iter()
            .filter_map(|index| self.entries.get(index).map(|entry| entry.id))
            .collect()
    }

    fn order(&self, preferences: &Preferences) -> Vec<usize> {
        let needle = engine::NfcText::from(self.search.trim())
            .as_str()
            .to_lowercase();
        let mut order: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                needle.is_empty() || entry.name.as_str().to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect();
        let sort = preferences.projects;
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

    fn set_entries(&mut self, entries: Vec<ProjectEntry>) {
        let ids: BTreeSet<ProjectId> = entries.iter().map(|entry| entry.id).collect();
        self.textures.retain(|id, _| {
            let before = self.entries.iter().find(|entry| entry.id == *id);
            let after = entries.iter().find(|entry| entry.id == *id);
            matches!((before, after), (Some(before), Some(after)) if before.thumbnail == after.thumbnail)
        });
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

    /// Re-read the catalog after a workspace edit changed a project's metadata.
    pub(crate) fn refresh(&mut self) {
        if self.ready {
            self.request_list();
        }
    }

    fn request_create(&mut self, base_name: String) {
        if self.send(Request::Create {
            id: next_id(),
            base_name,
        }) {
            self.pending += 1;
            self.creating = true;
        }
    }

    fn request_rename(&mut self, id: ProjectId, name: String) {
        if self.send(Request::Rename { id, name }) {
            self.pending += 1;
        }
    }

    fn request_duplicate(&mut self, ids: Vec<ProjectId>) {
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

    fn request_delete(&mut self, ids: Vec<ProjectId>) {
        if ids.is_empty() {
            return;
        }
        if self.send(Request::Delete { ids }) {
            self.notice = None;
            self.pending += 1;
        }
    }

    fn request_thumbnail(&mut self, id: ProjectId) {
        if self.send(Request::Thumbnail { id }) {
            self.notice = None;
            self.pending += 1;
        }
    }

    pub(crate) fn tick(&mut self) {
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

    fn apply(&mut self, event: Event) {
        match event {
            Event::Failed(error) => self.list_error = Some(error.to_string()),
            Event::Ready(result) => {
                self.ready = true;
                match result {
                    Ok(entries) => {
                        self.list_error = None;
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
                        self.action = Some(ProjectsAction::Open(path));
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

    fn select(&mut self, id: ProjectId, order: &[ProjectId], modifiers: egui::Modifiers) {
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

    fn update_projects(&mut self, preferences: &mut Preferences, projects: ProjectsPreferences) {
        if preferences.projects == projects {
            return;
        }
        if let Err(error) = preferences.set(
            "projects",
            serde_json::to_value(projects).unwrap_or(serde_json::Value::Null),
        ) {
            self.notice = Some((Text::ProjectError, Some(error.to_string())));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, preferences: &mut Preferences) {
        let locale = preferences.locale;
        let dark = preferences.dark;
        let tr = |key: Text| catalog::text(locale, key);

        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new(tr(Text::Projects)).size(28.0));
            ui.add_space(18.0);
            if action_button(
                ui,
                "project.create",
                Icon::Plus,
                tr(Text::NewProject),
                !self.busy(),
            ) {
                self.request_create(tr(Text::NewProject).to_owned());
            }
            if action_button(
                ui,
                "project.open",
                Icon::FolderOpen,
                tr(Text::OpenProject),
                !self.busy(),
            ) {
                self.action = Some(ProjectsAction::OpenFile);
            }
            if action_button(ui, "home.shorts", Icon::Scissors, tr(Text::Shorts), true) {
                self.action = Some(ProjectsAction::Shorts);
            }
        });
        ui.add_space(10.0);

        if self.creating {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr(Text::LoadingProject));
            });
        }

        if let Some((text, detail)) = &self.notice {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr(*text)).color(theme::error_text(dark)));
                if let Some(detail) = detail.as_ref().filter(|detail| !detail.is_empty()) {
                    ui.label(
                        egui::RichText::new(detail)
                            .small()
                            .color(theme::secondary(dark)),
                    );
                }
            });
            ui.add_space(6.0);
        }
        if let Some(error) = self.list_error.clone() {
            let mut retry = false;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr(Text::ProjectsListError)).color(theme::error_text(dark)),
                );
                if !error.is_empty() {
                    ui.label(
                        egui::RichText::new(&error)
                            .small()
                            .color(theme::secondary(dark)),
                    );
                }
                if action_button(
                    ui,
                    "projects.retry",
                    Icon::ArrowClockwise,
                    tr(Text::ProjectsRetry),
                    !self.busy(),
                ) {
                    retry = true;
                }
            });
            if retry {
                self.retry();
            }
            ui.add_space(6.0);
        }

        if !self.ready {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr(Text::LoadingProject));
            });
            self.dialog(ui.ctx(), preferences);
            return;
        }

        let shown = self.order(preferences);
        let order: Vec<ProjectId> = shown
            .iter()
            .filter_map(|index| self.entries.get(*index).map(|entry| entry.id))
            .collect();
        self.toolbar(ui, preferences, &order, locale, dark);

        let mut actions = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.busy() {
                    ui.disable();
                }
                if shown.is_empty() {
                    let text = if self.entries.is_empty() {
                        Text::ProjectsEmpty
                    } else {
                        Text::ProjectsNoResults
                    };
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(tr(text)).color(theme::secondary(dark)));
                    });
                    return;
                }
                match preferences.projects.view {
                    ProjectsView::Grid => {
                        ui.horizontal_wrapped(|ui| {
                            for index in &shown {
                                let Some(entry) = self.entries.get(*index) else {
                                    continue;
                                };
                                card(
                                    ui,
                                    entry,
                                    self.selection.contains(&entry.id),
                                    false,
                                    locale,
                                    dark,
                                    &mut self.textures,
                                    &mut actions,
                                );
                            }
                        });
                    }
                    ProjectsView::List => {
                        for index in &shown {
                            let Some(entry) = self.entries.get(*index) else {
                                continue;
                            };
                            card(
                                ui,
                                entry,
                                self.selection.contains(&entry.id),
                                true,
                                locale,
                                dark,
                                &mut self.textures,
                                &mut actions,
                            );
                        }
                    }
                }
            });

        for action in actions {
            match action {
                CardAction::Select { id, modifiers } => {
                    let order = order.clone();
                    self.select(id, &order, modifiers);
                }
                CardAction::Rename(id) => {
                    let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
                        continue;
                    };
                    self.rename = Some(RenameDialog {
                        id,
                        name: entry.name.as_str().to_owned(),
                        error: None,
                    });
                }
                CardAction::Info(id) => self.info = Some(id),
                CardAction::Open(id) => {
                    if let Some(entry) = self.entries.iter().find(|entry| entry.id == id) {
                        self.action = Some(ProjectsAction::Open(entry.path.clone()));
                    }
                }
                CardAction::Duplicate(id) => {
                    self.selection.clear();
                    self.selection.insert(id);
                    self.anchor = Some(id);
                    self.request_duplicate(vec![id]);
                }
                CardAction::Delete(id) => {
                    self.delete = Some(DeleteDialog {
                        ids: vec![id],
                        error: None,
                    });
                }
                CardAction::Thumbnail(id) => self.request_thumbnail(id),
            }
        }

        self.dialog(ui.ctx(), preferences);
    }

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        preferences: &mut Preferences,
        order: &[ProjectId],
        locale: Locale,
        dark: bool,
    ) {
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(catalog::text(locale, Text::ProjectsSearch))
                    .desired_width(240.0),
            );
            set_author_id(&search, "projects.search");

            let sort = preferences.projects;
            let key = sort_key_text(sort.sort);
            let order_label = if sort.ascending {
                Text::ProjectsSortAscending
            } else {
                Text::ProjectsSortDescending
            };
            let value = catalog::format(
                locale,
                Text::ProjectsSortValue,
                &[
                    ("key", catalog::text(locale, key)),
                    ("order", catalog::text(locale, order_label)),
                ],
            );
            let label = catalog::format(locale, Text::ProjectsSort, &[("sort", &value)]);
            let sort_response = ui
                .menu_image_text_button(icons::icon(Icon::Funnel, 16.0), label, |ui| {
                    for candidate in [
                        ProjectsSortKey::Created,
                        ProjectsSortKey::Modified,
                        ProjectsSortKey::Name,
                        ProjectsSortKey::Duration,
                    ] {
                        let id = match candidate {
                            ProjectsSortKey::Created => "projects.sort.created",
                            ProjectsSortKey::Modified => "projects.sort.modified",
                            ProjectsSortKey::Name => "projects.sort.name",
                            ProjectsSortKey::Duration => "projects.sort.duration",
                        };
                        let label = catalog::text(locale, sort_key_text(candidate));
                        let response = ui
                            .push_id(id, |ui| ui.selectable_label(sort.sort == candidate, label))
                            .inner;
                        set_author_id(&response, id);
                        if response.clicked() {
                            let mut next = sort;
                            if next.sort == candidate {
                                next.ascending = !next.ascending;
                            } else {
                                next.sort = candidate;
                                next.ascending = true;
                            }
                            if let Err(error) = preferences.set(
                                "projects",
                                serde_json::to_value(next).unwrap_or(serde_json::Value::Null),
                            ) {
                                self.notice = Some((Text::ProjectError, Some(error.to_string())));
                            }
                            ui.close();
                        }
                    }
                })
                .response;
            set_author_id(&sort_response, "projects.sort");

            ui.add_space(10.0);
            if toggle_button(
                ui,
                "projects.view.grid",
                Icon::FilmStrip,
                catalog::text(locale, Text::ProjectsGrid),
                preferences.projects.view == ProjectsView::Grid,
            ) {
                let mut next = preferences.projects;
                next.view = ProjectsView::Grid;
                self.update_projects(preferences, next);
            }
            if toggle_button(
                ui,
                "projects.view.list",
                Icon::ArrowsInLineHorizontal,
                catalog::text(locale, Text::ProjectsList),
                preferences.projects.view == ProjectsView::List,
            ) {
                let mut next = preferences.projects;
                next.view = ProjectsView::List;
                self.update_projects(preferences, next);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let selected: Vec<ProjectId> = self
                    .selection
                    .iter()
                    .copied()
                    .filter(|id| order.contains(id))
                    .collect();
                let has_selection = !selected.is_empty();
                if action_button(
                    ui,
                    "projects.toolbar.delete",
                    Icon::Trash,
                    catalog::text(locale, Text::Delete),
                    has_selection && !self.busy(),
                ) {
                    self.delete = Some(DeleteDialog {
                        ids: selected.clone(),
                        error: None,
                    });
                }
                if action_button(
                    ui,
                    "projects.toolbar.duplicate",
                    Icon::FloppyDiskBack,
                    catalog::text(locale, Text::ProjectsDuplicate),
                    has_selection && !self.busy(),
                ) {
                    self.selection = selected.iter().copied().collect();
                    let ids = selected.clone();
                    self.request_duplicate(ids);
                }
                if let Some((done, total)) = self.duplicate {
                    let done = catalog::number(locale, done as u64);
                    let total = catalog::number(locale, total as u64);
                    ui.label(catalog::format(
                        locale,
                        Text::ProjectsDuplicating,
                        &[("done", &done), ("total", &total)],
                    ));
                } else if has_selection {
                    let count = catalog::number(locale, selected.len() as u64);
                    ui.label(
                        egui::RichText::new(catalog::format(
                            locale,
                            Text::ProjectsSelected,
                            &[("n", &count)],
                        ))
                        .color(theme::secondary(dark)),
                    );
                }
            });
        });
        ui.add_space(8.0);
    }

    fn dialog(&mut self, ctx: &egui::Context, preferences: &Preferences) {
        let locale = preferences.locale;
        let dark = preferences.dark;
        let busy = self.busy();
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if escape && !busy {
            self.rename = None;
            self.info = None;
            self.delete = None;
        }

        let mut rename_submit: Option<(ProjectId, String)> = None;
        if let Some(dialog) = &mut self.rename {
            let mut save = false;
            let mut cancel = false;
            let title = catalog::text(locale, Text::ProjectsRenameTitle);
            let response =
                egui::Modal::new(egui::Id::unique("projects.rename.dialog")).show(ctx, |ui| {
                    ui.heading(title);
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(320.0));
                    set_author_id(&response, "projects.rename.input");
                    if let Some(error) = &dialog.error {
                        ui.label(
                            egui::RichText::new(catalog::text(locale, Text::ProjectsRenameInvalid))
                                .color(theme::error_text(dark)),
                        );
                        if !error.is_empty() {
                            ui.label(
                                egui::RichText::new(error)
                                    .small()
                                    .color(theme::secondary(dark)),
                            );
                        }
                    }
                    ui.horizontal(|ui| {
                        if action_button(
                            ui,
                            "projects.rename.save",
                            Icon::FloppyDisk,
                            catalog::text(locale, Text::ProjectsRenameSave),
                            !busy,
                        ) {
                            save = true;
                        }
                        if action_button(
                            ui,
                            "projects.rename.cancel",
                            Icon::X,
                            catalog::text(locale, Text::Cancel),
                            !busy,
                        ) {
                            cancel = true;
                        }
                    });
                });
            set_author_id(&response.response, "projects.rename.dialog");
            if save {
                let name = dialog.name.trim().to_owned();
                if name.is_empty() {
                    dialog.error = Some(String::new());
                } else {
                    dialog.error = None;
                    rename_submit = Some((dialog.id, name));
                }
            }
            if cancel {
                self.rename = None;
            }
        }
        if let Some((id, name)) = rename_submit {
            self.request_rename(id, name);
        }

        if let Some(id) = self.info {
            let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
                self.info = None;
                return;
            };
            let name = entry.name.as_str().to_owned();
            let duration = display_time(entry.duration);
            let created = catalog::date(locale, entry.created_unix_ms);
            let modified = catalog::date(locale, entry.modified_unix_ms);
            let short_id = entry.short_id.clone();
            let title = catalog::text(locale, Text::ProjectsInfoTitle);
            let mut close = false;
            let response =
                egui::Modal::new(egui::Id::unique("projects.info.dialog")).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.heading(title);
                    ui.label(egui::RichText::new(name).strong().size(16.0));
                    ui.label(catalog::format(
                        locale,
                        Text::ProjectsDuration,
                        &[("duration", &duration)],
                    ));
                    ui.label(catalog::format(
                        locale,
                        Text::CreatedDate,
                        &[("date", &created)],
                    ));
                    ui.label(catalog::format(
                        locale,
                        Text::ModifiedDate,
                        &[("date", &modified)],
                    ));
                    ui.label(format!(
                        "{} {}",
                        catalog::text(locale, Text::ProjectsInfoId),
                        short_id
                    ));
                    if action_button(
                        ui,
                        "projects.info.close",
                        Icon::X,
                        catalog::text(locale, Text::ProjectsInfoClose),
                        true,
                    ) {
                        close = true;
                    }
                });
            set_author_id(&response.response, "projects.info.dialog");
            if close {
                self.info = None;
            }
        }

        if let Some(dialog) = &self.delete {
            let names: Vec<String> = dialog
                .ids
                .iter()
                .filter_map(|id| {
                    self.entries
                        .iter()
                        .find(|entry| entry.id == *id)
                        .map(|entry| entry.name.as_str().to_owned())
                })
                .collect();
            let mut confirm = false;
            let mut cancel = false;
            let title = catalog::text(locale, Text::ProjectsDeleteTitle);
            let response =
                egui::Modal::new(egui::Id::unique("projects.delete.dialog")).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.heading(title);
                    ui.label(catalog::text(locale, Text::ProjectsDeletePrompt));
                    let names_label = names.join(", ");
                    let response =
                        ui.add(egui::Label::new(egui::RichText::new(names_label).strong()).wrap());
                    set_author_id(&response, "projects.delete.names");
                    if let Some(error) = &dialog.error {
                        ui.label(egui::RichText::new(error).color(theme::error_text(dark)));
                    }
                    ui.horizontal(|ui| {
                        if action_button(
                            ui,
                            "projects.delete.confirm",
                            Icon::Trash,
                            catalog::text(locale, Text::ProjectsDeleteConfirm),
                            !busy,
                        ) {
                            confirm = true;
                        }
                        if action_button(
                            ui,
                            "projects.delete.cancel",
                            Icon::X,
                            catalog::text(locale, Text::Cancel),
                            !busy,
                        ) {
                            cancel = true;
                        }
                    });
                });
            set_author_id(&response.response, "projects.delete.dialog");
            if confirm {
                let ids = dialog.ids.clone();
                self.request_delete(ids);
            }
            if cancel {
                self.delete = None;
            }
        }
    }
}

impl Drop for ProjectsPage {
    fn drop(&mut self) {
        self.request = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum CardAction {
    Select {
        id: ProjectId,
        modifiers: egui::Modifiers,
    },
    Rename(ProjectId),
    Info(ProjectId),
    Open(ProjectId),
    Duplicate(ProjectId),
    Delete(ProjectId),
    Thumbnail(ProjectId),
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    selected: bool,
    list: bool,
    locale: Locale,
    dark: bool,
    textures: &mut BTreeMap<ProjectId, egui::TextureHandle>,
    actions: &mut Vec<CardAction>,
) {
    let frame = egui::Frame::new()
        .fill(if selected {
            theme::hover(dark)
        } else {
            theme::panel(dark)
        })
        .stroke(egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                theme::accent(dark)
            } else {
                theme::separator(dark)
            },
        ))
        .corner_radius(8)
        .inner_margin(8);
    let width = if list {
        ui.available_width()
    } else {
        CARD_WIDTH
    };
    let inner = frame.show(ui, |ui| {
        ui.set_width(width - 16.0);
        let thumb_size = if list { LIST_THUMB } else { GRID_THUMB };
        if list {
            ui.horizontal(|ui| {
                thumbnail(ui, entry, thumb_size, textures, actions, locale);
                ui.vertical(|ui| {
                    name_and_metadata(ui, entry, locale, dark, actions);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    card_menu(ui, entry, locale, actions);
                });
            });
        } else {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space(0.0);
                    thumbnail(ui, entry, thumb_size, textures, actions, locale);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        card_menu(ui, entry, locale, actions);
                    });
                });
                name_and_metadata(ui, entry, locale, dark, actions);
            });
        }
    });
    if inner.response.hovered() && !selected {
        ui.painter().rect_stroke(
            inner.response.rect,
            8,
            egui::Stroke::new(1.0, theme::border(dark)),
            egui::StrokeKind::Inside,
        );
    }
}

fn thumbnail(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    size: [f32; 2],
    textures: &mut BTreeMap<ProjectId, egui::TextureHandle>,
    actions: &mut Vec<CardAction>,
    locale: Locale,
) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size[0], size[1]), egui::Sense::click());
    set_author_id(
        &response,
        &format!("projects.card.{:016x}.thumbnail", entry.id.0),
    );
    if response.clicked() {
        actions.push(CardAction::Select {
            id: entry.id,
            modifiers: ui.input(|input| input.modifiers),
        });
    }
    if response.double_clicked() {
        actions.push(CardAction::Open(entry.id));
    }
    response.context_menu(|ui| {
        card_menu_items(ui, &entry.id, actions, locale);
    });
    ui.painter().rect_filled(rect, 4.0, theme::field(dark(ui)));
    match &entry.thumbnail {
        Thumbnail::Ready {
            width,
            height,
            rgba,
        } => {
            let texture = textures.entry(entry.id).or_insert_with(|| {
                ui.ctx().load_texture(
                    format!("projects.thumbnail.{}", entry.id.0),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [*width as usize, *height as usize],
                        rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
            let image = egui::vec2(*width as f32, *height as f32);
            let scale = (rect.width() / image.x)
                .min(rect.height() / image.y)
                .max(0.0);
            let fitted = image * scale;
            ui.painter().image(
                texture.id(),
                egui::Rect::from_center_size(rect.center(), fitted),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                theme::image_tint(),
            );
        }
        Thumbnail::Missing | Thumbnail::Unavailable => {
            let label = egui::Label::new(
                egui::RichText::new(catalog::text(locale, Text::ProjectsThumbnailMissing))
                    .small()
                    .color(theme::secondary(dark(ui))),
            );
            let label = ui.put(rect.shrink(4.0), label);
            set_author_id(
                &label,
                &format!("projects.card.{:016x}.thumbnail.placeholder", entry.id.0),
            );
        }
    }
}

fn name_and_metadata(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    locale: Locale,
    dark: bool,
    actions: &mut Vec<CardAction>,
) {
    let duration = display_time(entry.duration);
    let created = catalog::date(locale, entry.created_unix_ms);
    let text = format!(
        "{}\n{}\n{}",
        entry.name.as_str(),
        duration,
        catalog::format(locale, Text::CreatedDate, &[("date", &created)])
    );
    let response = ui.add(
        egui::Label::new(egui::RichText::new(text).color(theme::text(dark)))
            .wrap()
            .sense(egui::Sense::click()),
    );
    set_author_id(&response, &format!("projects.card.{:016x}", entry.id.0));
    if response.clicked() {
        actions.push(CardAction::Select {
            id: entry.id,
            modifiers: ui.input(|input| input.modifiers),
        });
    }
    if response.double_clicked() {
        actions.push(CardAction::Open(entry.id));
    }
    response.context_menu(|ui| {
        card_menu_items(ui, &entry.id, actions, locale);
    });
}

fn card_menu(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    locale: Locale,
    actions: &mut Vec<CardAction>,
) {
    let response = ui
        .menu_image_button(icons::icon(Icon::SlidersHorizontal, 16.0), |ui| {
            card_menu_items(ui, &entry.id, actions, locale);
        })
        .response;
    let response = response.on_hover_text(catalog::text(locale, Text::ProjectsCardMenu));
    set_author_id(
        &response,
        &format!("projects.card.{:016x}.menu", entry.id.0),
    );
}

fn card_menu_items(
    ui: &mut egui::Ui,
    id: &ProjectId,
    actions: &mut Vec<CardAction>,
    locale: Locale,
) {
    let prefix = format!("projects.card.{:016x}", id.0);
    if menu_action(
        ui,
        &format!("{prefix}.open"),
        Icon::FolderOpen,
        catalog::text(locale, Text::ProjectsOpen),
    ) {
        actions.push(CardAction::Open(*id));
        ui.close();
    }
    if menu_action(
        ui,
        &format!("{prefix}.rename"),
        Icon::PencilSimple,
        catalog::text(locale, Text::ProjectsRename),
    ) {
        actions.push(CardAction::Rename(*id));
        ui.close();
    }
    if menu_action(
        ui,
        &format!("{prefix}.info"),
        Icon::File,
        catalog::text(locale, Text::ProjectsInfoTitle),
    ) {
        actions.push(CardAction::Info(*id));
        ui.close();
    }
    if menu_action(
        ui,
        &format!("{prefix}.duplicate"),
        Icon::FloppyDiskBack,
        catalog::text(locale, Text::ProjectsDuplicate),
    ) {
        actions.push(CardAction::Duplicate(*id));
        ui.close();
    }
    if menu_action(
        ui,
        &format!("{prefix}.delete"),
        Icon::Trash,
        catalog::text(locale, Text::Delete),
    ) {
        actions.push(CardAction::Delete(*id));
        ui.close();
    }
    if menu_action(
        ui,
        &format!("{prefix}.thumbnail.refresh"),
        Icon::ArrowClockwise,
        catalog::text(locale, Text::ProjectsRefreshThumbnail),
    ) {
        actions.push(CardAction::Thumbnail(*id));
        ui.close();
    }
}

fn menu_action(ui: &mut egui::Ui, id: &str, icon: Icon, label: &str) -> bool {
    let response = ui
        .push_id(id, |ui| {
            ui.add(egui::Button::image_and_text(
                icons::tinted(icon, 16.0, ui.visuals().text_color()),
                label,
            ))
        })
        .inner;
    set_author_id(&response, id);
    response.clicked()
}

fn action_button(ui: &mut egui::Ui, id: &str, icon: Icon, label: &str, enabled: bool) -> bool {
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let response = ui
        .push_id(id, |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::image_and_text(
                    icons::tinted(icon, 16.0, color),
                    egui::RichText::new(label).color(color),
                )
                .min_size(egui::vec2(28.0, 26.0)),
            )
        })
        .inner;
    set_author_id(&response, id);
    response.clicked()
}

fn toggle_button(ui: &mut egui::Ui, id: &str, icon: Icon, label: &str, selected: bool) -> bool {
    let (rect, response) = ui
        .push_id(id, |ui| {
            ui.allocate_exact_size(egui::Vec2::splat(ICON_BUTTON), egui::Sense::click())
        })
        .inner;
    let color = if selected {
        theme::accent(ui.visuals().dark_mode)
    } else {
        ui.visuals().text_color()
    };
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, theme::field(ui.visuals().dark_mode));
    }
    icons::tinted(icon, 16.0, color).paint_at(
        ui,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(16.0)),
    );
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, label));
    let response = response.on_hover_text(label);
    set_author_id(&response, id);
    response.clicked()
}

fn set_author_id(response: &egui::Response, id: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
}

fn sort_key_text(key: ProjectsSortKey) -> Text {
    match key {
        ProjectsSortKey::Created => Text::ProjectsSortCreated,
        ProjectsSortKey::Modified => Text::ProjectsSortModified,
        ProjectsSortKey::Name => Text::ProjectsSortName,
        ProjectsSortKey::Duration => Text::ProjectsSortDuration,
    }
}

fn display_time(at: Tick) -> String {
    let seconds = at.seconds().max(0.0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (seconds / 3600.0) as u64,
        (seconds / 60.0) as u64 % 60,
        seconds as u64 % 60,
        ((seconds.fract() * 1000.0).floor()) as u64
    )
}

fn dark(ui: &egui::Ui) -> bool {
    ui.visuals().dark_mode
}

fn next_id() -> ProjectId {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let value = u64::try_from(epoch)
        .unwrap_or(u64::MAX)
        .wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed));
    ProjectId(value)
}

fn unique_name(entries: &[ProjectEntry], base: &str) -> String {
    if !entries.iter().any(|entry| entry.name.as_str() == base) {
        return base.to_owned();
    }
    for number in 2..=u64::from(u32::MAX) {
        let candidate = format!("{base} {number}");
        if !entries.iter().any(|entry| entry.name.as_str() == candidate) {
            return candidate;
        }
    }
    format!("{base} {}", u64::MAX)
}

fn catalog_worker(
    root: PathBuf,
    budget: ByteBudget,
    requests: Receiver<Request>,
    events: Sender<Event>,
) {
    let mut library = match ProjectLibrary::open(&root, budget) {
        Ok(library) => library,
        Err(error) => {
            let _ = events.send(Event::Ready(Err(error.to_string())));
            return;
        }
    };
    let initial = library.list().map_err(|error| error.to_string());
    let _ = events.send(Event::Ready(initial));
    while let Ok(request) = requests.recv() {
        match request {
            Request::List => {
                let listed = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::Listed(listed));
            }
            Request::Create { id, base_name } => {
                let result = (|| {
                    let entries = library.list()?;
                    let name = unique_name(&entries, &base_name);
                    library.execute(
                        id,
                        Command::CreateProject {
                            id,
                            name: name.as_str().into(),
                        },
                    )?;
                    let entries = library.list()?;
                    let path = entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .map(|entry| entry.path.clone())
                        .ok_or(engine::EditError::NoItem(project::ItemId(0)))?;
                    Ok((path, entries))
                })()
                .map_err(|error: engine::EditError| error.to_string());
                let _ = events.send(Event::Created(result));
            }
            Request::Rename { id, name } => {
                let result = (|| {
                    library.execute(
                        id,
                        Command::RenameProject {
                            name: name.as_str().into(),
                        },
                    )?;
                    library.list()
                })()
                .map_err(|error: engine::EditError| error.to_string());
                let _ = events.send(Event::Renamed(result));
            }
            Request::Duplicate { sources } => {
                let mut failures = Vec::new();
                for source in sources {
                    let result = (|| {
                        let entries = library.list()?;
                        let next = entries
                            .iter()
                            .map(|entry| entry.id.0)
                            .max()
                            .unwrap_or(0)
                            .checked_add(1)
                            .ok_or(engine::EditError::Limit)?;
                        library.execute(
                            source,
                            Command::DuplicateProject {
                                id: ProjectId(next),
                            },
                        )?;
                        Ok(())
                    })()
                    .map_err(|error: engine::EditError| error.to_string());
                    if let Err(error) = &result {
                        failures.push(error.clone());
                    }
                    let _ = events.send(Event::Duplicated { result });
                }
                let entries = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::DuplicateFinished { failures, entries });
            }
            Request::Delete { ids } => {
                let mut failures = Vec::new();
                for id in ids {
                    if let Err(error) = library.execute(id, Command::DeleteProject {}) {
                        failures.push(error.to_string());
                    }
                }
                let entries = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::Deleted { failures, entries });
            }
            Request::Thumbnail { id } => {
                let result = library
                    .execute(id, Command::ThumbnailProject {})
                    .and_then(|_| library.list())
                    .map_err(|error| error.to_string());
                let _ = events.send(Event::Thumbnail(result));
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use engine::{ByteBudget, Command};
    use project::ProjectId;

    fn library(root: &std::path::Path) -> ProjectLibrary {
        ProjectLibrary::open(root, ByteBudget::new(64 << 20).expect("budget")).expect("library")
    }

    #[test]
    fn unique_names_follow_the_created_order_and_stop_at_a_free_suffix() {
        let root = tempfile::tempdir().expect("temporary directory");
        let mut library = library(root.path());
        for (id, name) in [(1u64, "New project"), (2, "New project 2"), (3, "Other")] {
            library
                .execute(
                    ProjectId(id),
                    Command::CreateProject {
                        id: ProjectId(id),
                        name: name.into(),
                    },
                )
                .expect("create");
        }
        let entries = library.list().expect("list");
        assert_eq!(unique_name(&entries, "New project"), "New project 3");
        assert_eq!(unique_name(&entries, "Other"), "Other 2");
    }

    #[test]
    fn duration_display_is_stable_for_boundaries() {
        assert_eq!(display_time(Tick::ZERO), "00:00:00.000");
        assert_eq!(display_time(Tick(-1)), "00:00:00.000");
        assert_eq!(
            display_time(Tick::from_seconds_f64(3661.5, time::Rounding::Nearest).unwrap()),
            "01:01:01.500"
        );
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
                        },
                    )
                    .expect("create");
            }
        }
        let mut page =
            ProjectsPage::new(root.path().to_owned(), ByteBudget::new(64 << 20).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while page.busy() {
            page.tick();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
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
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| {
                crate::Editor::fonts(ctx);
                icons::install(ctx);
            })
            .with_size(egui::vec2(1440.0, 900.0))
            .build_ui_state(
                |ui, state: &mut (ProjectsPage, Preferences)| {
                    crate::Editor::theme(ui.ctx(), state.1.dark);
                    state.0.tick();
                    state.0.ui(ui, &mut state.1);
                },
                (page, Preferences::default()),
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
    fn thumbnail_refresh_replaces_only_changed_textures() {
        let root = tempfile::tempdir().expect("directory");
        let mut catalog = library(root.path());
        catalog
            .execute(
                ProjectId(1),
                Command::CreateProject {
                    id: ProjectId(1),
                    name: "Alpha".into(),
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
        let mut page =
            ProjectsPage::new(root.path().to_owned(), ByteBudget::new(64 << 20).unwrap());
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
