//! Renders shared controls without application state.
//!
//! Callers own labels, stable IDs, draft values and dispatch.

#![allow(dead_code)] // Shared contract for the parallel frame/panel consumer tickets.

use super as theme;
use super as icons;
use super::{Icon, icon, radius, size, space, type_size};
use eframe::egui::{self, Align2, InnerResponse, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::ops::RangeInclusive;

fn author(response: &Response, id: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
}

// Disabled custom paint uses the app disabled ink directly, without the
// toolkit also fading it. Native fields retain their own disabled presentation.
fn paint(ui: &Ui) -> egui::Painter {
    let mut painter = ui.painter().clone();
    if !ui.is_enabled() && ui.is_visible() {
        painter.set_opacity(1.0);
    }
    painter
}

fn paint_icon(ui: &mut Ui, glyph: Icon, color: egui::Color32, rect: Rect) {
    ui.scope(|ui| {
        let color = if !ui.is_enabled() {
            if ui.is_visible() {
                ui.set_opacity(1.0);
            }
            theme::disabled_icon(ui.visuals().dark_mode)
        } else {
            color
        };
        icons::tinted(glyph, icon::DENSE, color).paint_at(ui, rect);
    });
}

fn focus_ring(ui: &Ui, response: &Response) {
    if response.has_focus() {
        paint(ui).rect_stroke(
            response.rect,
            radius::CONTROL,
            Stroke::new(space::HAIRLINE, theme::accent(ui.visuals().dark_mode)),
            egui::StrokeKind::Inside,
        );
    }
}

fn ink(ui: &Ui, selected: bool) -> egui::Color32 {
    let dark = ui.visuals().dark_mode;
    if !ui.is_enabled() {
        theme::disabled(dark)
    } else if selected {
        theme::accent(dark)
    } else {
        theme::text(dark)
    }
}

/// A bare rail slider over `0..=1` inside `rect`: a 2 px rail, filled up to a
/// 6×12 thumb, no label or field. The caller supplies a stable automation ID.
pub(crate) fn rail_slider(ui: &mut Ui, id: &str, rect: Rect, value: &mut f32) -> Response {
    let mut response = ui.interact(rect, ui.make_persistent_id(id), Sense::click_and_drag());
    let half = size::ZOOM_THUMB_WIDTH / 2.0;
    let track = (rect.left() + half)..=(rect.right() - half);
    if (response.clicked() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let next = ((pointer.x - track.start()) / (track.end() - track.start())).clamp(0.0, 1.0);
        if (next - *value).abs() > f32::EPSILON {
            *value = next;
            response.mark_changed();
        }
    }
    let dark = ui.visuals().dark_mode;
    let rail = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), size::SLIDER_RAIL));
    let x = egui::lerp(track, (*value).clamp(0.0, 1.0));
    paint(ui).rect_filled(rail, radius::PANEL, theme::track_rest(dark));
    paint(ui).rect_filled(
        Rect::from_min_max(rail.min, egui::pos2(x, rail.bottom())),
        radius::PANEL,
        theme::bright(dark),
    );
    paint(ui).rect_filled(
        Rect::from_center_size(
            egui::pos2(x, rail.center().y),
            Vec2::new(size::ZOOM_THUMB_WIDTH, size::SLIDER_THUMB_HEIGHT),
        ),
        radius::CONTROL,
        theme::bright(dark),
    );
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), id));
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// A dense icon action with keyboard focus, tooltip and a stable automation ID.
pub(crate) fn icon_button(
    ui: &mut Ui,
    id: &str,
    glyph: Icon,
    label: &str,
    selected: bool,
) -> Response {
    let response = ui
        .push_id(id, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::splat(size::CONTROL), Sense::click());
            if response.hovered()
                || response.has_focus()
                || response.is_pointer_button_down_on()
                || selected
            {
                paint(ui).rect_filled(rect, radius::CONTROL, theme::hover(ui.visuals().dark_mode));
            }
            paint_icon(
                ui,
                glyph,
                ink(ui, selected),
                Rect::from_center_size(rect.center(), Vec2::splat(icon::DENSE)),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label)
            });
            response
        })
        .inner
        .on_hover_text(label);
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// A single-line primary action in an exact-width, 22 px window-bar box.
pub(crate) fn action_button(
    ui: &mut Ui,
    id: &str,
    glyph: Icon,
    label: &str,
    width: f32,
) -> Response {
    let response = ui
        .push_id(id, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(width, size::FIELD), Sense::click());
            let dark = ui.visuals().dark_mode;
            let color = theme::on_accent(dark);
            paint(ui).rect_filled(rect, radius::CONTROL, theme::accent(dark));
            let text = paint(ui).layout_no_wrap(
                label.to_owned(),
                egui::FontId::proportional(type_size::BODY),
                color,
            );
            let total = icon::DENSE + space::XS + text.size().x;
            let start = egui::pos2(rect.center().x - total / 2.0, rect.center().y);
            paint_icon(
                ui,
                glyph,
                color,
                Rect::from_min_size(
                    start - Vec2::new(0.0, icon::DENSE / 2.0),
                    Vec2::splat(icon::DENSE),
                ),
            );
            paint(ui).galley(
                start + Vec2::new(icon::DENSE + space::XS, -text.size().y / 2.0),
                text,
                color,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
            });
            response
        })
        .inner
        .on_hover_text(label);
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// Compact transport action with a smaller glyph or a circular contrasting face.
pub(crate) fn transport_button(
    ui: &mut Ui,
    id: &str,
    glyph: Icon,
    label: &str,
    round: bool,
) -> Response {
    let response = ui
        .push_id(id, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::splat(size::CONTROL), Sense::click());
            let color = ink(ui, false);
            if round {
                paint(ui).circle_filled(rect.center(), icon::DENSE / 2.0, color);
                icons::tinted(glyph, icon::PLAY, theme::panel(ui.visuals().dark_mode)).paint_at(
                    ui,
                    Rect::from_center_size(rect.center(), Vec2::splat(icon::PLAY)),
                );
            } else {
                if response.hovered() || response.has_focus() {
                    paint(ui).rect_filled(
                        rect,
                        radius::CONTROL,
                        theme::hover(ui.visuals().dark_mode),
                    );
                }
                icons::tinted(glyph, icon::PLAY, color).paint_at(
                    ui,
                    Rect::from_center_size(rect.center(), Vec2::splat(icon::PLAY)),
                );
            }
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
            });
            response
        })
        .inner
        .on_hover_text(label);
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// Outlined value badge inside a comfortable pointer target.
pub(crate) fn badge_button(ui: &mut Ui, id: &str, label: &str) -> Response {
    let response = ui
        .push_id(id, |ui| {
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(size::PLAYER_RATIO_WIDTH, size::FIELD),
                Sense::click(),
            );
            let color = ink(ui, false);
            let galley = paint(ui).layout_no_wrap(
                label.to_owned(),
                egui::FontId::proportional(type_size::BADGE),
                color,
            );
            let badge =
                Rect::from_center_size(rect.center(), galley.size() + Vec2::new(space::XS, 0.0));
            paint(ui).rect_stroke(
                badge,
                radius::PANEL,
                Stroke::new(space::HAIRLINE, color),
                egui::StrokeKind::Inside,
            );
            paint(ui).galley(rect.center() - galley.size() / 2.0, galley, color);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
            });
            response
        })
        .inner
        .on_hover_text(label);
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// Feature tab: icon above label, with the measured strip height and inset.
fn icon_tab(ui: &mut Ui, id: &str, glyph: Icon, label: &str, selected: bool) -> Response {
    let color = ink(ui, selected);
    let font = theme::label_font();
    let text = paint(ui).layout_no_wrap(label.to_owned(), font, color);
    let width = (text.size().x + space::LG * 2.0).max(size::TAB_MIN);
    let response = ui
        .push_id(id, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(width, size::TAB_STRIP), Sense::click());
            paint(ui).rect_filled(rect, radius::PANEL, theme::strip(ui.visuals().dark_mode));
            if response.hovered() || response.has_focus() {
                paint(ui).rect_filled(rect, radius::CONTROL, theme::hover(ui.visuals().dark_mode));
            }
            let height = icon::DENSE + space::ICON_LABEL + text.size().y;
            let top = rect.center().y - height / 2.0;
            paint_icon(
                ui,
                glyph,
                color,
                Rect::from_center_size(
                    egui::pos2(rect.center().x, top + icon::DENSE / 2.0),
                    Vec2::splat(icon::DENSE),
                ),
            );
            paint(ui).galley(
                egui::pos2(
                    rect.center().x - text.size().x / 2.0,
                    top + icon::DENSE + space::ICON_LABEL,
                ),
                text,
                color,
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), selected, label)
            });
            response
        })
        .inner;
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// A content-width segmented track. Each cell keeps its own automation ID.
pub(crate) fn tab_strip(
    ui: &mut Ui,
    id: &str,
    tabs: &[(&str, Option<Icon>)],
    selected: &mut usize,
    cell_width: f32,
) -> Vec<Response> {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        tabs.iter()
            .enumerate()
            .map(|(index, (label, glyph))| {
                let cell_id = format!("{id}.{index}");
                if let Some(glyph) = glyph {
                    let response = icon_tab(ui, &cell_id, *glyph, label, index == *selected);
                    if response.clicked() {
                        *selected = index;
                    }
                    return response;
                }
                let response = ui
                    .push_id(&cell_id, |ui| {
                        let active = index == *selected;
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(cell_width, size::CONTROL),
                            Sense::click(),
                        );
                        let dark = ui.visuals().dark_mode;
                        paint(ui).rect_filled(
                            rect,
                            radius::CONTROL,
                            if active || response.hovered() {
                                theme::hover(dark)
                            } else {
                                theme::field(dark)
                            },
                        );
                        paint(ui).text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            *label,
                            theme::label_font(),
                            ink(ui, false),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::RadioButton,
                                ui.is_enabled(),
                                active,
                                *label,
                            )
                        });
                        response
                    })
                    .inner;
                author(&response, &cell_id);
                focus_ring(ui, &response);
                if response.clicked() {
                    *selected = index;
                }
                response
            })
            .collect()
    })
    .inner
}

/// Header responses stay separate so a reset or keyframe click cannot toggle it.
/// Controls the caller did not ask for are absent, never drawn inert.
pub(crate) struct SectionResponse {
    pub toggle: Response,
    pub reset: Option<Response>,
    pub keyframe: Option<Response>,
}

pub(crate) fn section_header(
    ui: &mut Ui,
    id: &str,
    label: &str,
    open: &mut bool,
    reset_label: &str,
    keyframe_label: &str,
) -> SectionResponse {
    section_header_with(ui, id, label, open, Some(reset_label), Some(keyframe_label))
}

/// Measured header: label with a caret, then reset and the keyframe group at
/// the right edge. A hairline closes the section.
pub(crate) fn section_header_with(
    ui: &mut Ui,
    id: &str,
    label: &str,
    open: &mut bool,
    reset_label: Option<&str>,
    keyframe_label: Option<&str>,
) -> SectionResponse {
    let dark = ui.visuals().dark_mode;
    let row = ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), size::SECTION),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let reserved = reset_label.map_or(0.0, |_| size::CONTROL + space::MD)
                + keyframe_label.map_or(0.0, |_| size::KEYFRAME + space::MD);
            let width = (ui.available_width() - reserved).max(0.0);
            let (rect, toggle) =
                ui.allocate_exact_size(Vec2::new(width, size::FIELD), Sense::click());
            let galley =
                paint(ui).layout_no_wrap(label.to_owned(), theme::label_font(), ink(ui, false));
            let label_width = galley.size().x;
            paint(ui).galley(
                egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                galley,
                ink(ui, false),
            );
            let center = egui::pos2(rect.left() + label_width + space::MD, rect.center().y);
            let direction = if *open { -1.0 } else { 1.0 };
            let points = vec![
                center + egui::vec2(-space::XS, -direction * space::XXS),
                center + egui::vec2(space::XS, -direction * space::XXS),
                center + egui::vec2(0.0, direction * space::XXS),
            ];
            paint(ui).add(egui::Shape::convex_polygon(
                points,
                theme::secondary(dark),
                Stroke::NONE,
            ));
            toggle.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), *open, label)
            });
            author(&toggle, id);
            focus_ring(ui, &toggle);
            if toggle.clicked() {
                *open = !*open;
            }
            let reset = reset_label.map(|reset_label| {
                icon_button(
                    ui,
                    &format!("{id}.reset"),
                    Icon::ArrowCounterClockwise,
                    reset_label,
                    false,
                )
            });
            let keyframe = keyframe_label.map(|keyframe_label| {
                let keyframe_id = format!("{id}.keyframe");
                let response = ui
                    .push_id(&keyframe_id, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(size::KEYFRAME, size::CONTROL),
                            Sense::click(),
                        );
                        let center = rect.center();
                        let half = size::KEYFRAME_DIAMOND / 2.0;
                        let points = vec![
                            center - egui::vec2(half, 0.0),
                            center - egui::vec2(0.0, half),
                            center + egui::vec2(half, 0.0),
                            center + egui::vec2(0.0, half),
                        ];
                        paint(ui).add(egui::Shape::closed_line(
                            points,
                            Stroke::new(space::HAIRLINE, ink(ui, false)),
                        ));
                        for direction in [-1.0, 1.0] {
                            let tip = center
                                + egui::vec2(direction * (size::KEYFRAME / 2.0 - space::XXS), 0.0);
                            let points = vec![
                                tip + egui::vec2(-direction * space::XXS, -space::XS),
                                tip,
                                tip + egui::vec2(-direction * space::XXS, space::XS),
                            ];
                            paint(ui).add(egui::Shape::line(
                                points,
                                Stroke::new(space::HAIRLINE, theme::disabled(dark)),
                            ));
                        }
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                ui.is_enabled(),
                                keyframe_label,
                            )
                        });
                        response
                    })
                    .inner
                    .on_hover_text(keyframe_label);
                author(&response, &keyframe_id);
                focus_ring(ui, &response);
                response
            });
            SectionResponse {
                toggle,
                reset,
                keyframe,
            }
        },
    );
    paint(ui).hline(
        row.response.rect.x_range(),
        row.response.rect.top(),
        Stroke::new(space::HAIRLINE, theme::separator(dark)),
    );
    row.inner
}

/// The top tab row of a panel: text tabs on the strip, accent for the active
/// one, measured gap and inset. Each cell keeps its own automation ID.
pub(crate) fn text_tabs(
    ui: &mut Ui,
    ids: &[String],
    labels: &[&str],
    selected: usize,
) -> Vec<Response> {
    let dark = ui.visuals().dark_mode;
    let full = Rect::from_min_size(
        ui.available_rect_before_wrap().min,
        Vec2::new(ui.available_width(), size::TAB_STRIP),
    );
    paint(ui).rect_filled(full, radius::PANEL, theme::strip(dark));
    let mut x = full.left() + space::ROW_INSET + space::XXS / 2.0;
    let mut out = Vec::new();
    for (index, (id, label)) in ids.iter().zip(labels).enumerate() {
        let color = ink(ui, index == selected);
        let galley = paint(ui).layout_no_wrap((*label).to_owned(), theme::label_font(), color);
        let rect = Rect::from_min_size(
            egui::pos2(x, full.top()),
            Vec2::new(galley.size().x, size::TAB_STRIP),
        );
        let response = ui
            .push_id(id, |ui| {
                ui.interact(rect, ui.scope_id().with("tab"), Sense::click())
            })
            .inner;
        paint(ui).galley(
            egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), index == selected, *label)
        });
        author(&response, id);
        focus_ring(ui, &response);
        x = rect.right() + space::TAB_GAP;
        out.push(response);
    }
    ui.allocate_rect(full, Sense::hover());
    out
}

/// A full-width segmented bar: equal cells on a field-coloured track, the
/// active cell raised. Each cell keeps its own automation ID.
pub(crate) fn segmented(
    ui: &mut Ui,
    ids: &[String],
    labels: &[&str],
    selected: usize,
) -> Vec<Response> {
    let dark = ui.visuals().dark_mode;
    let full = Rect::from_min_size(
        ui.available_rect_before_wrap().min,
        Vec2::new(ui.available_width(), size::CONTROL),
    );
    paint(ui).rect_filled(full, radius::CONTROL, theme::field(dark));
    let width = full.width() / ids.len().max(1) as f32;
    let mut out = Vec::new();
    for (index, (id, label)) in ids.iter().zip(labels).enumerate() {
        let rect = Rect::from_min_size(
            egui::pos2(full.left() + width * index as f32, full.top()),
            Vec2::new(width, size::CONTROL),
        );
        let response = ui
            .push_id(id, |ui| {
                ui.interact(rect, ui.scope_id().with("segment"), Sense::click())
            })
            .inner;
        let active = index == selected;
        if active || response.hovered() {
            paint(ui).rect_filled(rect, radius::CONTROL, theme::hover(dark));
        }
        paint(ui).text(
            rect.center(),
            Align2::CENTER_CENTER,
            *label,
            theme::label_font(),
            ink(ui, false),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), active, *label)
        });
        author(&response, id);
        focus_ring(ui, &response);
        out.push(response);
    }
    ui.allocate_rect(full, Sense::hover());
    out
}

/// A 36x16 switch. The caller owns the value; the response carries the click.
pub(crate) fn toggle(ui: &mut Ui, id: &str, value: &mut bool, label: &str) -> Response {
    let dark = ui.visuals().dark_mode;
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(size::TOGGLE_WIDTH, size::TOGGLE_HEIGHT),
        Sense::click(),
    );
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let track = if *value {
        theme::accent(dark)
    } else {
        theme::track_rest(dark)
    };
    paint(ui).rect_filled(rect, size::TOGGLE_HEIGHT / 2.0, track);
    let knob = size::TOGGLE_HEIGHT - space::XS;
    let x = if *value {
        rect.right() - space::XXS - knob / 2.0
    } else {
        rect.left() + space::XXS + knob / 2.0
    };
    let knob_color = if *value {
        theme::on_accent(dark)
    } else {
        theme::bright(dark)
    };
    paint(ui).circle_filled(egui::pos2(x, rect.center().y), knob / 2.0, knob_color);
    let on = *value;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, label)
    });
    author(&response, id);
    focus_ring(ui, &response);
    response.on_hover_text(label)
}

/// A 16x22 up/down stepper beside a value field: two chevrons, one hit area each.
pub(crate) fn stepper(ui: &mut Ui, id: &str, label: &str) -> (Response, Response) {
    let dark = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(size::STEPPER, size::FIELD), Sense::hover());
    paint(ui).rect_filled(rect, radius::FIELD, theme::hover(dark));
    let cell_response = |ui: &mut Ui, up: bool| -> Response {
        let cell = Rect::from_min_size(
            egui::pos2(
                rect.left(),
                rect.top() + if up { 0.0 } else { size::FIELD / 2.0 },
            ),
            Vec2::new(size::STEPPER, size::FIELD / 2.0),
        );
        let step_id = format!("{id}.{}", if up { "increase" } else { "decrease" });
        let response = ui
            .push_id(&step_id, |ui| {
                ui.interact(cell, ui.scope_id().with("step"), Sense::click())
            })
            .inner;
        let center = egui::pos2(
            cell.center().x,
            cell.center().y + if up { 1.0 } else { -1.0 },
        );
        let direction = if up { -1.0 } else { 1.0 };
        paint(ui).add(egui::Shape::line(
            vec![
                center + egui::vec2(-3.0, -direction * 1.5),
                center + egui::vec2(0.0, direction * 1.5),
                center + egui::vec2(3.0, -direction * 1.5),
            ],
            Stroke::new(1.5, ink(ui, false)),
        ));
        focus_ring(ui, &response);
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
        author(&response, &step_id);
        response
    };
    let increase = cell_response(ui, true);
    let decrease = cell_response(ui, false);
    (increase, decrease)
}

/// A measured numeric text field: centred draft text, optional dim prefix
/// (an axis letter). The caller owns the draft and commit.
pub(crate) fn value_field(
    ui: &mut Ui,
    id: &str,
    text: &mut String,
    width: f32,
    prefix: Option<&str>,
    label: &str,
) -> Response {
    let dark = ui.visuals().dark_mode;
    let response = ui
        .push_id(id, |ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                state.corner_radius = egui::CornerRadius::same(radius::FIELD);
            }
            ui.add_sized(
                [width, size::FIELD],
                egui::TextEdit::singleline(text)
                    .char_limit(32)
                    .align(egui::Align2::CENTER_CENTER)
                    .background_color(theme::field(dark))
                    .margin(egui::Margin::symmetric(4, 0)),
            )
        })
        .inner;
    author(&response, id);
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_label(label));
    if let Some(prefix) = prefix {
        paint(ui).text(
            egui::pos2(response.rect.left() + space::MD, response.rect.center().y),
            Align2::LEFT_CENTER,
            prefix,
            theme::label_font(),
            theme::secondary(dark),
        );
    }
    response
}

/// A rail with a thumb and no value. Dragging reports a new value through the
/// response; the caller owns gesture start, preview and commit.
pub(crate) fn rail_slider(
    ui: &mut Ui,
    id: &str,
    value: f64,
    range: RangeInclusive<f64>,
    width: f32,
) -> (Response, f64) {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, size::SLIDER_THUMB_HEIGHT + space::XS),
        Sense::click_and_drag(),
    );
    let start = *range.start();
    let span = *range.end() - start;
    let half = size::SLIDER_THUMB_WIDTH / 2.0;
    let mut value = value;
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.drag_started() || response.clicked())
        && span > 0.0
    {
        let fraction = f64::from(
            ((pointer.x - rect.left() - half) / (rect.width() - 2.0 * half)).clamp(0.0, 1.0),
        );
        value = start + fraction * span;
    }
    let fraction = if span > 0.0 {
        ((value - start) / span).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let x = egui::lerp((rect.left() + half)..=(rect.right() - half), fraction);
    let rail = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), size::SLIDER_RAIL));
    let fill = if ui.is_enabled() {
        theme::bright(dark)
    } else {
        theme::disabled(dark)
    };
    paint(ui).rect_filled(rail, radius::PANEL, theme::track_rest(dark));
    paint(ui).rect_filled(
        Rect::from_min_max(rail.min, egui::pos2(x, rail.bottom())),
        radius::PANEL,
        fill,
    );
    paint(ui).rect_filled(
        Rect::from_center_size(
            egui::pos2(x, rail.center().y),
            Vec2::new(size::SLIDER_THUMB_WIDTH, size::SLIDER_THUMB_HEIGHT),
        ),
        radius::CONTROL,
        fill,
    );
    author(&response, id);
    focus_ring(ui, &response);
    (response, value)
}

/// Label-left/control-right row. The caller supplies a control with its own ID.
pub(crate) fn property_row<R>(
    ui: &mut Ui,
    label: &str,
    body: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), size::PROPERTY_ROW),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
            ui.label(egui::RichText::new(label).size(type_size::BODY));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), body)
                .inner
        },
    )
}

pub(crate) struct SliderResponse {
    pub slider: Response,
    pub field: Response,
    pub decrease: Response,
    pub increase: Response,
}

/// Labels, units and bounds come from the caller's property contract.
pub(crate) struct NumberSpec<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub unit: &'a str,
    pub range: RangeInclusive<f64>,
    pub step: f64,
}

/// Bounded draft editing only: caller chooses when to preview, commit or cancel.
pub(crate) fn slider_stepper(ui: &mut Ui, value: &mut f64, spec: NumberSpec<'_>) -> SliderResponse {
    let NumberSpec {
        id,
        label,
        unit,
        range,
        step,
    } = spec;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(egui::RichText::new(label).size(type_size::BODY));
        ui.horizontal(|ui| {
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
            ui.spacing_mut().slider_width = size::SLIDER;
            ui.spacing_mut().item_spacing.x = space::XXS;
            ui.spacing_mut().slider_rail_height = size::SLIDER_RAIL;
            ui.spacing_mut().interact_size.y = size::SLIDER_THUMB_HEIGHT;
            let slider = ui
                .push_id(format!("{id}.slider"), |ui| {
                    ui.add(
                        egui::Slider::new(value, range.clone())
                            .step_by(step)
                            .show_value(false)
                            .handle_shape(egui::style::HandleShape::Rect {
                                aspect_ratio: size::SLIDER_THUMB_WIDTH / size::SLIDER_THUMB_HEIGHT,
                            }),
                    )
                })
                .inner;
            author(&slider, &format!("{id}.slider"));
            slider.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Slider, ui.is_enabled(), label)
            });
            let dark = ui.visuals().dark_mode;
            paint(ui).rect_filled(slider.rect, radius::PANEL, theme::panel(dark));
            let rail = Rect::from_center_size(
                slider.rect.center(),
                Vec2::new(size::SLIDER, size::SLIDER_RAIL),
            );
            let start = *range.start();
            let span = *range.end() - start;
            let fraction = if span > 0.0 {
                ((*value - start) / span).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let x = egui::lerp(
                (rail.left() + size::SLIDER_THUMB_WIDTH / 2.0)
                    ..=(rail.right() - size::SLIDER_THUMB_WIDTH / 2.0),
                fraction,
            );
            paint(ui).rect_filled(rail, radius::PANEL, theme::track_rest(dark));
            let fill = if ui.is_enabled() {
                theme::bright(dark)
            } else {
                theme::disabled(dark)
            };
            paint(ui).rect_filled(
                Rect::from_min_max(rail.min, egui::pos2(x, rail.bottom())),
                radius::PANEL,
                fill,
            );
            paint(ui).rect_filled(
                Rect::from_center_size(
                    egui::pos2(x, rail.center().y),
                    Vec2::new(size::SLIDER_THUMB_WIDTH, size::SLIDER_THUMB_HEIGHT),
                ),
                radius::CONTROL,
                fill,
            );
            if slider.has_focus() {
                paint(ui).rect_stroke(
                    slider.rect,
                    radius::CONTROL,
                    Stroke::new(space::HAIRLINE, theme::accent(ui.visuals().dark_mode)),
                    egui::StrokeKind::Inside,
                );
            }
            ui.add_space(space::LG + space::ICON_LABEL);
            let field = ui
                .push_id(id, |ui| {
                    ui.add_sized(
                        [size::VALUE, size::FIELD],
                        egui::DragValue::new(value)
                            .range(range.clone())
                            .speed(step)
                            .suffix(unit),
                    )
                })
                .inner;
            author(&field, id);
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::SpinButton, ui.is_enabled(), label)
            });
            let buttons = ui
                .push_id(format!("{id}.stepper"), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let stepper_rect = Rect::from_min_size(
                            ui.next_widget_position(),
                            Vec2::new(size::STEPPER, size::FIELD),
                        );
                        paint(ui).rect_filled(
                            stepper_rect,
                            radius::CONTROL,
                            theme::hover(ui.visuals().dark_mode),
                        );
                        let mut arrow = |up: bool| {
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(size::STEPPER, size::FIELD / 2.0),
                                Sense::click(),
                            );
                            let center = rect.center();
                            let direction = if up { -1.0 } else { 1.0 };
                            let points = vec![
                                center + egui::vec2(-space::XXS, -direction),
                                center + egui::vec2(space::XXS, -direction),
                                center + egui::vec2(0.0, direction * space::XXS),
                            ];
                            paint(ui).add(egui::Shape::convex_polygon(
                                points,
                                ink(ui, false),
                                Stroke::NONE,
                            ));
                            focus_ring(ui, &response);
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    ui.is_enabled(),
                                    label,
                                )
                            });
                            response
                        };
                        let increase = arrow(true);
                        let decrease = arrow(false);
                        (decrease, increase)
                    })
                    .inner
                })
                .inner;
            author(&buttons.0, &format!("{id}.decrease"));
            author(&buttons.1, &format!("{id}.increase"));
            if buttons.0.clicked() {
                *value = (*value - step).clamp(*range.start(), *range.end());
            }
            if buttons.1.clicked() {
                *value = (*value + step).clamp(*range.start(), *range.end());
            }
            SliderResponse {
                slider,
                field,
                decrease: buttons.0,
                increase: buttons.1,
            }
        })
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests;
