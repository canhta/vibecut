//! Draws the mask sub-panel and turns its edits into mask actions.
use super::state::{Choice, State};
use crate::egui::{self, Align, Align2, Id, Rect, Sense, Stroke, StrokeKind, Vec2};
use crate::i18n::{self, Locale, Text};
use crate::kit as theme;
use project::{Animations, ItemId, Mask};
use time::Tick;

/// The reference's mask panel is 240 px wide; the shape grid needs a little more.
const WIDTH: f32 = 288.0;
const TILE: f32 = 60.0;
const TILE_GAP: f32 = 8.0;
const CELL_HEIGHT: f32 = 78.0;
const FIELD_WIDTH: f32 = 64.0;
const DONE: Vec2 = Vec2::new(90.0, 34.0);

/// The playhead as the mask's keys see it: the item's animations, the item-relative
/// time, and whether that time sits inside the item where keys can change.
#[derive(Clone, Copy)]
pub struct Timing<'a> {
    pub animations: &'a Animations,
    pub time: Tick,
    pub inside: bool,
}

/// One open mask with its playhead: the stored mask, the mask at the playhead
/// (what the fields show) and the edit gates.
#[derive(Clone, Copy)]
struct Keys<'a> {
    item: ItemId,
    timing: Timing<'a>,
    mask: &'a Mask,
    shown: &'a Mask,
    locked: bool,
}

impl<'a> Keys<'a> {
    /// The animated parameter behind one mask field, if it has one.
    fn param(self, key: &str) -> Option<&'a project::Param> {
        let animations: &'a Animations = self.timing.animations;
        animations.get(&self.mask.path(key).ok()?)
    }
    /// Whether the field is animated at all: its parameter exists.
    fn animated(self, key: &str) -> bool {
        self.param(key).is_some()
    }
    /// Whether the field's channel owns a key at the playhead.
    fn keyed(self, key: &str) -> bool {
        let time = self.timing.time;
        self.param(key)
            .and_then(|param| param.channel(None).ok())
            .is_some_and(|channel| {
                channel
                    .keys()
                    .binary_search_by_key(&time, |k| k.time)
                    .is_ok()
            })
    }
    /// Whether the field carries keys; edits at the playhead then upsert one.
    fn curved(self, key: &str) -> bool {
        self.param(key)
            .and_then(|param| param.channel(None).ok())
            .is_some_and(|channel| !channel.keys().is_empty())
    }
    /// The engine value the toggle records for an unkeyed field: what is shown now.
    fn shown_value(self, key: &str) -> f64 {
        engine_value(self.shown, key)
    }
}

/// Engine units of a field, as the static and keyed edits store them.
fn engine_value(mask: &Mask, key: &str) -> f64 {
    match key {
        "feather" => mask.feather,
        "width" => mask.width,
        "height" => mask.height,
        "center_x" => mask.center_x,
        "center_y" => mask.center_y,
        "rotation" => mask.rotation,
        _ => 0.0,
    }
}

/// Converts a displayed field value (per cent for position and size) to engine units.
fn engine_units(key: &str, value: f64) -> f64 {
    match key {
        "feather" => value.clamp(0.0, 100.0),
        "width" | "height" => (value / 100.0).clamp(0.000_001, 100.0),
        "center_x" | "center_y" => (value / 100.0).clamp(-100.0, 100.0),
        "rotation" => value.clamp(-1_000_000.0, 1_000_000.0),
        _ => value,
    }
}

/// What the mask panel asks the properties owner to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Close,
    /// Add the shape when the item has no mask, update the first one otherwise;
    /// `None` removes it.
    Choose {
        item: ItemId,
        shape: Option<project::MaskShape>,
    },
    /// A static edit of the stored mask.
    Update {
        item: ItemId,
        mask: Mask,
    },
    /// Toggle the diamond over one group of mask fields at the playhead, as one step.
    ToggleKeyframe {
        item: ItemId,
        mask: String,
        fields: Vec<(&'static str, f64)>,
        time: Tick,
    },
    /// Upsert a key on an animated mask field at the playhead.
    SetKeyframe {
        item: ItemId,
        mask: String,
        key: &'static str,
        value: f64,
        time: Tick,
    },
    /// Replace the base of an animated mask field that holds no keys.
    SetBase {
        item: ItemId,
        mask: String,
        key: &'static str,
        value: f64,
    },
}

/// Widgets for the frame's automation map, requests to route and whether a
/// field owns the keyboard.
pub struct Event {
    pub widgets: Vec<(String, egui::Response)>,
    pub actions: Vec<Action>,
    pub editing: bool,
}

/// Draw the open panel below its row, clamped to the window.
#[allow(clippy::too_many_arguments)]
pub fn show(
    state: &mut State,
    ctx: &egui::Context,
    locale: Locale,
    dark: bool,
    anchor: egui::Pos2,
    item: ItemId,
    mask: Option<&Mask>,
    locked: bool,
    timing: Option<Timing<'_>>,
) -> Event {
    let mut event = Event {
        widgets: Vec::new(),
        actions: Vec::new(),
        editing: false,
    };
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        event.actions.push(Action::Close);
        return event;
    }
    let tr = |key| i18n::text(locale, key);
    let window = egui::Window::new(tr(Text::Mask))
        .id(Id::unique("inspector.mask.panel"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(anchor)
        .constrain_to(ctx.content_rect().shrink(8.0));
    window.show(ctx, |ui| {
        ui.set_width(WIDTH);
        header(ui, tr, &mut event);
        ui.add_space(theme::space::XS);
        grid(ui, tr, dark, locked, item, mask, &mut event);
        if let Some(mask) = mask {
            ui.add_space(theme::space::XS);
            ui.separator();
            ui.add_space(theme::space::XS);
            ui.label(
                egui::RichText::new(tr(Text::Adjust))
                    .strong()
                    .size(theme::type_size::BODY),
            );
            // Fields show the mask at the playhead; static edits still write the stored mask.
            let shown = timing.map_or_else(
                || mask.clone(),
                |timing| {
                    mask.at(timing.animations, timing.time)
                        .unwrap_or_else(|_| mask.clone())
                },
            );
            let keys = timing.map(|timing| Keys {
                item,
                timing,
                mask,
                shown: &shown,
                locked,
            });
            fields(
                ui, state, locale, tr, dark, locked, item, mask, &shown, keys, &mut event,
            );
            ui.add_space(theme::space::XS);
        }
        ui.separator();
        ui.add_space(theme::space::XS);
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let response = ui.add_sized(
                [DONE.x, DONE.y],
                egui::Button::new(
                    egui::RichText::new(tr(Text::MaskDone)).color(theme::on_accent(dark)),
                )
                .fill(theme::accent(dark)),
            );
            record(&mut event.widgets, "inspector.mask.done", response.clone());
            if response.clicked() {
                event.actions.push(Action::Close);
            }
        });
    });
    event
}

fn header(ui: &mut egui::Ui, tr: impl Fn(Text) -> &'static str + Copy, event: &mut Event) {
    ui.horizontal(|ui| {
        let back = theme::icon_button(
            ui,
            "inspector.mask.back",
            theme::Icon::ArrowLeft,
            tr(Text::Mask),
            false,
        );
        record(&mut event.widgets, "inspector.mask.back", back.clone());
        if back.clicked() {
            event.actions.push(Action::Close);
        }
        ui.label(
            egui::RichText::new(tr(Text::Mask))
                .strong()
                .size(theme::type_size::CHROME),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let close = theme::icon_button(
                ui,
                "inspector.mask.close",
                theme::Icon::X,
                tr(Text::Mask),
                false,
            );
            record(&mut event.widgets, "inspector.mask.close", close.clone());
            if close.clicked() {
                event.actions.push(Action::Close);
            }
        });
    });
}

fn grid(
    ui: &mut egui::Ui,
    tr: impl Fn(Text) -> &'static str + Copy,
    dark: bool,
    locked: bool,
    item: ItemId,
    mask: Option<&Mask>,
    event: &mut Event,
) {
    let selected = mask.and_then(|mask| Choice::of(&mask.shape));
    for row in Choice::ALL.chunks(4) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for choice in row {
                let (cell, response) = ui.allocate_exact_size(
                    Vec2::new(TILE, CELL_HEIGHT),
                    if locked {
                        Sense::hover()
                    } else {
                        Sense::click()
                    },
                );
                let tile = Rect::from_min_size(cell.min, Vec2::splat(TILE));
                let active = selected == Some(*choice);
                let fill = if active || response.hovered() {
                    theme::hover(dark)
                } else {
                    theme::field(dark)
                };
                let painter = ui.painter();
                painter.rect_filled(tile, theme::radius::CONTROL, fill);
                if active {
                    painter.rect_stroke(
                        tile,
                        theme::radius::CONTROL,
                        Stroke::new(1.5, theme::accent(dark)),
                        StrokeKind::Inside,
                    );
                }
                glyph(ui, tile, *choice, dark);
                let label_rect = Rect::from_min_size(
                    egui::pos2(cell.left(), tile.bottom() + 2.0),
                    Vec2::new(TILE, CELL_HEIGHT - TILE - 2.0),
                );
                let mut label_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(label_rect)
                        .layout(egui::Layout::top_down(Align::Center)),
                );
                label_ui.add(
                    egui::Label::new(
                        egui::RichText::new(tr(choice.label()))
                            .font(theme::label_font())
                            .color(theme::text(dark)),
                    )
                    .truncate(),
                );
                let id = format!("inspector.mask.shape.{}", choice.id());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::Button,
                        !locked,
                        active,
                        tr(choice.label()),
                    )
                });
                record(&mut event.widgets, &id, response.clone());
                if response.clicked() && !locked {
                    event.actions.push(Action::Choose {
                        item,
                        shape: choice.shape(),
                    });
                }
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn fields(
    ui: &mut egui::Ui,
    state: &mut State,
    locale: Locale,
    tr: impl Fn(Text) -> &'static str + Copy,
    dark: bool,
    locked: bool,
    item: ItemId,
    mask: &Mask,
    shown: &Mask,
    keys: Option<Keys<'_>>,
    event: &mut Event,
) {
    let mut changes: Vec<(&'static str, f64)> = Vec::new();
    feather_row(
        ui,
        state,
        tr,
        dark,
        locked,
        shown,
        keys,
        &mut changes,
        event,
    );
    let choice = Choice::of(&mask.shape);
    let (show_width, show_height) = choice.map_or((true, true), |choice| choice.size_fields());
    if show_width || show_height {
        let mut specs = Vec::new();
        if show_width {
            specs.push(Spec {
                id: "inspector.mask.size.w",
                key: "width",
                prefix: Some("W"),
                value: shown.width * 100.0,
                min: 1.0,
                max: 10_000.0,
                step: 1.0,
                suffix: "%",
            });
        }
        if show_height {
            specs.push(Spec {
                id: "inspector.mask.size.h",
                key: "height",
                prefix: Some("H"),
                value: shown.height * 100.0,
                min: 1.0,
                max: 10_000.0,
                step: 1.0,
                suffix: "%",
            });
        }
        row(
            ui,
            state,
            locale,
            Text::MaskSize,
            &specs,
            dark,
            locked,
            &mut changes,
            event,
            keys.map(|keys| (keys, &["width", "height"][..])),
        );
    }
    row(
        ui,
        state,
        locale,
        Text::Position,
        &[
            Spec {
                id: "inspector.mask.position.x",
                key: "center_x",
                prefix: Some("X"),
                value: shown.center_x * 100.0,
                min: -10_000.0,
                max: 10_000.0,
                step: 1.0,
                suffix: "%",
            },
            Spec {
                id: "inspector.mask.position.y",
                key: "center_y",
                prefix: Some("Y"),
                value: shown.center_y * 100.0,
                min: -10_000.0,
                max: 10_000.0,
                step: 1.0,
                suffix: "%",
            },
        ],
        dark,
        locked,
        &mut changes,
        event,
        keys.map(|keys| (keys, &["center_x", "center_y"][..])),
    );
    row(
        ui,
        state,
        locale,
        Text::MaskRotation,
        &[Spec {
            id: "inspector.mask.rotation",
            key: "rotation",
            prefix: None,
            value: shown.rotation,
            min: -1_000_000.0,
            max: 1_000_000.0,
            step: 1.0,
            suffix: "°",
        }],
        dark,
        locked,
        &mut changes,
        event,
        keys.map(|keys| (keys, &["rotation"][..])),
    );
    let mut inverted = mask.inverted;
    let response = ui
        .add_enabled_ui(!locked, |ui| {
            ui.horizontal(|ui| {
                let response = theme::toggle(
                    ui,
                    "inspector.mask.invert",
                    &mut inverted,
                    tr(Text::MaskInvert),
                );
                ui.label(
                    egui::RichText::new(tr(Text::MaskInvert))
                        .size(theme::type_size::BODY)
                        .color(theme::text(dark)),
                );
                response
            })
            .inner
        })
        .inner;
    record(
        &mut event.widgets,
        "inspector.mask.invert",
        response.clone(),
    );
    if response.changed() && !locked {
        changes.push(("inverted", f64::from(u8::from(inverted))));
    }
    // An animated field edits its key at the playhead, or its base when it holds no keys;
    // every other field edits the stored mask in one update.
    let mut next = mask.clone();
    let mut stored = false;
    for (key, value) in changes {
        let value = engine_units(key, value);
        match keys.filter(|keys| keys.animated(key)) {
            Some(keys) if keys.curved(key) => event.actions.push(Action::SetKeyframe {
                item,
                mask: mask.id.clone(),
                key,
                value,
                time: keys.timing.time,
            }),
            Some(_) => event.actions.push(Action::SetBase {
                item,
                mask: mask.id.clone(),
                key,
                value,
            }),
            None => stored |= set_stored(&mut next, key, value),
        }
    }
    if stored {
        event.actions.push(Action::Update { item, mask: next });
    }
}

/// Writes one stored field; `false` for a key the mask does not store.
fn set_stored(mask: &mut Mask, key: &str, value: f64) -> bool {
    match key {
        "feather" => mask.feather = value,
        "width" => mask.width = value,
        "height" => mask.height = value,
        "center_x" => mask.center_x = value,
        "center_y" => mask.center_y = value,
        "rotation" => mask.rotation = value,
        "inverted" => mask.inverted = value != 0.0,
        _ => return false,
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn feather_row(
    ui: &mut egui::Ui,
    state: &mut State,
    tr: impl Fn(Text) -> &'static str + Copy,
    dark: bool,
    locked: bool,
    shown: &Mask,
    keys: Option<Keys<'_>>,
    changes: &mut Vec<(&'static str, f64)>,
    event: &mut Event,
) {
    ui.label(
        egui::RichText::new(tr(Text::MaskFeather))
            .size(theme::type_size::BODY)
            .color(theme::text(dark)),
    );
    ui.horizontal(|ui| {
        let reserve = keys.map_or(0.0, |_| theme::size::KEYFRAME_CLUSTER + theme::space::XXS);
        let group = FIELD_WIDTH + theme::size::STEPPER + theme::space::XXS + reserve;
        let rail_width = (ui.available_width() - group - theme::space::MD).max(40.0);
        let preview = state.feather_preview().unwrap_or(shown.feather);
        let (response, value) = ui
            .add_enabled_ui(!locked, |ui| {
                theme::rail_slider(
                    ui,
                    "inspector.mask.feather.slider",
                    preview,
                    0.0..=100.0,
                    rail_width,
                )
            })
            .inner;
        record(
            &mut event.widgets,
            "inspector.mask.feather.slider",
            response.clone(),
        );
        if response.dragged() || response.drag_started() {
            state.set_feather_preview(Some(value));
        }
        if (response.drag_stopped() || response.clicked()) && !locked {
            let value = state.feather_preview().unwrap_or(value);
            state.set_feather_preview(None);
            changes.push(("feather", value));
        }
        let spec = Spec {
            id: "inspector.mask.feather",
            key: "feather",
            prefix: None,
            value: state.feather_preview().unwrap_or(shown.feather),
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        };
        if let Some(value) = number(
            ui,
            state,
            spec,
            locked,
            &mut event.widgets,
            &mut event.editing,
        ) {
            changes.push(("feather", value));
        }
        if let Some(keys) = keys {
            cluster(ui, tr, keys, &["feather"], event);
        }
    });
}

/// The row-end keyframe cluster of one group of mask fields. Its diamond keys every
/// field of the group at the playhead, or removes them when all are keyed. Previous and
/// next seeks are not wired in this popover, so both arrows stay disabled.
fn cluster(
    ui: &mut egui::Ui,
    tr: impl Fn(Text) -> &'static str + Copy,
    keys: Keys<'_>,
    group: &[&'static str],
    event: &mut Event,
) {
    let Some(lead) = group.first() else {
        return;
    };
    let id = format!("inspector.mask.keyframe.{lead}");
    let keyed = group.iter().all(|key| keys.keyed(key));
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(theme::size::KEYFRAME_CLUSTER, theme::size::CONTROL),
        Sense::hover(),
    );
    let [previous, toggle, next] = theme::keyframe_cluster(
        ui,
        &id,
        rect,
        &theme::KeyframeControl {
            previous_label: tr(Text::PreviousKeyframe),
            toggle_label: tr(if keyed {
                Text::RemoveKeyframe
            } else {
                Text::AddKeyframe
            }),
            next_label: tr(Text::NextKeyframe),
            keyed,
            can_toggle: keys.timing.inside && !keys.locked,
            has_previous: false,
            has_next: false,
        },
    );
    record(&mut event.widgets, &format!("{id}.previous"), previous);
    record(&mut event.widgets, &id, toggle.clone());
    record(&mut event.widgets, &format!("{id}.next"), next);
    if toggle.clicked() && keys.timing.inside && !keys.locked {
        event.actions.push(Action::ToggleKeyframe {
            item: keys.item,
            mask: keys.mask.id.clone(),
            fields: group
                .iter()
                .map(|key| (*key, keys.shown_value(key)))
                .collect(),
            time: keys.timing.time,
        });
    }
}

#[derive(Clone, Copy)]
struct Spec {
    id: &'static str,
    key: &'static str,
    prefix: Option<&'static str>,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    suffix: &'static str,
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    state: &mut State,
    locale: Locale,
    label: Text,
    specs: &[Spec],
    dark: bool,
    locked: bool,
    changes: &mut Vec<(&'static str, f64)>,
    event: &mut Event,
    keys: Option<(Keys<'_>, &[&'static str])>,
) {
    let (row, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
        Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(row.left(), row.center().y),
        Align2::LEFT_CENTER,
        i18n::text(locale, label),
        theme::label_font(),
        theme::text(dark),
    );
    let spec_width = specs.len() as f32 * (FIELD_WIDTH + theme::size::STEPPER);
    let gap = (specs.len().saturating_sub(1)) as f32 * (theme::space::XXS + 3.0);
    let reserve = keys.map_or(0.0, |_| theme::size::KEYFRAME_CLUSTER + theme::space::XXS);
    let mut value_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                egui::pos2(row.right() - spec_width - gap - reserve, row.top()),
                row.max,
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    value_ui.spacing_mut().item_spacing.x = theme::space::XXS;
    for (index, spec) in specs.iter().enumerate() {
        if index > 0 {
            value_ui.add_space(3.0);
        }
        if let Some(value) = number(
            &mut value_ui,
            state,
            *spec,
            locked,
            &mut event.widgets,
            &mut event.editing,
        ) {
            changes.push((spec.key, value));
        }
    }
    if let Some((keys, group)) = keys {
        let tr = |text: Text| i18n::text(locale, text);
        cluster(&mut value_ui, tr, keys, group, event);
    }
}

fn number(
    ui: &mut egui::Ui,
    state: &mut State,
    spec: Spec,
    locked: bool,
    widgets: &mut Vec<(String, egui::Response)>,
    editing: &mut bool,
) -> Option<f64> {
    let mut next = state
        .draft(spec.key)
        .map_or_else(|| shown(spec.value, spec.suffix), str::to_owned);
    let (response, (up, down)) = ui
        .add_enabled_ui(!locked, |ui| {
            ui.horizontal(|ui| {
                let response =
                    theme::value_field(ui, spec.id, &mut next, FIELD_WIDTH, spec.prefix, spec.id);
                let (up, down) = theme::stepper(ui, spec.id, spec.id);
                (response, (up, down))
            })
            .inner
        })
        .inner;
    record(widgets, spec.id, response.clone());
    record(widgets, &format!("{}.increase", spec.id), up.clone());
    record(widgets, &format!("{}.decrease", spec.id), down.clone());
    if response.has_focus() {
        *editing = true;
    }
    if response.changed() {
        state.set_draft(spec.key, next.clone());
    }
    for (clicked, sign) in [(up.clicked(), 1.0), (down.clicked(), -1.0)] {
        if clicked {
            state.clear_draft(spec.key);
            return Some((spec.value + sign * spec.step).clamp(spec.min, spec.max));
        }
    }
    let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if enter || response.lost_focus() {
        let draft = state.draft(spec.key).map(str::to_owned)?;
        state.clear_draft(spec.key);
        let value = parse(&draft, spec.suffix)?;
        return Some(value.clamp(spec.min, spec.max));
    }
    None
}

fn shown(value: f64, suffix: &str) -> String {
    format!("{value:.0}{suffix}")
}

fn parse(text: &str, suffix: &str) -> Option<f64> {
    let text = text.trim();
    let text = text.strip_suffix(suffix).unwrap_or(text).trim();
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn glyph(ui: &egui::Ui, tile: Rect, choice: Choice, dark: bool) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.5, theme::secondary(dark));
    let center = tile.center();
    let frame = Rect::from_center_size(center, Vec2::splat(30.0));
    match choice {
        Choice::None => {
            painter.circle_stroke(center, 11.0, stroke);
            painter.line_segment(
                [center + Vec2::new(-8.0, 8.0), center + Vec2::new(8.0, -8.0)],
                stroke,
            );
        }
        Choice::Horizontal => {
            painter.rect_stroke(frame, 3.0, stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(frame.left(), center.y),
                    egui::pos2(frame.right(), center.y),
                ],
                stroke,
            );
        }
        Choice::Mirror => {
            painter.rect_stroke(frame, 3.0, stroke, StrokeKind::Inside);
            for offset in [-7.0, 7.0] {
                painter.line_segment(
                    [
                        egui::pos2(frame.left(), center.y + offset),
                        egui::pos2(frame.right(), center.y + offset),
                    ],
                    stroke,
                );
            }
        }
        Choice::Circle => {
            painter.rect_stroke(frame, 3.0, stroke, StrokeKind::Inside);
            painter.circle_stroke(center, 9.0, stroke);
        }
        Choice::Rectangle => {
            painter.rect_stroke(frame, 3.0, stroke, StrokeKind::Inside);
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::splat(16.0)),
                2.0,
                stroke,
                StrokeKind::Inside,
            );
        }
        Choice::Heart => {
            painter.add(egui::Shape::closed_line(heart(center), stroke));
        }
        Choice::Stars => {
            painter.add(egui::Shape::closed_line(star(center), stroke));
        }
        Choice::Path => {
            painter.add(egui::Shape::closed_line(
                [
                    center + Vec2::new(-12.0, 8.0),
                    center + Vec2::new(-4.0, -12.0),
                    center + Vec2::new(10.0, -6.0),
                    center + Vec2::new(12.0, 10.0),
                ]
                .to_vec(),
                stroke,
            ));
            for point in [
                center + Vec2::new(-12.0, 8.0),
                center + Vec2::new(10.0, -6.0),
            ] {
                painter.circle_filled(point, 2.5, theme::secondary(dark));
            }
        }
    }
}

fn heart(center: egui::Pos2) -> Vec<egui::Pos2> {
    (0..=64)
        .map(|step| {
            let t = step as f32 / 64.0 * std::f32::consts::TAU;
            let x = t.sin().powi(3) * 16.0;
            let y =
                -(13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos());
            center + Vec2::new(x, y) * 0.62
        })
        .collect()
}

fn star(center: egui::Pos2) -> Vec<egui::Pos2> {
    (0..10)
        .map(|step| {
            let angle = step as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            let radius = if step % 2 == 0 { 14.0 } else { 6.0 };
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

fn record(widgets: &mut Vec<(String, egui::Response)>, id: &str, response: egui::Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
    widgets.push((id.to_owned(), response));
}
