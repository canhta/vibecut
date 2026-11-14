//! Renders shared controls without application state.
//!
//! Callers own labels, stable IDs, draft values and dispatch.

use super as theme;
use super as icons;
use super::{Icon, icon, radius, size, space, type_size};
use eframe::egui::{self, Align2, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::ops::RangeInclusive;

pub(super) fn author(response: &Response, id: &str) {
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
pub(crate) fn zoom_rail(ui: &mut Ui, id: &str, rect: Rect, value: &mut f32) -> Response {
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

/// A rounded dashed border for upload surfaces, using the shared focus ink.
pub(crate) fn dashed_outline(ui: &Ui, rect: Rect, radius: u8, focused: bool) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let rect = rect.shrink(space::HAIRLINE / 2.0);
    let radius = (f32::from(radius) - space::HAIRLINE / 2.0)
        .max(0.0)
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    let mut points = Vec::with_capacity(21);
    for (center, start) in [
        (rect.left_top() + Vec2::splat(radius), PI),
        (rect.right_top() + Vec2::new(-radius, radius), -FRAC_PI_2),
        (rect.right_bottom() - Vec2::splat(radius), 0.0),
        (rect.left_bottom() + Vec2::new(radius, -radius), FRAC_PI_2),
    ] {
        for step in 0..=4 {
            let angle = start + FRAC_PI_2 * step as f32 / 4.0;
            points.push(center + Vec2::new(angle.cos(), angle.sin()) * radius);
        }
    }
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    let dark = ui.visuals().dark_mode;
    paint(ui).extend(egui::Shape::dashed_line(
        &points,
        Stroke::new(
            space::HAIRLINE,
            if focused {
                theme::accent(dark)
            } else {
                theme::border(dark)
            },
        ),
        space::XXS,
        space::XXS,
    ));
}

/// A dense icon action with keyboard focus, tooltip and a stable automation ID.
///
/// The button allocates in one explicit scope so a conditional label drawn
/// earlier in the same `ui` cannot shift its widget id: egui drops a press and
/// release pair that resolve to different widgets.
pub(crate) fn icon_button(
    ui: &mut Ui,
    id: &str,
    glyph: Icon,
    label: &str,
    selected: bool,
) -> Response {
    let scope = ui.make_persistent_id(id);
    let response = ui
        .scope_builder(egui::UiBuilder::new().scope_id(scope), |ui| {
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

/// What a row-end keyframe cluster shows: the three tooltips and the state of
/// each cell. The caller owns the keys; the widget only paints and reports.
pub(crate) struct KeyframeControl<'a> {
    pub previous_label: &'a str,
    pub toggle_label: &'a str,
    pub next_label: &'a str,
    /// A key exists at the playhead: the diamond is filled.
    pub keyed: bool,
    /// The playhead is inside the item, so a key can be added or removed.
    pub can_toggle: bool,
    pub has_previous: bool,
    pub has_next: bool,
}

/// One hit cell of the keyframe cluster: hover fill, automation ID and focus ring.
fn keyframe_cell(ui: &mut Ui, id: &str, area: Rect, enabled: bool) -> Response {
    let cell = ui.make_persistent_id(id);
    let response = if enabled {
        ui.interact(area, cell, Sense::click())
    } else {
        // A press on a disabled cell must not fall through to the toggle a few pixels
        // away (egui hands a click on a non-clicking widget to the closest clickable one).
        // This backstop takes the press; it is not focusable, so it has no AccessKit node.
        ui.interact(area, ui.make_persistent_id((id, "backstop")), Sense::CLICK);
        // The cell itself is a disabled widget, so its response and node read disabled.
        ui.add_enabled_ui(false, |ui| ui.interact(area, cell, Sense::hover()))
            .inner
    };
    if enabled
        && (response.hovered() || response.has_focus() || response.is_pointer_button_down_on())
    {
        paint(ui).rect_filled(area, radius::CONTROL, theme::hover(ui.visuals().dark_mode));
    }
    author(&response, id);
    focus_ring(ui, &response);
    response
}

/// The measured row-end keyframe cluster `< diamond >` inside `rect`
/// (`KEYFRAME_CLUSTER` wide): previous key, toggle at the playhead, next key.
/// Each cell is its own response with the automation IDs `{id}.previous`, `{id}`
/// and `{id}.next`; an arrow with no key on its side is disabled. The returned
/// responses are previous, toggle and next.
pub(crate) fn keyframe_cluster(
    ui: &mut Ui,
    id: &str,
    rect: Rect,
    control: &KeyframeControl<'_>,
) -> [Response; 3] {
    let dark = ui.visuals().dark_mode;
    let cell_area = |left: f32, width: f32| {
        Rect::from_min_size(
            egui::pos2(left, rect.top()),
            Vec2::new(width, rect.height()),
        )
    };
    let previous_area = cell_area(rect.left(), size::KEYFRAME_ARROW_CELL);
    let toggle_area = cell_area(previous_area.right(), size::KEYFRAME_TOGGLE_CELL);
    let next_area = cell_area(toggle_area.right(), size::KEYFRAME_ARROW_CELL);
    let previous = keyframe_cell(
        ui,
        &format!("{id}.previous"),
        previous_area,
        control.has_previous,
    );
    let toggle = keyframe_cell(ui, id, toggle_area, control.can_toggle);
    let next = keyframe_cell(ui, &format!("{id}.next"), next_area, control.has_next);
    let ink = |enabled: bool| {
        if enabled {
            theme::text(dark)
        } else {
            theme::disabled(dark)
        }
    };
    for (area, direction, enabled) in [
        (previous_area, -1.0_f32, control.has_previous),
        (next_area, 1.0, control.has_next),
    ] {
        let center = area.center();
        let half_width = size::KEYFRAME_ARROW_WIDTH / 2.0;
        let half_height = size::KEYFRAME_ARROW_HEIGHT / 2.0;
        paint(ui).add(egui::Shape::line(
            vec![
                center + egui::vec2(-direction * half_width, -half_height),
                center + egui::vec2(direction * half_width, 0.0),
                center + egui::vec2(-direction * half_width, half_height),
            ],
            Stroke::new(space::HAIRLINE, ink(enabled)),
        ));
    }
    let center = toggle_area.center();
    let half = size::KEYFRAME_GLYPH / 2.0;
    let points = vec![
        center - egui::vec2(half, 0.0),
        center - egui::vec2(0.0, half),
        center + egui::vec2(half, 0.0),
        center + egui::vec2(0.0, half),
    ];
    if control.keyed && control.can_toggle {
        paint(ui).add(egui::Shape::convex_polygon(
            points,
            theme::accent(dark),
            Stroke::NONE,
        ));
    } else {
        paint(ui).add(egui::Shape::closed_line(
            points,
            Stroke::new(space::HAIRLINE, ink(control.can_toggle)),
        ));
    }
    for (response, enabled, label, selected) in [
        (
            &previous,
            control.has_previous,
            control.previous_label,
            false,
        ),
        (
            &toggle,
            control.can_toggle,
            control.toggle_label,
            control.keyed,
        ),
        (&next, control.has_next, control.next_label, false),
    ] {
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, label)
        });
    }
    [
        previous.on_hover_text(control.previous_label),
        toggle.on_hover_text(control.toggle_label),
        next.on_hover_text(control.next_label),
    ]
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

/// Feature tab: icon above a 12 px label, the measured strip height and inset. The width
/// follows the label (12 px of padding, 48 px minimum), so neighbours never overlap.
pub(crate) fn icon_tab(
    ui: &mut Ui,
    id: &str,
    glyph: Icon,
    label: &str,
    selected: bool,
) -> Response {
    let color = ink(ui, selected);
    let mut font = theme::label_font();
    font.size = theme::type_size::BODY;
    let text = paint(ui).layout_no_wrap(label.to_owned(), font, color);
    let width = (text.size().x + space::SM * 2.0).max(size::TAB_MIN);
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
                    egui::pos2(rect.center().x, top + icon::DENSE / 2.0 + space::XS),
                    Vec2::splat(icon::DENSE),
                ),
            );
            paint(ui).galley(
                egui::pos2(
                    rect.center().x - text.size().x / 2.0,
                    top + icon::DENSE + space::ICON_LABEL - space::HAIRLINE,
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

/// Header responses stay separate so a reset or keyframe click cannot toggle it.
/// Controls the caller did not ask for are absent, never drawn inert.
pub(crate) struct SectionResponse {
    pub toggle: Response,
    pub reset: Option<Response>,
    /// The enable checkbox before the label, when the section has one.
    pub enable: Option<Response>,
}

/// Measured header: an optional enable checkbox, the label with a caret, then
/// reset and the keyframe group at the right edge. `enable` is the checkbox's
/// label and checked state; a click on it is the caller's to apply. A hairline
/// closes the section.
pub(crate) fn section_header_with(
    ui: &mut Ui,
    id: &str,
    label: &str,
    open: &mut bool,
    reset_label: Option<&str>,
    keyframe_label: Option<&str>,
    enable: Option<(&str, bool)>,
) -> SectionResponse {
    let dark = ui.visuals().dark_mode;
    let row = ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), size::SECTION),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let enable = enable.map(|(enable_label, checked)| {
                let enable_id = format!("{id}.enable");
                let (rect, response) = ui
                    .allocate_exact_size(Vec2::new(size::CHECKBOX, size::CHECKBOX), Sense::click());
                paint_checkbox(ui, rect, checked, dark);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::CheckBox,
                        ui.is_enabled(),
                        checked,
                        enable_label,
                    )
                });
                author(&response, &enable_id);
                focus_ring(ui, &response);
                response.on_hover_text(enable_label)
            });
            let enable_width = enable.as_ref().map_or(0.0, |_| size::CHECKBOX + space::MD);
            let reserved = enable_width
                + reset_label.map_or(0.0, |_| size::CONTROL + space::MD)
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
            if let Some(keyframe_label) = keyframe_label {
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
            }
            SectionResponse {
                toggle,
                reset,
                enable,
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
    let galleys: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            paint(ui).layout_no_wrap(
                (*label).to_owned(),
                theme::label_font(),
                ink(ui, index == selected),
            )
        })
        .collect();
    // A long locale shrinks the gaps so the last tab never leaves the strip.
    let text: f32 = galleys.iter().map(|galley| galley.size().x).sum();
    let room = full.right() - x - space::ROW_INSET - text;
    let gap = if galleys.len() > 1 {
        (room / (galleys.len() - 1) as f32).clamp(space::XS, space::TAB_GAP)
    } else {
        space::TAB_GAP
    };
    for (index, ((id, label), galley)) in ids.iter().zip(labels).zip(galleys).enumerate() {
        let color = ink(ui, index == selected);
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
        x = rect.right() + gap;
        out.push(response);
    }
    ui.allocate_rect(full, Sense::hover());
    out
}

/// The fill of one segment cell: the measured selected fill, the hover token while only
/// hovered, or none so the field-coloured track shows through.
fn segment_fill(dark: bool, active: bool, hovered: bool) -> Option<egui::Color32> {
    if active {
        Some(theme::segment_active(dark))
    } else if hovered {
        Some(theme::hover(dark))
    } else {
        None
    }
}

/// A full-width segmented bar: equal cells on a field-coloured track, the
/// selected cell filled and outlined as measured. Each cell keeps its own automation ID.
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
        if let Some(fill) = segment_fill(dark, active, response.hovered()) {
            paint(ui).rect_filled(rect, radius::CONTROL, fill);
        }
        if active && let Some(outline) = theme::segment_outline(dark) {
            paint(ui).rect_stroke(
                rect,
                radius::CONTROL,
                Stroke::new(space::HAIRLINE, outline),
                egui::StrokeKind::Inside,
            );
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

/// The 12 px box: an accent fill with a tick when checked, a hairline outline when not.
fn paint_checkbox(ui: &Ui, rect: Rect, checked: bool, dark: bool) {
    if checked {
        paint(ui).rect_filled(rect, radius::CONTROL, theme::accent(dark));
        paint(ui).add(egui::Shape::line(
            vec![
                rect.min + egui::vec2(3.0, 6.0),
                rect.min + egui::vec2(5.0, 8.5),
                rect.min + egui::vec2(9.0, 3.5),
            ],
            Stroke::new(1.5, theme::on_accent(dark)),
        ));
    } else {
        paint(ui).rect_stroke(
            rect,
            radius::CONTROL,
            Stroke::new(space::HAIRLINE, theme::border(dark)),
            egui::StrokeKind::Inside,
        );
    }
}

/// A 12 px checkbox with its label to its right; the whole row is the hit area. The caller
/// owns the value; the response carries the click.
pub(crate) fn checkbox(ui: &mut Ui, id: &str, value: &mut bool, label: &str) -> Response {
    let dark = ui.visuals().dark_mode;
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), theme::label_font(), theme::text(dark));
    let width = size::CHECKBOX + space::MD + galley.size().x;
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, size::FIELD), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let boxed = Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - size::CHECKBOX / 2.0),
        Vec2::splat(size::CHECKBOX),
    );
    paint_checkbox(ui, boxed, *value, dark);
    let text_pos = egui::pos2(
        boxed.right() + space::MD,
        rect.center().y - galley.size().y / 2.0,
    );
    paint(ui).galley(text_pos, galley, theme::text(dark));
    let on = *value;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, label)
    });
    author(&response, id);
    focus_ring(ui, &response);
    response.on_hover_text(label)
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
        theme::toggle_knob(dark)
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
        // A direct interaction, not a `push_id` scope: an empty scope still advances the
        // cursor by the item spacing, which would leave a gap after the stepper.
        let response = ui.interact(cell, ui.scope_id().with(&step_id), Sense::click());
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
    let start = *range.start();
    let span = *range.end() - start;
    let fraction = if span > 0.0 {
        ((value - start) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (response, fraction) = rail_fraction(ui, id, fraction, &[], width);
    (response, start + fraction * span)
}

/// A rail with a tick at each stop, evenly spaced, and a thumb that maps piecewise-linearly
/// between neighbouring stops. `stops` ascends and holds at least two values.
pub(crate) fn rail_stops_slider(
    ui: &mut Ui,
    id: &str,
    value: f64,
    stops: &[f64],
    width: f32,
) -> (Response, f64) {
    let last = stops.len().saturating_sub(1).max(1);
    let ticks: Vec<f32> = (0..stops.len()).map(|i| i as f32 / last as f32).collect();
    let (response, fraction) = rail_fraction(ui, id, stop_fraction(stops, value), &ticks, width);
    (response, stop_value(stops, fraction))
}

/// Position of `value` on a rail whose evenly spaced ticks sit at `stops`, 0 to 1.
pub(crate) fn stop_fraction(stops: &[f64], value: f64) -> f64 {
    let (Some(&first), Some(&end)) = (stops.first(), stops.last()) else {
        return 0.0;
    };
    let last = stops.len() - 1;
    if last == 0 || value <= first {
        return 0.0;
    }
    if value >= end {
        return 1.0;
    }
    let index = stops.iter().rposition(|&stop| stop <= value).unwrap_or(0);
    let (Some(&low), Some(&high)) = (stops.get(index), stops.get(index + 1)) else {
        return 1.0;
    };
    (index as f64 + (value - low) / (high - low)) / last as f64
}

/// The value at rail position `fraction` (0 to 1): the inverse of [`stop_fraction`].
pub(crate) fn stop_value(stops: &[f64], fraction: f64) -> f64 {
    let (Some(&first), Some(&end)) = (stops.first(), stops.last()) else {
        return 0.0;
    };
    let last = stops.len() - 1;
    if last == 0 {
        return first;
    }
    let at = fraction.clamp(0.0, 1.0) * last as f64;
    let index = (at.floor() as usize).min(last - 1);
    let (Some(&low), Some(&high)) = (stops.get(index), stops.get(index + 1)) else {
        return end;
    };
    if fraction >= 1.0 {
        return end;
    }
    low + (at - index as f64) * (high - low)
}

/// The rail itself: the thumb sits at `fraction` (0 to 1), `ticks` are fractions to mark.
fn rail_fraction(
    ui: &mut Ui,
    id: &str,
    fraction: f64,
    ticks: &[f32],
    width: f32,
) -> (Response, f64) {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, size::SLIDER_THUMB_HEIGHT + space::XS),
        Sense::click_and_drag(),
    );
    let half = size::SLIDER_THUMB_WIDTH / 2.0;
    let mut fraction = fraction;
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.drag_started() || response.clicked())
    {
        fraction = f64::from(
            ((pointer.x - rect.left() - half) / (rect.width() - 2.0 * half)).clamp(0.0, 1.0),
        );
    }
    let x = egui::lerp(
        (rect.left() + half)..=(rect.right() - half),
        fraction as f32,
    );
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
    for &tick in ticks {
        let at = egui::lerp((rect.left() + half)..=(rect.right() - half), tick);
        paint(ui).rect_filled(
            Rect::from_center_size(
                egui::pos2(at, rail.center().y),
                Vec2::new(1.0, size::SLIDER_RAIL + 2.0 * space::XXS),
            ),
            0.0,
            theme::secondary(dark),
        );
    }
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
    (response, fraction)
}

#[cfg(test)]
mod tests;
