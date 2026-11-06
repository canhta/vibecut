//! Renders numeric property rows: label left, controls right.
use super::Panel;
use crate::editor::properties::{
    Action, Error, ItemId, Rect, ScrubSpec, ScrubTarget, Text, Vec2, egui, read, theme,
};
use crate::kit;

/// Width of the value column's left edge from the row start, measured.
const VALUE_COLUMN: f32 = 120.0;
const FIELD_WIDTH: f32 = 68.0;
const SLIDER_GAP: f32 = 24.0;
const SLIDER_ROW: f32 = 44.0;
const SLIDER_FIELD: f32 = 54.0;

/// Shared draft state for one numeric field; the model value stays in model units.
fn percent(field: &str) -> bool {
    matches!(field, "opacity" | "transform.scaleX" | "transform.scaleY")
}

fn unit(spec: ScrubSpec) -> &'static str {
    if percent(spec.field) {
        "%"
    } else {
        spec.suffix
    }
}

fn shown(spec: ScrubSpec, value: f64) -> String {
    let factor = if percent(spec.field) { 100.0 } else { 1.0 };
    let decimals = if percent(spec.field) {
        0
    } else {
        spec.decimals
    };
    format!(
        "{:.*}{}",
        decimals,
        spec.quantize(value) * factor,
        unit(spec)
    )
}

fn parse(spec: ScrubSpec, text: &str) -> Option<f64> {
    let factor = if percent(spec.field) { 100.0 } else { 1.0 };
    let value = text
        .trim()
        .trim_end_matches(unit(spec))
        .trim()
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value / factor)
}

/// Arrow steps move one displayed unit: a percent, a pixel, a degree.
fn arrow_step(spec: ScrubSpec) -> f64 {
    if spec.field == "transform.rotate" {
        1.0
    } else {
        spec.step
    }
}

/// Where a numeric field sits: its dim prefix, width and the extra gap before its stepper.
#[derive(Clone, Copy)]
struct Layout {
    prefix: Option<&'static str>,
    width: f32,
    gap: f32,
}

impl Panel<'_, '_> {
    /// Writes one field; with uniform scale on, Scale X also writes Scale Y.
    fn write_field(&mut self, item: ItemId, spec: ScrubSpec, value: &str) {
        if let Some(time) = self.animated_time(item, spec.field) {
            // `write_field` receives `spec.format` model units, not display percent.
            let Ok(parsed) = value.trim().parse::<f64>() else {
                self.fail(Error::Invalid("scrub value"));
                return;
            };
            let value = spec.quantize(parsed);
            self.actions.push(Action::SetKeyframeAt {
                item,
                field: spec.field,
                time,
                value,
            });
            if self.state.uniform_scale && spec.field == "transform.scaleX" {
                self.actions.push(Action::SetKeyframeAt {
                    item,
                    field: "transform.scaleY",
                    time,
                    value,
                });
            }
            return;
        }
        self.set_item_field(item, spec.field, value);
        if self.state.uniform_scale && spec.field == "transform.scaleX" {
            self.set_item_field(item, "transform.scaleY", value);
        }
    }

    /// The playhead's item-relative time when this field already has keys and
    /// the playhead sits inside the item; an edit must then upsert a key.
    fn animated_time(&self, item: ItemId, field: &str) -> Option<time::Tick> {
        if !matches!(
            field,
            "opacity"
                | "audio_gain"
                | "transform.positionX"
                | "transform.positionY"
                | "transform.scaleX"
                | "transform.scaleY"
                | "transform.rotate"
        ) || !read::channel_has_keys(self.cx.session.project(), item, field)
        {
            return None;
        }
        let (start, end) = read::range_of(self.cx.session.project(), item)?;
        let at = self.cx.playhead;
        (start <= at && at <= end).then(|| at.saturating_sub(start))
    }

    fn item_value(&self, item: ItemId, field: &str) -> Option<f64> {
        read::item_value(self.cx.session.project(), item, field)
    }

    /// A label on the left of a 39 px row with the controls at the value column.
    /// The label drags the value, like the scrub handle of the previous layout.
    pub(super) fn scrub_field(&mut self, ui: &mut egui::Ui, item: ItemId, spec: ScrubSpec) {
        self.number_row(ui, item, spec.label, &[(spec, None)]);
    }

    /// One slider row: the label above, the rail and the value group below.
    pub(super) fn slider_row(&mut self, ui: &mut egui::Ui, item: ItemId, spec: ScrubSpec) {
        let Some(current) = self.item_value(item, spec.field) else {
            return;
        };
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), SLIDER_ROW),
            egui::Sense::hover(),
        );
        let label = self.tr(spec.label);
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.dark),
        );
        let label_rect = Rect::from_min_size(
            egui::pos2(row.left(), row.top() - 2.0),
            Vec2::new(galley.size().x.max(theme::size::CONTROL), 18.0),
        );
        ui.painter().galley(
            egui::pos2(
                label_rect.left(),
                label_rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::text(self.cx.dark),
        );
        self.scrub_handle(ui, item, spec, current, label_rect);
        let line = Rect::from_min_size(
            egui::pos2(row.left(), row.top() + 14.0),
            Vec2::new(row.width(), theme::size::FIELD),
        );
        let group = SLIDER_FIELD + theme::space::XXS + theme::size::STEPPER;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(line.right() - group, line.top()),
                    line.max,
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = theme::space::XXS;
        self.numeric_edit(
            &mut child,
            item,
            spec,
            current,
            Layout {
                prefix: None,
                width: SLIDER_FIELD,
                gap: 0.0,
            },
        );
        let rail_width = (line.width() - group - SLIDER_GAP).max(40.0);
        let mut rail_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    line.min,
                    Vec2::new(rail_width, line.height()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let max = if percent(spec.field) && spec.max > 4.0 {
            4.0
        } else {
            spec.max
        };
        let id = format!("inspector.{}.slider", spec.field);
        let (response, value) =
            kit::rail_slider(&mut rail_ui, &id, current, spec.min..=max, rail_width);
        self.remember(id, &response);
        self.drive_scrub(item, spec, current, &response, value);
        self.keyframe_diamonds(ui, row, item, &[spec]);
    }

    /// Rows with one or two fields sharing a label: Position (X, Y), Rotate, Text size.
    pub(super) fn number_row(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        label: Text,
        fields: &[(ScrubSpec, Option<&'static str>)],
    ) {
        let Some(&(first, _)) = fields.first() else {
            return;
        };
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
            egui::Sense::hover(),
        );
        let color = theme::text(self.cx.dark);
        let galley = ui.painter().layout_no_wrap(
            self.tr(label).to_owned(),
            egui::FontId::proportional(theme::type_size::BODY),
            color,
        );
        let label_rect = Rect::from_min_size(
            egui::pos2(row.left(), row.center().y - 9.0),
            Vec2::new(
                galley
                    .size()
                    .x
                    .clamp(theme::size::CONTROL, VALUE_COLUMN - 8.0),
                18.0,
            ),
        );
        ui.painter().galley(
            egui::pos2(label_rect.left(), row.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        let Some(current) = self.item_value(item, first.field) else {
            return;
        };
        self.scrub_handle(ui, item, first, current, label_rect);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(row.left() + VALUE_COLUMN, row.top()),
                    row.max,
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = theme::space::XXS;
        for (index, (spec, prefix)) in fields.iter().enumerate() {
            let Some(value) = self.item_value(item, spec.field) else {
                continue;
            };
            if index > 0 {
                child.add_space(3.0);
            }
            self.numeric_edit(
                &mut child,
                item,
                *spec,
                value,
                Layout {
                    prefix: *prefix,
                    width: FIELD_WIDTH,
                    gap: 4.0,
                },
            );
        }
        let specs: Vec<ScrubSpec> = fields.iter().map(|(spec, _)| *spec).collect();
        self.keyframe_diamonds(ui, row, item, &specs);
    }

    pub(super) fn start_scrub(&mut self, item: ItemId, spec: ScrubSpec, value: f64) {
        self.start_scrub_target(item, ScrubTarget::Item(spec.field), value);
    }

    pub(super) fn start_effect_scrub(&mut self, item: ItemId, effect: &str, key: &str, value: f64) {
        let target = ScrubTarget::Effect {
            effect: effect.to_owned(),
            key: key.to_owned(),
        };
        self.start_scrub_target(item, target, value);
    }

    pub(super) fn start_scrub_target(&mut self, item: ItemId, target: ScrubTarget, value: f64) {
        let actions = self.state.begin_scrub(item, target, value);
        self.actions.extend(actions);
    }

    pub(super) fn finish_scrub(&mut self, commit: bool) {
        let actions = self.state.end_scrub(commit);
        self.actions.extend(actions);
    }

    /// The scrub drag target over a label: drags the value one engine undo step.
    fn scrub_handle(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        spec: ScrubSpec,
        current: f64,
        rect: Rect,
    ) {
        let handle_id = format!("inspector.{}.scrub", spec.field);
        let handle = ui
            .push_id(&handle_id, |ui| {
                ui.interact(
                    rect,
                    ui.scope_id().with("scrub"),
                    egui::Sense::click_and_drag(),
                )
            })
            .inner
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text(self.tr(Text::DragToAdjust));
        self.remember(handle_id, &handle);
        if handle.drag_started() {
            self.start_scrub(item, spec, current);
        }
        if handle.dragged()
            && let Some(mut scrub) = self.state.scrub.take()
        {
            if scrub.target == ScrubTarget::Item(spec.field) {
                let steps = f64::from(handle.total_drag_delta().map_or(0.0, |delta| delta.x))
                    / f64::from(spec.pixels_per_step);
                let value = spec.quantize(scrub.start + steps * spec.step);
                if value != scrub.last {
                    self.write_field(item, spec, &spec.format(value));
                    scrub.last = value;
                }
            }
            self.state.scrub = Some(scrub);
        }
        if handle.drag_stopped() {
            self.finish_scrub(true);
        }
    }

    /// Rail drags use the same preview-then-commit gesture as the label.
    fn drive_scrub(
        &mut self,
        item: ItemId,
        spec: ScrubSpec,
        current: f64,
        response: &egui::Response,
        value: f64,
    ) {
        if response.drag_started() || (response.clicked() && self.state.scrub.is_none()) {
            self.start_scrub(item, spec, current);
        }
        if (response.dragged() || response.clicked())
            && let Some(mut scrub) = self.state.scrub.take()
        {
            if scrub.target == ScrubTarget::Item(spec.field) {
                let value = spec.quantize(value);
                if value != scrub.last {
                    self.write_field(item, spec, &spec.format(value));
                    scrub.last = value;
                }
            }
            self.state.scrub = Some(scrub);
        }
        if response.drag_stopped() || response.clicked() {
            self.finish_scrub(true);
        }
    }

    /// Draft field (commit on Enter or blur, Escape cancels) with its stepper.
    fn numeric_edit(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        spec: ScrubSpec,
        current: f64,
        layout: Layout,
    ) {
        let Layout { prefix, width, gap } = layout;
        let id = format!("inspector.{}", spec.field);
        let mut next = self
            .state
            .drafts
            .get(spec.field)
            .cloned()
            .unwrap_or_else(|| shown(spec, current));
        let response = kit::value_field(ui, &id, &mut next, width, prefix, self.tr(spec.label));
        self.remember(id.clone(), &response);
        if response.has_focus() {
            self.state.focus = Some(spec.field.to_owned());
        }
        if response.changed() {
            self.state.drafts.insert(spec.field.to_owned(), next);
        }
        let label = self.tr(spec.label);
        ui.add_space(gap);
        let (up, down) = kit::stepper(ui, &id, label);
        self.remember(format!("{id}.increase"), &up);
        self.remember(format!("{id}.decrease"), &down);
        for (clicked, sign) in [(up.clicked(), 1.0), (down.clicked(), -1.0)] {
            if clicked {
                let value = spec.quantize(current + sign * arrow_step(spec));
                self.write_field(item, spec, &spec.format(value));
                self.state.drafts.remove(spec.field);
            }
        }
        let editing = response.has_focus() || self.state.focus.as_deref() == Some(spec.field);
        if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(spec.field);
            self.state.focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.state.focus = None;
            if let Some(draft) = self.state.drafts.remove(spec.field) {
                let Some(parsed) = parse(spec, &draft) else {
                    self.fail(Error::Invalid("scrub value"));
                    return;
                };
                let value = spec.quantize(parsed);
                self.write_field(item, spec, &spec.format(value));
            }
        }
    }
}
