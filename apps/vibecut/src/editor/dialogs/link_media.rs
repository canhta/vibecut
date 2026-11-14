//! Link media: lists the sources whose file is gone and relinks them to the files found.
//!
//! Layout is measured on CapCut 9.7.1 (editor.md "Missing media"). The wording is ours.
//! The dialog never reads files itself beyond the folder search: the match is
//! `SourceAsset::relink`, which accepts only a file with the recorded content hash, and
//! the edit is the `media.relink` command, one undo step for every asset linked at once.
//! A native pick leaves as a [`Pick`] request the window serves through `app/picker.rs`.
#[cfg(test)]
mod tests;

use crate::{
    editor::{Context, Port, egui, media_panel},
    i18n::Text,
    kit,
    services::progress,
};
use engine::Command;
use media::{AssetAvailability, AssetId, SourceAsset};
use std::{
    collections::VecDeque,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use time::TICKS_PER_SECOND;

/// Logical px: header row, data row and the left edges of the three columns (095518).
const HEADER_HEIGHT: f32 = 29.0;
const ROW_HEIGHT: f32 = 50.0;
const NAME_X: f32 = 36.0;
const PATH_X: f32 = 206.0;
const DURATION_INSET: f32 = 24.0;
/// The path column shows this many characters, the middle replaced by an ellipsis.
const PATH_CHARS: usize = 34;
/// A folder pick searches this deep and looks at no more than this many entries.
const SEARCH_DEPTH: usize = 4;
const SEARCH_ENTRIES: usize = 20_000;
/// The worker reports hashing progress at most once per this many bytes.
const PROGRESS_STEP: u64 = 4 << 20;

/// What the native picker is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// One file for one asset.
    File(AssetId),
    /// A folder searched for every missing asset's file name.
    Folder,
}

/// One missing source as the dialog listed it when it opened.
struct Row {
    id: AssetId,
    name: String,
    last: PathBuf,
    seconds: Option<u64>,
}

/// What hashing one candidate file concluded.
pub(in crate::editor) enum Hashed {
    /// The file has the recorded content hash: the relinked record.
    Match(SourceAsset),
    /// Not the file (other content, unreadable).
    Miss,
    /// The cancel flag stopped the read.
    Cancelled,
}

/// Hashes `candidate` against the asset's recorded content, checking the flag before each
/// read and reporting `(bytes hashed, file length)`. The desktop uses
/// `SourceAsset::relink_with_cancel`; tests inject a slow or gated one.
pub(in crate::editor) type Hasher = Arc<
    dyn Fn(&SourceAsset, PathBuf, &AtomicBool, &mut dyn FnMut(u64, u64)) -> Hashed + Send + Sync,
>;

fn real_hasher() -> Hasher {
    Arc::new(
        |asset: &SourceAsset,
         path: PathBuf,
         cancelled: &AtomicBool,
         progress: &mut dyn FnMut(u64, u64)| {
            match asset.relink_with_cancel(
                path,
                || cancelled.load(Ordering::Relaxed),
                |done, total| progress(done, total),
            ) {
                Ok(relinked) => Hashed::Match(relinked),
                Err(media::MediaError::Cancelled) => Hashed::Cancelled,
                Err(_) => Hashed::Miss,
            }
        },
    )
}

/// What the worker thread tells the dialog.
enum Event {
    /// The worker starts hashing the file for this source name.
    Start(String),
    Progress(u64, u64),
    Matched(SourceAsset),
    Done,
}

/// One background match: a worker thread hashing candidates one file at a time. The
/// project is untouched until the worker is done and the dialog commits the hits.
struct Job {
    cancel: Arc<AtomicBool>,
    events: mpsc::Receiver<Event>,
    /// Name, bytes hashed and file length of the file being read.
    current: Option<(String, u64, u64)>,
    matched: Vec<SourceAsset>,
}

impl Drop for Job {
    /// Leaving the dialog or the project stops the read at its next 64 KiB.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The dialog's state across frames.
pub(in crate::editor) struct State {
    job: Option<Job>,
    hasher: Hasher,
    open: bool,
    /// The folder pick links every asset it finds; off, the button picks one file.
    folder: bool,
    rows: Vec<Row>,
    notice: Option<Text>,
    request: Option<Pick>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            folder: true,
            job: None,
            hasher: real_hasher(),
            rows: Vec::new(),
            notice: None,
            request: None,
        }
    }
}

/// What the dialog shows, for inspection and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkMediaView {
    pub open: bool,
    pub names: Vec<String>,
    pub linked: usize,
    pub folder: bool,
    /// A background match is running, and the file it is reading with its progress.
    pub matching: bool,
    pub progress: Option<(String, u64, u64)>,
}

pub(in crate::editor) enum Action {
    Remember(String, egui::Response),
    Pick(Pick),
    /// Advance the background match: progress, and the commit once it is done.
    Poll,
    /// Stop the background match; nothing is linked.
    CancelJob,
    Close,
}

fn missing(assets: &[SourceAsset]) -> impl Iterator<Item = &SourceAsset> {
    assets
        .iter()
        .filter(|asset| asset.availability().ok() == Some(AssetAvailability::Missing))
}

impl State {
    pub(in crate::editor) fn is_open(&self) -> bool {
        self.open
    }

    /// Replace the hasher; tests gate or slow it deterministically.
    #[cfg(test)]
    pub(in crate::editor) fn set_hasher(&mut self, hasher: Hasher) {
        self.hasher = hasher;
    }

    /// Open on the sources that are missing now. Nothing missing leaves it closed.
    pub(in crate::editor) fn open(&mut self, project: &project::Project) {
        self.rows = missing(&project.data().assets)
            .map(|asset| Row {
                id: asset.id(),
                name: media_panel::asset_name(asset),
                last: asset.path().to_owned(),
                seconds: asset
                    .probe()
                    .duration
                    .map(|d| (d.0 as f64 / TICKS_PER_SECOND as f64).round() as u64),
            })
            .collect();
        self.notice = None;
        self.request = None;
        self.job = None;
        self.open = !self.rows.is_empty();
    }

    /// The pick waiting for the native dialog, if any; taking it hands the request over.
    pub(in crate::editor) fn take_request(&mut self) -> Option<Pick> {
        self.request.take()
    }

    pub(in crate::editor) fn view(&self, project: &project::Project) -> LinkMediaView {
        LinkMediaView {
            open: self.open,
            names: self.rows.iter().map(|row| row.name.clone()).collect(),
            linked: self.linked(project),
            folder: self.folder,
            matching: self.job.is_some(),
            progress: self.job.as_ref().and_then(|job| job.current.clone()),
        }
    }

    fn is_linked(&self, project: &project::Project, row: &Row) -> bool {
        project
            .data()
            .assets
            .iter()
            .find(|asset| asset.id() == row.id)
            .is_some_and(|asset| asset.path() != row.last.as_path())
    }

    fn linked(&self, project: &project::Project) -> usize {
        self.rows
            .iter()
            .filter(|row| self.is_linked(project, row))
            .count()
    }
}

/// Paint the dialog and return what it asks for.
pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if !state.open {
        return actions;
    }
    let frame = kit::dialog_frame(ui.ctx()).inner_margin(kit::size::LINK_DIALOG_INSET);
    let modal = egui::Modal::new(egui::Id::unique("link_media.dialog"))
        .frame(frame)
        .show(ui.ctx(), |ui| {
            ui.set_width(kit::size::LINK_DIALOG_WIDTH - 2.0 * kit::size::LINK_DIALOG_INSET);
            kit::dialog_title(ui, Text::LinkMediaTitle.text(cx.locale));
            ui.add_space(kit::space::LG);
            ui.label(Text::LinkMediaBody.text(cx.locale));
            ui.add_space(kit::space::LG);
            list(state, cx, ui, &mut actions);
            ui.add_space(kit::space::LG);
            footer(state, cx, ui, &mut actions);
        });
    if modal.should_close() {
        actions.push(Action::Close);
    }
    if state.job.is_some() {
        actions.push(Action::Poll);
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
    actions
}

fn list(state: &State, cx: &Context<'_>, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let dark = cx.dark;
    let project = cx.staged_project();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), kit::size::LINK_LIST_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, kit::radius::FRAME_PANEL, kit::window(dark));
    let font = || egui::FontId::proportional(kit::type_size::BODY);
    let head_y = rect.top() + HEADER_HEIGHT / 2.0;
    for (x, align, text) in [
        (
            rect.left() + NAME_X,
            egui::Align2::LEFT_CENTER,
            Text::LinkMediaName,
        ),
        (
            rect.left() + PATH_X,
            egui::Align2::LEFT_CENTER,
            Text::LinkMediaLastPath,
        ),
        (
            rect.right() - DURATION_INSET,
            egui::Align2::RIGHT_CENTER,
            Text::LinkMediaDuration,
        ),
    ] {
        painter.text(
            egui::pos2(x, head_y),
            align,
            text.text(cx.locale),
            font(),
            kit::secondary(dark),
        );
    }
    let rule = egui::Stroke::new(1.0, kit::separator(dark));
    let inset = kit::space::LG;
    painter.hline(
        (rect.left() + inset)..=(rect.right() - inset),
        rect.top() + HEADER_HEIGHT,
        rule,
    );
    let foot_y = rect.bottom() - kit::size::LINK_LIST_FOOTER;
    painter.hline((rect.left())..=(rect.right()), foot_y, rule);
    let linked = state.linked(project);
    let total = state.rows.len();
    let tone = if linked == total {
        kit::success(dark)
    } else {
        kit::warning(dark)
    };
    let count = painter.text(
        egui::pos2(
            rect.right() - kit::size::LINK_DIALOG_INSET,
            foot_y + kit::size::LINK_LIST_FOOTER / 2.0,
        ),
        egui::Align2::RIGHT_CENTER,
        Text::LinkMediaLinked.text(cx.locale),
        font(),
        kit::secondary(dark),
    );
    painter.text(
        egui::pos2(count.left() - 4.0, count.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{linked}/{total}"),
        font(),
        tone,
    );
    let rows = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + HEADER_HEIGHT + 1.0),
        egui::pos2(rect.right(), foot_y),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("link_media.rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                for row in &state.rows {
                    let (cell, _) = ui.allocate_exact_size(
                        egui::vec2(rows.width(), ROW_HEIGHT),
                        egui::Sense::hover(),
                    );
                    let done = state.is_linked(project, row);
                    let ink = if done {
                        kit::success(dark)
                    } else {
                        kit::accent(dark)
                    };
                    let painter = ui.painter_at(cell);
                    let y = cell.center().y;
                    painter.text(
                        egui::pos2(cell.left() + NAME_X, y),
                        egui::Align2::LEFT_CENTER,
                        &row.name,
                        font(),
                        ink,
                    );
                    painter.text(
                        egui::pos2(cell.left() + PATH_X, y),
                        egui::Align2::LEFT_CENTER,
                        ellipsize(&row.last),
                        font(),
                        ink,
                    );
                    if let Some(seconds) = row.seconds {
                        painter.text(
                            egui::pos2(cell.right() - DURATION_INSET, y),
                            egui::Align2::RIGHT_CENTER,
                            format!("{:02}:{:02}", seconds / 60, seconds % 60),
                            font(),
                            ink,
                        );
                    }
                    let id = format!("link_media.row.{}", row.id.0);
                    let response = ui.interact(
                        cell,
                        egui::Id::unique(("link_media.row", row.id.0)),
                        if done || state.job.is_some() {
                            egui::Sense::hover()
                        } else {
                            egui::Sense::click()
                        },
                    );
                    if response.clicked() {
                        actions.push(Action::Pick(Pick::File(row.id)));
                    }
                    actions.push(Action::Remember(id, response));
                }
            });
    });
}

fn footer(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    if let Some(notice) = state.notice {
        ui.colored_label(kit::error_text(cx.dark), notice.text(cx.locale));
        ui.add_space(kit::space::MD);
    }
    let busy = state.job.is_some();
    if let Some(job) = &state.job {
        let checking = Text::LinkMediaChecking.text(cx.locale);
        match &job.current {
            Some((name, done, total)) => progress::line(
                ui,
                "link_media.progress",
                cx.locale,
                &format!("{checking} {name}"),
                *done as f32 / (*total).max(1) as f32,
            ),
            None => {
                ui.label(checking);
            }
        }
        ui.add_space(kit::space::MD);
    }
    ui.horizontal(|ui| {
        let response = ui
            .add_enabled_ui(!busy, |ui| {
                kit::checkbox(
                    ui,
                    "link_media.folder",
                    &mut state.folder,
                    Text::LinkMediaFolder.text(cx.locale),
                )
            })
            .inner;
        actions.push(Action::Remember("link_media.folder".to_owned(), response));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let link = ui
                .add_enabled_ui(!busy, |ui| {
                    footer_button(ui, cx, "link_media.link", Text::LinkMediaAction, true)
                })
                .inner;
            if link.clicked() {
                let first = state
                    .rows
                    .iter()
                    .find(|row| !state.is_linked(cx.staged_project(), row));
                match (state.folder, first) {
                    (true, _) => actions.push(Action::Pick(Pick::Folder)),
                    (false, Some(row)) => actions.push(Action::Pick(Pick::File(row.id))),
                    (false, None) => {}
                }
            }
            actions.push(Action::Remember("link_media.link".to_owned(), link));
            ui.add_space(kit::space::LG);
            // While a match runs Cancel stops it and links nothing; otherwise it closes.
            let cancel = footer_button(ui, cx, "link_media.cancel", Text::Cancel, false);
            if cancel.clicked() {
                actions.push(if busy {
                    Action::CancelJob
                } else {
                    Action::Close
                });
            }
            actions.push(Action::Remember("link_media.cancel".to_owned(), cancel));
        });
    });
}

/// A 72 x 22 footer button (095518); the primary one is filled with the accent.
fn footer_button(
    ui: &mut egui::Ui,
    cx: &Context<'_>,
    _id: &str,
    text: Text,
    primary: bool,
) -> egui::Response {
    let label = text.text(cx.locale);
    let button = if primary {
        egui::Button::new(egui::RichText::new(label).color(kit::on_accent(cx.dark)))
            .fill(kit::accent(cx.dark))
    } else {
        egui::Button::new(label)
    };
    ui.add_sized(
        egui::vec2(kit::size::LINK_BUTTON_WIDTH, kit::size::LINK_BUTTON_HEIGHT),
        button,
    )
}

/// Keep the head and tail of a long path with an ellipsis between them.
fn ellipsize(path: &Path) -> String {
    let text = path.to_string_lossy();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= PATH_CHARS {
        return text.into_owned();
    }
    let tail = PATH_CHARS * 2 / 3;
    let head = PATH_CHARS - tail - 1;
    let mut out: String = chars[..head].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - tail..]);
    out
}

pub(in crate::editor) fn apply(state: &mut State, port: &mut Port<'_>, action: Action) {
    match action {
        Action::Remember(id, response) => port.widget(id, None, None, &response),
        Action::Pick(pick) => state.request = Some(pick),
        Action::Poll => poll(state, port),
        Action::CancelJob => {
            if let Some(job) = &state.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        Action::Close => {
            // Dropping the job raises its cancel flag.
            state.job = None;
            state.open = false;
        }
    }
}

/// The first file named `name` under `folder`, breadth first, bounded in depth and size.
fn find_by_name(folder: &Path, name: &OsStr) -> Option<PathBuf> {
    let mut queue = VecDeque::from([(folder.to_owned(), 0_usize)]);
    let mut seen = 0_usize;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SEARCH_ENTRIES {
                return None;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_file() && path.file_name() == Some(name) {
                return Some(path);
            }
            if kind.is_dir() && depth < SEARCH_DEPTH {
                queue.push_back((path, depth + 1));
            }
        }
    }
    None
}

/// Start matching what the native pick found, off the UI thread. A file is checked against
/// the asset it was asked for, a folder is searched for every missing asset's file name and
/// each candidate is hashed one file at a time; only a file with the recorded content hash
/// links. Nothing changes until the worker is done (`poll`), then all hits are one undo step.
pub(in crate::editor) fn picked(state: &mut State, port: &mut Port<'_>, pick: Pick, path: PathBuf) {
    if state.job.is_some() {
        return;
    }
    let assets: Vec<SourceAsset> = missing(&port.session.project().data().assets)
        .filter(|asset| match pick {
            Pick::File(id) => asset.id() == id,
            Pick::Folder => true,
        })
        .cloned()
        .collect();
    state.notice = None;
    state.job = Some(spawn(assets, pick, path, state.hasher.clone()));
}

fn spawn(assets: Vec<SourceAsset>, pick: Pick, path: PathBuf, hasher: Hasher) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let (send, events) = mpsc::channel();
    let flag = cancel.clone();
    std::thread::spawn(move || {
        for asset in &assets {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let candidate = match pick {
                Pick::File(_) => Some(path.clone()),
                Pick::Folder => asset
                    .path()
                    .file_name()
                    .and_then(|name| find_by_name(&path, name)),
            };
            let Some(candidate) = candidate else {
                continue;
            };
            let _ = send.send(Event::Start(media_panel::asset_name(asset)));
            let mut last = 0_u64;
            let outcome = hasher(asset, candidate, flag.as_ref(), &mut |done, total| {
                if done >= total || done.saturating_sub(last) >= PROGRESS_STEP {
                    last = done;
                    let _ = send.send(Event::Progress(done, total));
                }
            });
            match outcome {
                Hashed::Match(relinked) => {
                    let _ = send.send(Event::Matched(relinked));
                }
                Hashed::Miss => {}
                Hashed::Cancelled => break,
            }
        }
        let _ = send.send(Event::Done);
    });
    Job {
        cancel,
        events,
        current: None,
        matched: Vec::new(),
    }
}

/// Drain the worker's events; once it is done, commit the hits.
fn poll(state: &mut State, port: &mut Port<'_>) {
    let Some(job) = state.job.as_mut() else {
        return;
    };
    let mut done = false;
    loop {
        match job.events.try_recv() {
            Ok(Event::Start(name)) => job.current = Some((name, 0, 1)),
            Ok(Event::Progress(hashed, total)) => {
                if let Some(current) = job.current.as_mut() {
                    current.1 = hashed;
                    current.2 = total.max(1);
                }
            }
            Ok(Event::Matched(asset)) => job.matched.push(asset),
            Ok(Event::Done) | Err(mpsc::TryRecvError::Disconnected) => {
                done = true;
                break;
            }
            Err(mpsc::TryRecvError::Empty) => break,
        }
    }
    if !done {
        return;
    }
    let Some(mut job) = state.job.take() else {
        return;
    };
    let cancelled = job.cancel.load(Ordering::Relaxed);
    let found = std::mem::take(&mut job.matched);
    if cancelled {
        state.notice = Some(Text::LinkMediaCancelled);
        return;
    }
    commit(state, port, found);
}

/// Relink every hit in one transaction.
fn commit(state: &mut State, port: &mut Port<'_>, found: Vec<SourceAsset>) {
    if found.is_empty() {
        state.notice = Some(Text::LinkMediaNoMatch);
        return;
    }
    let result = port
        .session
        .transaction(|session| -> Result<(), engine::EditError> {
            for asset in found {
                session.apply(Command::RelinkAsset { asset })?;
            }
            Ok(())
        });
    match result {
        Ok(()) => {
            port.sync();
            let all = state.linked(port.session.project()) == state.rows.len();
            if all {
                state.open = false;
                port.toast(port.tr(Text::LinkMediaDone), None);
            }
        }
        Err(error) => port.fail(error.to_string()),
    }
}
