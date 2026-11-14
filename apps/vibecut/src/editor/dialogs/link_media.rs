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
};
use engine::Command;
use media::{AssetAvailability, AssetId, SourceAsset};
use std::{
    collections::VecDeque,
    ffi::OsStr,
    path::{Path, PathBuf},
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

/// The dialog's state across frames.
pub(in crate::editor) struct State {
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
}

pub(in crate::editor) enum Action {
    Remember(String, egui::Response),
    Pick(Pick),
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
    let modal = egui::Modal::new(egui::Id::new("link_media.dialog"))
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
                        egui::Id::new(("link_media.row", row.id.0)),
                        if done {
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
    ui.horizontal(|ui| {
        let response = kit::checkbox(
            ui,
            "link_media.folder",
            &mut state.folder,
            Text::LinkMediaFolder.text(cx.locale),
        );
        actions.push(Action::Remember("link_media.folder".to_owned(), response));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let link = footer_button(ui, cx, "link_media.link", Text::LinkMediaAction, true);
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
            let cancel = footer_button(ui, cx, "link_media.cancel", Text::Cancel, false);
            if cancel.clicked() {
                actions.push(Action::Close);
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
        Action::Close => state.open = false,
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

/// Relink what the native pick found. A file relinks the asset it was asked for, a folder
/// every missing asset whose file name it holds; only a file with the recorded content
/// hash links. All hits are one undo step. Nothing found leaves a line in the dialog.
pub(in crate::editor) fn picked(state: &mut State, port: &mut Port<'_>, pick: Pick, path: PathBuf) {
    let assets = port.session.project().data().assets.clone();
    let mut found = Vec::new();
    match pick {
        Pick::File(id) => {
            if let Some(asset) = missing(&assets).find(|asset| asset.id() == id)
                && let Ok(relinked) = asset.relink(path)
            {
                found.push(relinked);
            }
        }
        Pick::Folder => {
            for asset in missing(&assets) {
                if let Some(name) = asset.path().file_name()
                    && let Some(hit) = find_by_name(&path, name)
                    && let Ok(relinked) = asset.relink(hit)
                {
                    found.push(relinked);
                }
            }
        }
    }
    if found.is_empty() {
        state.notice = Some(Text::LinkMediaNoMatch);
        return;
    }
    state.notice = None;
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
