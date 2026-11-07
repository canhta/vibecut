//! Draws the Projects section: heading, toolbar and the grid or list of project cards.

use super::{
    CARD_RADIUS, GRID_BLEED, GRID_GAP, GRID_INSET, ROW_GAP, THUMB, THUMB_RADIUS,
    widgets::{display_time, line, menu_action, quiet_buttons, semibold, set_author_id},
};
use crate::{
    catalog::{self, Locale, Preferences, ProjectsSortKey, ProjectsView, Text},
    egui,
    home::state::{DeleteDialog, RenameDialog, State},
    kit::{self, Icon},
};
use engine::library::ProjectEntry;
use project::{ProjectId, Thumbnail};
use std::collections::BTreeMap;

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

const SEARCH_WIDTH: f32 = 148.0;
const LIST_ROW: f32 = 56.0;
const LIST_THUMB: f32 = 40.0;

pub(super) fn section(ui: &mut egui::Ui, state: &mut State, preferences: &mut Preferences) {
    let locale = preferences.locale;
    let dark = preferences.dark;
    let tr = |key: Text| catalog::text(locale, key);

    let shown = state.order(preferences);
    let order: Vec<ProjectId> = shown
        .iter()
        .filter_map(|index| state.entries.get(*index).map(|entry| entry.id))
        .collect();
    header(ui, state, preferences, &order);
    ui.add_space(16.0);
    notices(ui, state, locale, dark);
    if !state.ready {
        // One explicit scope for the transient loading row: the catalog
        // arrives asynchronously, so its widget count must not depend on how
        // many controls the ready grid would allocate in the same position.
        ui.scope_builder(
            egui::UiBuilder::new().scope_id(ui.make_persistent_id("projects.loading")),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr(Text::LoadingProject));
                });
            },
        );
        return;
    }

    let mut actions = Vec::new();
    // One explicit scope for the body: a notice row above appears and leaves
    // asynchronously, and without it every card below would derive a new
    // widget id and drop a press that straddles the update.
    ui.scope_builder(
        egui::UiBuilder::new().scope_id(ui.make_persistent_id("projects.body")),
        |ui| {
            if shown.is_empty() {
                let text = if state.entries.is_empty() {
                    Text::ProjectsEmpty
                } else {
                    Text::ProjectsNoResults
                };
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.add(kit::tinted(Icon::FilmStrip, 24.0, kit::secondary(dark)));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(tr(text)).color(kit::secondary(dark)));
                });
            } else {
                ui.add_enabled_ui(!state.busy(), |ui| match preferences.projects.view {
                    ProjectsView::Grid => grid(ui, state, &shown, locale, &mut actions),
                    ProjectsView::List => list(ui, state, &shown, locale, &mut actions),
                });
            }
        },
    );
    ui.add_space(24.0);
    apply(state, actions, &order);
}

fn notices(ui: &mut egui::Ui, state: &mut State, locale: Locale, dark: bool) {
    let tr = |key: Text| catalog::text(locale, key);
    if let Some((text, detail)) = &state.notice {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr(*text)).color(kit::error_text(dark)));
            if let Some(detail) = detail.as_ref().filter(|detail| !detail.is_empty()) {
                ui.label(
                    egui::RichText::new(detail)
                        .small()
                        .color(kit::secondary(dark)),
                );
            }
        });
        ui.add_space(6.0);
    }
    if let Some(error) = state.list_error.clone() {
        let mut retry = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr(Text::ProjectsListError)).color(kit::error_text(dark)));
            if !error.is_empty() {
                ui.label(
                    egui::RichText::new(&error)
                        .small()
                        .color(kit::secondary(dark)),
                );
            }
            ui.add_enabled_ui(!state.busy(), |ui| {
                retry = kit::icon_button(
                    ui,
                    "projects.retry",
                    Icon::ArrowClockwise,
                    tr(Text::ProjectsRetry),
                    false,
                )
                .clicked();
            });
        });
        if retry {
            state.retry();
        }
        ui.add_space(6.0);
    }
}

/// Heading at the left, 24 px controls at the right.
fn header(
    ui: &mut egui::Ui,
    state: &mut State,
    preferences: &mut Preferences,
    order: &[ProjectId],
) {
    let locale = preferences.locale;
    let dark = preferences.dark;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, kit::size::CONTROL),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new(catalog::text(locale, Text::Projects))
                    .font(semibold(16.0))
                    .color(kit::text(dark)),
            );
            if !state.ready {
                return;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                toolbar(ui, state, preferences, order);
            });
        },
    );
}

/// Right to left: selection actions, view switch, sort, search.
fn toolbar(
    ui: &mut egui::Ui,
    state: &mut State,
    preferences: &mut Preferences,
    order: &[ProjectId],
) {
    let locale = preferences.locale;
    let dark = preferences.dark;
    let selected: Vec<ProjectId> = state
        .selection
        .iter()
        .copied()
        .filter(|id| order.contains(id))
        .collect();
    let has_selection = !selected.is_empty();
    let actionable = has_selection && !state.busy();
    let tr = |key: Text| catalog::text(locale, key);

    ui.add_enabled_ui(actionable, |ui| {
        if kit::icon_button(
            ui,
            "projects.toolbar.delete",
            Icon::Trash,
            tr(Text::Delete),
            false,
        )
        .clicked()
        {
            state.delete = Some(DeleteDialog {
                ids: selected.clone(),
                error: None,
            });
        }
        if kit::icon_button(
            ui,
            "projects.toolbar.duplicate",
            Icon::Copy,
            tr(Text::ProjectsDuplicate),
            false,
        )
        .clicked()
        {
            state.selection = selected.iter().copied().collect();
            state.request_duplicate(selected.clone());
        }
    });
    if let Some((done, total)) = state.duplicate {
        let done = catalog::number(locale, done as u64);
        let total = catalog::number(locale, total as u64);
        ui.label(
            egui::RichText::new(catalog::format(
                locale,
                Text::ProjectsDuplicating,
                &[("done", &done), ("total", &total)],
            ))
            .color(kit::secondary(dark)),
        );
    } else if has_selection {
        let count = catalog::number(locale, selected.len() as u64);
        ui.label(
            egui::RichText::new(catalog::format(
                locale,
                Text::ProjectsSelected,
                &[("n", &count)],
            ))
            .color(kit::secondary(dark)),
        );
    }

    let (pill, _) = ui.allocate_exact_size(
        egui::vec2(kit::size::CONTROL * 2.0, kit::size::CONTROL),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(pill, kit::radius::CONTROL, kit::field(dark));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(pill)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (id, glyph, label, view) in [
                (
                    "projects.view.grid",
                    Icon::SquaresFour,
                    Text::ProjectsGrid,
                    ProjectsView::Grid,
                ),
                (
                    "projects.view.list",
                    Icon::ListBullets,
                    Text::ProjectsList,
                    ProjectsView::List,
                ),
            ] {
                let on = preferences.projects.view == view;
                if kit::icon_button(ui, id, glyph, tr(label), on).clicked() {
                    let mut next = preferences.projects;
                    next.view = view;
                    state.update_projects(preferences, next);
                }
            }
        },
    );

    sort_menu(ui, state, preferences);

    // The old text-first search field stays a visible field so typing needs no
    // extra click; it takes the 24 px control height and the field surface.
    let (field, _) = ui.allocate_exact_size(
        egui::vec2(SEARCH_WIDTH, kit::size::CONTROL),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(field, kit::radius::CONTROL, kit::field(dark));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(field.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(kit::tinted(
                Icon::MagnifyingGlass,
                kit::icon::DENSE,
                kit::secondary(dark),
            ));
            let search = ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .frame(egui::Frame::NONE)
                    .font(egui::FontId::proportional(kit::type_size::BODY))
                    .hint_text(tr(Text::ProjectsSearch))
                    .desired_width(SEARCH_WIDTH - 12.0 - 20.0),
            );
            set_author_id(&search, "projects.search");
        },
    );
}

fn sort_menu(ui: &mut egui::Ui, state: &mut State, preferences: &mut Preferences) {
    let locale = preferences.locale;
    let sort = preferences.projects;
    let order_label = if sort.ascending {
        Text::ProjectsSortAscending
    } else {
        Text::ProjectsSortDescending
    };
    let value = catalog::format(
        locale,
        Text::ProjectsSortValue,
        &[
            ("key", catalog::text(locale, sort_key_text(sort.sort))),
            ("order", catalog::text(locale, order_label)),
        ],
    );
    let label = catalog::format(locale, Text::ProjectsSort, &[("sort", &value)]);
    let response = ui
        .scope(|ui| {
            quiet_buttons(ui);
            ui.menu_image_button(
                kit::tinted(
                    Icon::Funnel,
                    kit::icon::DENSE,
                    kit::text(ui.visuals().dark_mode),
                ),
                |ui| {
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
                            state.update_projects(preferences, next);
                            ui.close();
                        }
                    }
                },
            )
            .response
        })
        .inner
        .on_hover_text(label);
    set_author_id(&response, "projects.sort");
}

fn sort_key_text(key: ProjectsSortKey) -> Text {
    match key {
        ProjectsSortKey::Created => Text::ProjectsSortCreated,
        ProjectsSortKey::Modified => Text::ProjectsSortModified,
        ProjectsSortKey::Name => Text::ProjectsSortName,
        ProjectsSortKey::Duration => Text::ProjectsSortDuration,
    }
}

/// Square thumbnails on the measured pitch, rows wrapping at the page width.
fn grid(
    ui: &mut egui::Ui,
    state: &mut State,
    shown: &[usize],
    locale: Locale,
    actions: &mut Vec<CardAction>,
) {
    let row_width = ui.available_width() + GRID_BLEED;
    let pitch = THUMB + GRID_GAP;
    let columns = (((row_width - GRID_INSET + GRID_GAP) / pitch).floor() as usize).max(1);
    let text_height = 2.0 + 16.0 + 16.0;
    for row in shown.chunks(columns) {
        let (band, _) = ui.allocate_exact_size(
            egui::vec2(row_width, THUMB + text_height),
            egui::Sense::hover(),
        );
        for (column, index) in row.iter().enumerate() {
            let Some(entry) = state.entries.get(*index).cloned() else {
                continue;
            };
            let left = band.min.x + GRID_INSET + column as f32 * pitch;
            let thumb =
                egui::Rect::from_min_size(egui::pos2(left, band.min.y), egui::Vec2::splat(THUMB));
            let texts = egui::Rect::from_min_size(
                egui::pos2(left, thumb.max.y + 2.0),
                egui::vec2(THUMB, 32.0),
            );
            let selected = state.selection.contains(&entry.id);
            card(
                ui,
                &entry,
                selected,
                thumb,
                texts,
                false,
                locale,
                &mut state.textures,
                actions,
            );
        }
        ui.add_space(ROW_GAP);
    }
}

fn list(
    ui: &mut egui::Ui,
    state: &mut State,
    shown: &[usize],
    locale: Locale,
    actions: &mut Vec<CardAction>,
) {
    let dark = ui.visuals().dark_mode;
    for index in shown {
        let Some(entry) = state.entries.get(*index).cloned() else {
            continue;
        };
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), LIST_ROW),
            egui::Sense::hover(),
        );
        let selected = state.selection.contains(&entry.id);
        if selected {
            ui.painter().rect_filled(row, CARD_RADIUS, kit::hover(dark));
        }
        let thumb = egui::Rect::from_min_size(
            egui::pos2(row.min.x + 8.0, row.center().y - LIST_THUMB / 2.0),
            egui::Vec2::splat(LIST_THUMB),
        );
        let texts = egui::Rect::from_min_max(
            egui::pos2(thumb.max.x + 12.0, row.center().y - 16.0),
            egui::pos2(row.max.x - 48.0, row.center().y + 16.0),
        );
        card(
            ui,
            &entry,
            selected,
            thumb,
            texts,
            true,
            locale,
            &mut state.textures,
            actions,
        );
    }
}

/// One project: thumbnail, name and `duration | date`, plus its menu.
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    selected: bool,
    thumb: egui::Rect,
    texts: egui::Rect,
    menu_always: bool,
    locale: Locale,
    textures: &mut BTreeMap<ProjectId, egui::TextureHandle>,
    actions: &mut Vec<CardAction>,
) {
    let dark = ui.visuals().dark_mode;
    let prefix = format!("projects.card.{:016x}", entry.id.0);

    thumbnail(ui, entry, thumb, locale, textures, actions);
    if selected {
        ui.painter().rect_stroke(
            thumb,
            THUMB_RADIUS,
            egui::Stroke::new(2.0, kit::accent(dark)),
            egui::StrokeKind::Inside,
        );
    } else if ui.rect_contains_pointer(thumb) {
        ui.painter().rect_stroke(
            thumb,
            THUMB_RADIUS,
            egui::Stroke::new(kit::space::HAIRLINE, kit::border(dark)),
            egui::StrokeKind::Inside,
        );
    }

    let name = egui::Rect::from_min_size(texts.min, egui::vec2(texts.width(), 16.0));
    let response = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(name)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(entry.name.as_str())
                            .font(semibold(kit::type_size::BODY))
                            .color(kit::text(dark)),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                )
            },
        )
        .inner;
    set_author_id(&response, &prefix);
    open_or_select(ui, &response, entry.id, locale, actions);

    let meta = format!(
        "{} | {}",
        display_time(entry.duration),
        catalog::date(locale, entry.created_unix_ms)
    );
    let meta_rect = egui::Rect::from_min_size(
        egui::pos2(texts.min.x, texts.min.y + 16.0),
        egui::vec2(texts.width(), 16.0),
    );
    let galley = line(
        ui,
        &meta,
        egui::FontId::proportional(kit::type_size::BODY),
        kit::secondary(dark),
        meta_rect.width(),
    );
    ui.painter()
        .galley(meta_rect.min, galley, kit::secondary(dark));

    let menu = egui::Rect::from_min_size(
        if menu_always {
            egui::pos2(texts.max.x + 12.0, thumb.center().y - 12.0)
        } else {
            egui::pos2(thumb.max.x - 4.0 - 24.0, thumb.min.y + 4.0)
        },
        egui::Vec2::splat(kit::size::CONTROL),
    );
    let show_menu = menu_always || selected || ui.rect_contains_pointer(thumb.union(menu));
    ui.scope_builder(egui::UiBuilder::new().max_rect(menu), |ui| {
        quiet_buttons(ui);
        if !menu_always {
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = kit::scrim();
            widgets.hovered.weak_bg_fill = kit::scrim();
            widgets.active.weak_bg_fill = kit::scrim();
            widgets.open.weak_bg_fill = kit::scrim();
        }
        if !show_menu {
            ui.set_opacity(0.0);
        }
        let tint = if menu_always {
            kit::text(dark)
        } else {
            kit::on_scrim()
        };
        let response = ui
            .menu_image_button(kit::tinted(Icon::DotsThree, kit::icon::DENSE, tint), |ui| {
                card_menu_items(ui, &entry.id, actions, locale);
            })
            .response
            .on_hover_text(catalog::text(locale, Text::ProjectsCardMenu));
        set_author_id(&response, &format!("{prefix}.menu"));
    });
}

fn open_or_select(
    ui: &egui::Ui,
    response: &egui::Response,
    id: ProjectId,
    locale: Locale,
    actions: &mut Vec<CardAction>,
) {
    if response.clicked() {
        actions.push(CardAction::Select {
            id,
            modifiers: ui.input(|input| input.modifiers),
        });
    }
    if response.double_clicked() {
        actions.push(CardAction::Open(id));
    }
    response.context_menu(|ui| card_menu_items(ui, &id, actions, locale));
}

fn thumbnail(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    rect: egui::Rect,
    locale: Locale,
    textures: &mut BTreeMap<ProjectId, egui::TextureHandle>,
    actions: &mut Vec<CardAction>,
) {
    let dark = ui.visuals().dark_mode;
    let prefix = format!("projects.card.{:016x}", entry.id.0);
    let response = ui.interact(
        rect,
        ui.scope_id().with((&prefix, "thumbnail")),
        egui::Sense::click(),
    );
    set_author_id(&response, &format!("{prefix}.thumbnail"));
    open_or_select(ui, &response, entry.id, locale, actions);
    // Missing and unrenderable frames sit on the field surface, as the black
    // frame of an unrendered clip does.
    ui.painter()
        .rect_filled(rect, THUMB_RADIUS, kit::field(dark));
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
            // Cover the square: scale to fill, crop the longer side evenly.
            let image = egui::vec2(*width as f32, *height as f32);
            let scale = (rect.width() / image.x)
                .max(rect.height() / image.y)
                .max(f32::EPSILON);
            let shown = rect.size() / (image * scale);
            let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), shown);
            egui::Image::new((texture.id(), rect.size()))
                .uv(uv)
                .corner_radius(THUMB_RADIUS)
                .tint(kit::image_tint())
                .paint_at(ui, rect);
        }
        Thumbnail::Missing | Thumbnail::Unavailable => {
            let glyph = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(24.0));
            let placeholder = ui.put(
                glyph,
                kit::tinted(Icon::FilmStrip, 24.0, kit::secondary(dark))
                    .alt_text(catalog::text(locale, Text::ProjectsThumbnailMissing)),
            );
            set_author_id(&placeholder, &format!("{prefix}.thumbnail.placeholder"));
        }
    }
}

fn card_menu_items(
    ui: &mut egui::Ui,
    id: &ProjectId,
    actions: &mut Vec<CardAction>,
    locale: Locale,
) {
    let prefix = format!("projects.card.{:016x}", id.0);
    let rows = [
        (
            "open",
            Icon::FolderOpen,
            Text::ProjectsOpen,
            CardAction::Open(*id),
        ),
        (
            "rename",
            Icon::PencilSimple,
            Text::ProjectsRename,
            CardAction::Rename(*id),
        ),
        (
            "info",
            Icon::File,
            Text::ProjectsInfoTitle,
            CardAction::Info(*id),
        ),
        (
            "duplicate",
            Icon::Copy,
            Text::ProjectsDuplicate,
            CardAction::Duplicate(*id),
        ),
        ("delete", Icon::Trash, Text::Delete, CardAction::Delete(*id)),
        (
            "thumbnail.refresh",
            Icon::ArrowClockwise,
            Text::ProjectsRefreshThumbnail,
            CardAction::Thumbnail(*id),
        ),
    ];
    for (suffix, icon, text, action) in rows {
        if menu_action(
            ui,
            &format!("{prefix}.{suffix}"),
            icon,
            catalog::text(locale, text),
        ) {
            actions.push(action);
            ui.close();
        }
    }
}

/// Apply what the cards asked for, once the frame's widgets are done.
fn apply(state: &mut State, actions: Vec<CardAction>, order: &[ProjectId]) {
    for action in actions {
        match action {
            CardAction::Select { id, modifiers } => state.select(id, order, modifiers),
            CardAction::Rename(id) => {
                let Some(entry) = state.entries.iter().find(|entry| entry.id == id) else {
                    continue;
                };
                state.rename = Some(RenameDialog {
                    id,
                    name: entry.name.as_str().to_owned(),
                    error: None,
                });
            }
            CardAction::Info(id) => state.info = Some(id),
            CardAction::Open(id) => {
                if let Some(entry) = state.entries.iter().find(|entry| entry.id == id) {
                    state.action = Some(crate::home::state::Action::Open(entry.path.clone()));
                }
            }
            CardAction::Duplicate(id) => {
                state.selection.clear();
                state.selection.insert(id);
                state.anchor = Some(id);
                state.request_duplicate(vec![id]);
            }
            CardAction::Delete(id) => {
                state.delete = Some(DeleteDialog {
                    ids: vec![id],
                    error: None,
                });
            }
            CardAction::Thumbnail(id) => state.request_thumbnail(id),
        }
    }
}
