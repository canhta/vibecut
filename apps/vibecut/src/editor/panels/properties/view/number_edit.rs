//! Renders numeric property rows: label left, controls right.
use super::Panel;
use crate::editor::properties::{
    Action, Error, ItemId, Rect, SPEED_SCRUB, SPEED_STOPS, ScrubSpec, ScrubTarget, Text, Vec2,
    egui, read, theme,
};
use crate::kit;

/// Width of the value column's left edge from the row start, measured.
const VALUE_COLUMN: f32 = 120.0;
const FIELD_WIDTH: f32 = 68.0;
const SLIDER_GAP: f32 = 24.0;
const SLIDER_ROW: f32 = 44.0;
const SLIDER_FIELD: f32 = 54.0;

/// A whole-number slider row over panel state or a batched edit, not an item field.
pub(super) struct WholeSlider {
    pub(super) id: &'static str,
    /// The draft key of the typed value.
    pub(super) draft: &'static str,
    pub(super) label: Text,
    /// The refusal subject of an unparsable typed value.
    pub(super) what: &'static str,
    pub(super) min: u32,
    pub(super) max: u32,
    pub(super) current: u32,
    pub(super) ticks: Option<u32>,
}

/// What a `WholeSlider` gesture produced this frame.
pub(super) enum WholeEdit {
    /// A rail drag in progress.
    Live(u32),
    /// A release, click, step or typed value.
    Commit(u32),
}

/// Shared draft state for one numeric field; the model value stays in model units.
/// The field's text: the model value scaled for display, in whole units when scaled.
fn shown(spec: ScrubSpec, value: f64) -> String {
    if let Some(label) = spec.floor_text(value) {
        return format!("{label}{}", spec.suffix);
    }
    let decimals = if spec.scale == 1.0 { spec.decimals } else { 0 };
    format!(
        "{:.*}{}",
        decimals,
        spec.quantize(value) * spec.scale,
        spec.suffix
    )
}

/// The model value a typed field text means, undoing the display scale.
fn parse(spec: ScrubSpec, text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches(spec.suffix).trim();
    if spec.is_floor_text(text) {
        return Some(spec.min / spec.scale);
    }
    let value = text.parse::<f64>().ok()?;
    value.is_finite().then_some(value / spec.scale)
}

/// Arrow steps move one displayed unit: a percent, a pixel, a degree.
fn arrow_step(spec: ScrubSpec) -> f64 {
    if project::BuiltinPath::parse(spec.field) == Some(project::BuiltinPath::TransformRotate) {
        1.0
    } else {
        spec.step
    }
}

/// Where a numeric field sits: its dim prefix, width and the extra gap before its stepper.
#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub(super) prefix: Option<&'static str>,
    pub(super) width: f32,
    pub(super) gap: f32,
}

impl Panel<'_, '_> {
    /// Writes one field; with uniform scale on, Scale X also writes Scale Y.
    fn write_field(&mut self, item: ItemId, spec: ScrubSpec, value: &str) {
        // Fades are seconds on `audio.set_fade`, not item params, and never keyed.
        if let Some(edge) = read::fade_edge(spec.field) {
            match value.trim().parse::<f64>() {
                Ok(seconds) => self.actions.push(Action::SetFade {
                    item,
                    edge,
                    seconds,
                }),
                Err(_) => self.fail(Error::Invalid("fade duration")),
            }
            return;
        }
        if (spec.field == "opacity" || spec.field.starts_with("params."))
            && read::channel_has_keys(self.cx.session.project(), item, spec.field)
            && self.animated_time(item, spec.field).is_none()
        {
            self.fail(Error::Invalid("opacity playhead outside item"));
            return;
        }
        let scale_x =
            project::BuiltinPath::parse(spec.field) == Some(project::BuiltinPath::TransformScaleX);
        if let Some(time) = self.animated_time(item, spec.field) {
            // `write_field` receives `spec.format` model units, not display percent.
            let Ok(parsed) = value.trim().parse::<f64>() else {
                self.fail(Error::Invalid("scrub value"));
                return;
            };
            let value = spec.quantize(parsed);
            self.actions.push(Action::SetKeyframeAt {
                item,
                field: spec.field.to_owned(),
                time,
                value,
            });
            if self.state.uniform_scale && scale_x {
                self.actions.push(Action::SetKeyframeAt {
                    item,
                    field: project::BuiltinPath::TransformScaleY.as_str().to_owned(),
                    time,
                    value,
                });
            }
            return;
        }
        self.set_item_field(item, spec.field, value);
        if self.state.uniform_scale && scale_x {
            self.set_item_field(item, project::BuiltinPath::TransformScaleY.as_str(), value);
        }
    }

    /// The playhead's item-relative time when this field already has keys and
    /// the playhead sits inside the item; an edit must then upsert a key.
    fn animated_time(&self, item: ItemId, field: &str) -> Option<time::Tick> {
        let animatable = field == "volume"
            || field.starts_with("params.")
            || matches!(
                project::BuiltinPath::parse(field),
                Some(
                    project::BuiltinPath::Opacity
                        | project::BuiltinPath::TransformPositionX
                        | project::BuiltinPath::TransformPositionY
                        | project::BuiltinPath::TransformScaleX
                        | project::BuiltinPath::TransformScaleY
                        | project::BuiltinPath::TransformRotate
                )
            );
        if !animatable || !read::channel_has_keys(self.cx.session.project(), item, field) {
            return None;
        }
        let (start, end) = read::range_of(self.cx.session.project(), item)?;
        let at = self.cx.playhead;
        (start <= at
            && (at < end || (field != "opacity" && !field.starts_with("params.") && at == end)))
            .then(|| at.saturating_sub(start))
    }

    fn item_value(&self, item: ItemId, field: &str) -> Option<f64> {
        if field == "opacity" || field.starts_with("params.") {
            let (start, _) = read::range_of(self.cx.session.project(), item)?;
            return read::item_value_at(
                self.cx.session.project(),
                item,
                field,
                self.cx.playhead.saturating_sub(start),
            );
        }
        read::item_value(self.cx.session.project(), item, field)
    }

    /// A label on the left of a 39 px row with the controls at the value column.
    /// The label drags the value, like the scrub handle of the previous layout.
    pub(super) fn scrub_field(&mut self, ui: &mut egui::Ui, item: ItemId, spec: ScrubSpec) {
        self.number_row(ui, item, spec.label, &[(spec, None)]);
    }

    /// One slider row: the label above, the rail and the value group below.
    pub(super) fn slider_row(&mut self, ui: &mut egui::Ui, item: ItemId, spec: ScrubSpec) {
        self.slider_row_in(ui, item, spec, true);
    }

    /// A slider row whose label, rail and value are read-only unless `editable`. The
    /// row-end keyframe cluster stays live either way, so its arrows still navigate.
    pub(super) fn slider_row_in(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        spec: ScrubSpec,
        editable: bool,
    ) {
        let Some(current) = self.item_value(item, spec.field) else {
            return;
        };
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), SLIDER_ROW),
            egui::Sense::hover(),
        );
        let line = Rect::from_min_size(
            egui::pos2(row.left(), row.top() + 14.0),
            Vec2::new(row.width(), theme::size::FIELD),
        );
        ui.add_enabled_ui(editable, |ui| {
            self.slider_controls(ui, item, spec, current, row, line);
        });
        self.keyframe_diamonds(ui, line, item, &[spec]);
    }

    fn slider_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        spec: ScrubSpec,
        current: f64,
        row: Rect,
        line: Rect,
    ) {
        let line = Rect::from_min_size(
            line.min,
            Vec2::new(
                (line.width() - super::keyframe_reserve(&[spec])).max(0.0),
                line.height(),
            ),
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
        let max = if spec.scale != 1.0 && spec.max > 4.0 {
            4.0
        } else {
            spec.max
        };
        let id = format!("inspector.{}.slider", spec.field);
        let (response, value) = if spec.field == SPEED_SCRUB.field {
            kit::rail_stops_slider(&mut rail_ui, &id, current, &SPEED_STOPS, rail_width)
        } else {
            kit::rail_slider(&mut rail_ui, &id, current, spec.min..=max, rail_width)
        };
        self.remember(id, &response);
        self.drive_scrub(item, spec, current, &response, value);
    }

    /// The matte brush's Size row. Size is panel view state (1 to 100), not an item field, so
    /// both a rail drag and a commit write through `SetMatteBrushSize`, with no keyframes or
    /// undo step.
    pub(super) fn matte_size_row(&mut self, ui: &mut egui::Ui) {
        let current = self.cx.matte_brush_size;
        let edit = self.whole_slider_row(
            ui,
            WholeSlider {
                id: "inspector.matte_brush.size",
                draft: "matte_brush_size",
                label: Text::EffectSize,
                what: "brush size",
                min: crate::editor::MATTE_BRUSH_SIZE_MIN,
                max: crate::editor::MATTE_BRUSH_SIZE_MAX,
                current,
                ticks: None,
            },
        );
        if let Some(WholeEdit::Live(size) | WholeEdit::Commit(size)) = edit
            && size != current
        {
            self.actions.push(Action::SetMatteBrushSize(size));
        }
    }

    /// A whole-number slider row that is not an item field: the label above, the rail and the
    /// value group (field and stepper) below, the slider row's layout. A rail drag reports
    /// `Live` values and a `Commit` on release or click; the stepper and a typed value
    /// (Enter or blur, its sign dropped and clamped into range as the reference does) report a
    /// `Commit`. `ticks` marks the rail every that many units.
    pub(super) fn whole_slider_row(
        &mut self,
        ui: &mut egui::Ui,
        slider: WholeSlider,
    ) -> Option<WholeEdit> {
        let WholeSlider {
            id,
            draft,
            label,
            what,
            min,
            max,
            current,
            ticks,
        } = slider;
        let label = self.tr(label);
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), SLIDER_ROW),
            egui::Sense::hover(),
        );
        let line = Rect::from_min_size(
            egui::pos2(row.left(), row.top() + 14.0),
            Vec2::new(row.width(), theme::size::FIELD),
        );
        let color = theme::text(self.cx.dark);
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(theme::type_size::BODY),
            color,
        );
        ui.painter().galley(
            egui::pos2(row.left(), row.top() + 7.0 - galley.size().y / 2.0),
            galley,
            color,
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
        let mut edit = None;
        let mut text = self
            .state
            .drafts
            .get(draft)
            .cloned()
            .unwrap_or_else(|| current.to_string());
        let response = kit::value_field(&mut child, id, &mut text, SLIDER_FIELD, None, label);
        self.remember(id, &response);
        if response.has_focus() {
            self.state.focus = Some(draft.to_owned());
        }
        if response.changed() {
            self.state.drafts.insert(draft.to_owned(), text);
        }
        let (up, down) = kit::stepper(&mut child, id, label);
        self.remember(format!("{id}.increase"), &up);
        self.remember(format!("{id}.decrease"), &down);
        for (clicked, next) in [
            (up.clicked(), current.saturating_add(1)),
            (down.clicked(), current.saturating_sub(1)),
        ] {
            if clicked {
                edit = Some(WholeEdit::Commit(next.clamp(min, max)));
                self.state.drafts.remove(draft);
            }
        }
        let editing = response.has_focus() || self.state.focus.as_deref() == Some(draft);
        if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(draft);
            self.state.focus = None;
            response.surrender_focus();
        } else if (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            || response.lost_focus()
        {
            self.state.focus = None;
            if let Some(typed) = self.state.drafts.remove(draft) {
                match typed
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                {
                    Some(number) => {
                        let value =
                            number.abs().round().clamp(f64::from(min), f64::from(max)) as u32;
                        edit = Some(WholeEdit::Commit(value));
                    }
                    None => self.fail(Error::Invalid(what)),
                }
            }
        }
        let rail_width = (line.width() - group - SLIDER_GAP).max(40.0);
        let mut rail_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    line.min,
                    Vec2::new(rail_width, line.height()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let slider_id = format!("{id}.slider");
        let (response, value) = kit::rail_slider(
            &mut rail_ui,
            &slider_id,
            f64::from(current),
            f64::from(min)..=f64::from(max),
            rail_width,
        );
        self.remember(slider_id, &response);
        if let Some(every) = ticks.filter(|every| *every > 0 && max > min) {
            let half = theme::size::SLIDER_THUMB_WIDTH / 2.0;
            let mark = egui::Stroke::new(1.0, theme::secondary(self.cx.dark));
            let below = response.rect.center().y + theme::size::SLIDER_RAIL;
            for at in (min..=max).filter(|at| (at - min) % every == 0) {
                let fraction = (at - min) as f32 / (max - min) as f32;
                let x = egui::lerp(
                    (response.rect.left() + half)..=(response.rect.right() - half),
                    fraction,
                );
                ui.painter().line_segment(
                    [egui::pos2(x, below + 1.0), egui::pos2(x, below + 4.0)],
                    mark,
                );
            }
        }
        let value = value.round() as u32;
        if response.drag_stopped() || response.clicked() {
            edit = Some(WholeEdit::Commit(value));
        } else if response.dragged() && value != current {
            edit = Some(WholeEdit::Live(value));
        }
        edit
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
        let specs: Vec<ScrubSpec> = fields.iter().map(|(spec, _)| *spec).collect();
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(row.left() + VALUE_COLUMN, row.top()),
                    egui::pos2(row.right() - super::keyframe_reserve(&specs), row.bottom()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = theme::space::XXS;
        // Fields keep their measured width and narrow only when the panel is too narrow for
        // every field, its gap and its stepper (the default columns at the minimum window).
        const GAP: f32 = 4.0;
        const BETWEEN: f32 = 3.0;
        let count = fields.len() as f32;
        let per_field = GAP + theme::size::STEPPER + 2.0 * theme::space::XXS;
        let room = child.max_rect().width()
            - count * per_field
            - (count - 1.0) * (BETWEEN + theme::space::XXS);
        let width = (room / count).clamp(theme::size::CONTROL, FIELD_WIDTH);
        for (index, (spec, prefix)) in fields.iter().enumerate() {
            let Some(value) = self.item_value(item, spec.field) else {
                continue;
            };
            if index > 0 {
                child.add_space(BETWEEN);
            }
            self.numeric_edit(
                &mut child,
                item,
                *spec,
                value,
                Layout {
                    prefix: *prefix,
                    width,
                    gap: GAP,
                },
            );
        }
        self.keyframe_diamonds(ui, row, item, &specs);
    }

    pub(super) fn start_scrub(&mut self, item: ItemId, spec: ScrubSpec, value: f64) {
        self.start_scrub_target(item, ScrubTarget::Item(spec.field), value);
        if spec.field == "opacity" || spec.field.starts_with("params.") {
            self.state.opacity_edit =
                Some((self.cx.session.project().clone(), item, self.cx.playhead));
        }
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
    pub(super) fn scrub_handle(
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
    pub(super) fn numeric_edit(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        spec: ScrubSpec,
        current: f64,
        layout: Layout,
    ) {
        let Layout { prefix, width, gap } = layout;
        let id = format!("inspector.{}", spec.field.replace("params.", "graphic."));
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
            if spec.field == "opacity" || spec.field.starts_with("params.") {
                self.state.opacity_edit =
                    Some((self.cx.session.project().clone(), item, self.cx.playhead));
            }
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
