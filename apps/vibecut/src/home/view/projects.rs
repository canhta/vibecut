//! Draws the Projects section: heading, toolbar and the grid or list of project cards.

use super::{
    CARD_RADIUS, GRID_BLEED, GRID_GAP, GRID_INSET, ROW_GAP, THUMB, THUMB_RADIUS,
    widgets::{display_time, line, menu_action, quiet_buttons, semibold, set_author_id},
};
use crate::{
    egui,
    home::state::{DeleteDialog, RenameDialog, State},
    home::{ProjectsPreferences, ProjectsSortKey, ProjectsView},
    i18n::{self, Locale, Text},
    kit::{self, Icon, Look},
};
use engine::library::ProjectEntry;
use project::{ProjectId, Thumbnail};
use std::collections::BTreeMap;

#[derive(Clone)]
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
    Restore(ProjectId),
    Purge(ProjectId),
    Thumbnail(ProjectId),
}

const SEARCH_WIDTH: f32 = 148.0;
const LIST_ROW: f32 = 56.0;
const LIST_THUMB: f32 = 40.0;
/// Measured width of the reference Trash control: icon and its label.
const TRASH_WIDTH: f32 = 64.0;

pub(super) fn section(
    ui: &mut egui::Ui,
    state: &mut State,
    look: Look,
    projects: &mut ProjectsPreferences,
) {
    let locale = look.locale;
    let dark = look.dark;
    let tr = |key: Text| i18n::text(locale, key);

    let trash = state.trash_mode;
    let shown = if trash {
        state.trash_order()
    } else {
        state.order(*projects)
    };
    let order: Vec<ProjectId> = if trash {
        shown
            .iter()
            .filter_map(|index| state.trash.get(*index).map(|entry| entry.id))
            .collect()
    } else {
        shown
            .iter()
            .filter_map(|index| state.entries.get(*index).map(|entry| entry.id))
            .collect()
    };
    header(ui, state, look, projects, &order);
    ui.add_space(16.0);
    notices(ui, state, locale, dark);
    if !state.ready || (trash && !state.trash_loaded) {
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
                let text = if trash {
                    if state.trash.is_empty() {
                        Text::ProjectsTrashEmpty
                    } else {
                        Text::ProjectsNoResults
                    }
                } else if state.entries.is_empty() {
                    Text::ProjectsEmpty
                } else {
                    Text::ProjectsNoResults
                };
                ui.add_space(24.0);
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), kit::size::PANEL_STATE),
                    egui::Sense::hover(),
                );
                kit::panel_state(
                    ui,
                    "projects.state",
                    rect,
                    if trash { Icon::Trash } else { Icon::FilmStrip },
                    tr(text),
                    kit::Tone::Quiet,
                );
            } else {
                ui.add_enabled_ui(!state.busy(), |ui| match projects.view {
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
    let tr = |key: Text| i18n::text(locale, key);
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
    look: Look,
    projects: &mut ProjectsPreferences,
    order: &[ProjectId],
) {
    let locale = look.locale;
    let dark = look.dark;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, kit::size::CONTROL),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new(i18n::text(
                    locale,
                    if state.trash_mode {
                        Text::ProjectsTrash
                    } else {
                        Text::Projects
                    },
                ))
                .font(semibold(16.0))
                .color(kit::text(dark)),
            );
            if !state.ready {
                return;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                toolbar(ui, state, look, projects, order);
            });
        },
    );
}

/// Right to left: selection actions, Trash, view switch, sort, search.
fn toolbar(
    ui: &mut egui::Ui,
    state: &mut State,
    look: Look,
    projects: &mut ProjectsPreferences,
    order: &[ProjectId],
) {
    let locale = look.locale;
    let dark = look.dark;
    let trash = state.trash_mode;
    let selected: Vec<ProjectId> = state
        .selection
        .iter()
        .copied()
        .filter(|id| order.contains(id))
        .collect();
    let has_selection = !selected.is_empty();
    let actionable = has_selection && !state.busy();
    let tr = |key: Text| i18n::text(locale, key);

    ui.add_enabled_ui(actionable, |ui| {
        if trash {
            if kit::icon_button(
                ui,
                "projects.toolbar.restore",
                Icon::ArrowCounterClockwise,
                tr(Text::ProjectsRestore),
                false,
            )
            .clicked()
            {
                state.request_restore(selected.clone());
            }
            if kit::icon_button(
                ui,
                "projects.toolbar.purge",
                Icon::Trash,
                tr(Text::ProjectsPurge),
                false,
            )
            .clicked()
            {
                state.delete = Some(DeleteDialog {
                    ids: selected.clone(),
                    error: None,
                    permanent: true,
                });
            }
        } else {
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
                    permanent: false,
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
        }
    });
    if let Some((done, total)) = state.duplicate {
        let done = i18n::number(locale, done as u64);
        let total = i18n::number(locale, total as u64);
        ui.label(
            egui::RichText::new(i18n::format(
                locale,
                Text::ProjectsDuplicating,
                &[("done", &done), ("total", &total)],
            ))
            .color(kit::secondary(dark)),
        );
    } else if has_selection {
        let count = i18n::number(locale, selected.len() as u64);
        ui.label(
            egui::RichText::new(i18n::format(
                locale,
                Text::ProjectsSelected,
                &[("n", &count)],
            ))
            .color(kit::secondary(dark)),
        );
    }

    // Trash sits between the view switch and the selection actions, as the
    // measured Projects row places it beside the view control.
    if trash_toggle(ui, tr(Text::ProjectsTrash), trash).clicked() {
        state.toggle_trash();
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
                let on = projects.view == view;
                if kit::icon_button(ui, id, glyph, tr(label), on).clicked() {
                    let mut next = *projects;
                    next.view = view;
                    state.update_projects(projects, next);
                }
            }
        },
    );

    // Trash rows follow their move order, so the project sort menu is hidden there.
    if !trash {
        sort_menu(ui, state, look, projects);
    }

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

/// The measured 64 px Trash control: trash glyph and its label. The selected
/// state marks that the grid below lists trashed projects, with the same quiet
/// surface and accent ink every other toolbar toggle uses.
fn trash_toggle(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let color = if selected {
        kit::accent(dark)
    } else {
        kit::text(dark)
    };
    let response = ui
        .scope_builder(
            egui::UiBuilder::new().scope_id(ui.make_persistent_id("projects.trash")),
            |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(TRASH_WIDTH, kit::size::CONTROL),
                    egui::Sense::click(),
                );
                if response.hovered()
                    || response.has_focus()
                    || response.is_pointer_button_down_on()
                    || selected
                {
                    ui.painter()
                        .rect_filled(rect, kit::radius::CONTROL, kit::hover(dark));
                }
                let text = ui.painter().layout_no_wrap(
                    label.to_owned(),
                    egui::FontId::proportional(kit::type_size::BODY),
                    color,
                );
                let glyph = kit::icon::DENSE;
                let gap = 6.0;
                let start = egui::pos2(
                    rect.center().x - (glyph + gap + text.size().x) / 2.0,
                    rect.center().y,
                );
                kit::tinted(Icon::Trash, glyph, color).paint_at(
                    ui,
                    egui::Rect::from_min_size(
                        egui::pos2(start.x, start.y - glyph / 2.0),
                        egui::Vec2::splat(glyph),
                    ),
                );
                ui.painter().galley(
                    egui::pos2(start.x + glyph + gap, start.y - text.size().y / 2.0),
                    text,
                    color,
                );
                if response.has_focus() {
                    ui.painter().rect_stroke(
                        rect,
                        kit::radius::CONTROL,
                        egui::Stroke::new(kit::space::HAIRLINE, kit::accent(dark)),
                        egui::StrokeKind::Inside,
                    );
                }
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label)
                });
                response
            },
        )
        .inner;
    set_author_id(&response, "projects.trash");
    response
}

fn sort_menu(ui: &mut egui::Ui, state: &mut State, look: Look, projects: &mut ProjectsPreferences) {
    let locale = look.locale;
    let sort = *projects;
    let order_label = if sort.ascending {
        Text::ProjectsSortAscending
    } else {
        Text::ProjectsSortDescending
    };
    let value = i18n::format(
        locale,
        Text::ProjectsSortValue,
        &[
            ("key", i18n::text(locale, sort_key_text(sort.sort))),
            ("order", i18n::text(locale, order_label)),
        ],
    );
    let label = i18n::format(locale, Text::ProjectsSort, &[("sort", &value)]);
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
                        let label = i18n::text(locale, sort_key_text(candidate));
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
                            state.update_projects(projects, next);
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

/// The listed catalog for the current mode.
fn rows(state: &State) -> &[ProjectEntry] {
    if state.trash_mode {
        &state.trash
    } else {
        &state.entries
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
            let Some(entry) = rows(state).get(*index).cloned() else {
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
                state.trash_mode,
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
        let Some(entry) = rows(state).get(*index).cloned() else {
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
            state.trash_mode,
            locale,
            &mut state.textures,
            actions,
        );
    }
}

/// One project: thumbnail, name and `duration | date`, plus its menu; a trashed
/// project shows its remaining days instead of its own metadata.
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    selected: bool,
    thumb: egui::Rect,
    texts: egui::Rect,
    menu_always: bool,
    trash: bool,
    locale: Locale,
    textures: &mut BTreeMap<ProjectId, egui::TextureHandle>,
    actions: &mut Vec<CardAction>,
) {
    let dark = ui.visuals().dark_mode;
    let prefix = format!("projects.card.{:016x}", entry.id.0);

    thumbnail(ui, entry, thumb, trash, locale, textures, actions);
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
    open_or_select(ui, &response, entry.id, trash, locale, actions);

    let meta = if trash {
        match entry.trash_days_left(now_unix_ms()) {
            Some(days) => i18n::format(
                locale,
                Text::ProjectsTrashDays,
                &[("days", &i18n::number(locale, days))],
            ),
            None => String::new(),
        }
    } else {
        format!(
            "{} | {}",
            display_time(entry.duration),
            i18n::date(locale, entry.created_unix_ms)
        )
    };
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
                card_menu_items(ui, &entry.id, trash, actions, locale);
            })
            .response
            .on_hover_text(i18n::text(locale, Text::ProjectsCardMenu));
        set_author_id(&response, &format!("{prefix}.menu"));
    });
}

fn open_or_select(
    ui: &egui::Ui,
    response: &egui::Response,
    id: ProjectId,
    trash: bool,
    locale: Locale,
    actions: &mut Vec<CardAction>,
) {
    if response.clicked() {
        actions.push(CardAction::Select {
            id,
            modifiers: ui.input(|input| input.modifiers),
        });
    }
    if response.double_clicked() && !trash {
        actions.push(CardAction::Open(id));
    }
    response.context_menu(|ui| card_menu_items(ui, &id, trash, actions, locale));
}

fn thumbnail(
    ui: &mut egui::Ui,
    entry: &ProjectEntry,
    rect: egui::Rect,
    trash: bool,
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
    open_or_select(ui, &response, entry.id, trash, locale, actions);
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
                    .alt_text(i18n::text(locale, Text::ProjectsThumbnailMissing)),
            );
            set_author_id(&placeholder, &format!("{prefix}.thumbnail.placeholder"));
        }
    }
}

fn card_menu_items(
    ui: &mut egui::Ui,
    id: &ProjectId,
    trash: bool,
    actions: &mut Vec<CardAction>,
    locale: Locale,
) {
    let prefix = format!("projects.card.{:016x}", id.0);
    let rows: &[(&str, Icon, Text, CardAction)] = if trash {
        &[
            (
                "restore",
                Icon::ArrowCounterClockwise,
                Text::ProjectsRestore,
                CardAction::Restore(*id),
            ),
            (
                "purge",
                Icon::Trash,
                Text::ProjectsPurge,
                CardAction::Purge(*id),
            ),
        ]
    } else {
        &[
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
        ]
    };
    for (suffix, icon, text, action) in rows {
        if menu_action(
            ui,
            &format!("{prefix}.{suffix}"),
            *icon,
            i18n::text(locale, *text),
        ) {
            actions.push(action.clone());
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
                    permanent: false,
                });
            }
            CardAction::Restore(id) => {
                state.selection.clear();
                state.selection.insert(id);
                state.anchor = Some(id);
                state.request_restore(vec![id]);
            }
            CardAction::Purge(id) => {
                state.delete = Some(DeleteDialog {
                    ids: vec![id],
                    error: None,
                    permanent: true,
                });
            }
            CardAction::Thumbnail(id) => state.request_thumbnail(id),
        }
    }
}

/// Trash age is presented in the creator's clock; the engine owns the policy.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}
