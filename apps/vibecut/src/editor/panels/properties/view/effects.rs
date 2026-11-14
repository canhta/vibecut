//! Renders effect parameters.
mod curve;
mod filter;
mod wheel;

use super::Panel;
use crate::editor::properties::{
    Action, ColorTarget, EffectReorder, Error, Icon, Id, ItemId, Rect, ScrubTarget, Text, Tick,
    Vec2, effect_scrub_spec, egui, icons, read, theme,
};

/// Three colour wheels share one row in the measured 460 px panel.
const WHEEL_DIAMETER: f32 = 104.0;
/// Distance a curve point or polyline must be before a click edits it.
const CURVE_REACH: f32 = 9.0;

/// Effects whose numeric text keeps authored hard-bound precision and whose
/// pending edits invalidate through the shared spatial draft path.
fn spatial_effect(kind: &str) -> bool {
    matches!(
        kind,
        "directional_blur"
            | "find_edges"
            | "color_emboss"
            | "sharpen"
            | "mosaic"
            | "unsharp_mask"
            | "bokeh_blur"
            | "mirror"
            | "offset"
    )
}

impl Panel<'_, '_> {
    fn lut_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        definition: &project::EffectDefinition,
    ) {
        let current = read::item(self.cx.session.project(), item)
            .and_then(|i| i.effects.iter().find(|e| e.id == effect))
            .and_then(|e| e.value("lut"))
            .and_then(|v| {
                if let project::EffectValue::Lut(id) = v {
                    *id
                } else {
                    None
                }
            });
        let records = self.cx.session.project().data().luts.clone();
        let busy = self.state.lut.busy();
        let enabled = !busy
            && !read::item_locked(self.cx.session.project(), item)
            && !self.cx.session.transaction_active();
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::Lut));
        value_ui.add_enabled_ui(enabled, |ui| {
            ui.spacing_mut().interact_size.y = theme::size::FIELD;
            let label = current
                .and_then(|id| records.iter().find(|r| r.id == id))
                .map_or(self.tr(Text::LutNone), |r| r.name.as_str());
            let combo = egui::ComboBox::from_id_salt(Id::unique("inspector.lut.select"))
                .width((ui.available_width() - 30.).max(40.))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    let response = ui.selectable_label(current.is_none(), self.tr(Text::LutNone));
                    self.remember("inspector.lut.none", &response);
                    if response.clicked() {
                        self.actions.push(Action::SelectLut { item, id: None });
                        ui.close();
                    }
                    for record in &records {
                        let response =
                            ui.selectable_label(current == Some(record.id), record.name.as_str());
                        self.remember(
                            format!("inspector.lut.option.{}", record.id.hex()),
                            &response,
                        );
                        if response.clicked() {
                            self.actions.push(Action::SelectLut {
                                item,
                                id: Some(record.id),
                            });
                            ui.close();
                        }
                    }
                })
                .response;
            combo.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, self.tr(Text::Lut))
            });
            self.remember("inspector.lut.select", &combo);
            let import = crate::kit::icon_button(
                ui,
                "inspector.lut.import",
                Icon::Plus,
                self.tr(Text::LutImport),
                false,
            );
            self.remember("inspector.lut.import", &import);
            if import.clicked() {
                self.actions.push(Action::ImportLut {
                    item,
                    expected: None,
                });
            }
        });
        if let Some(parameter) = definition.param("lut_strength") {
            let value = read::effect_param_value_at(
                self.cx.session.project(),
                item,
                effect,
                "lut_strength",
                self.effect_time(item),
            )
            .unwrap_or(100.);
            ui.add_enabled_ui(enabled && current.is_some(), |ui| {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::LutStrength));
                self.effect_scrub_value(
                    &mut value_ui,
                    item,
                    effect,
                    definition.id,
                    parameter,
                    value,
                    true,
                    false,
                )
            });
        }
        if busy {
            ui.label(self.tr(Text::LutLoading));
        }
        if current.is_some() && current == self.state.lut.unavailable {
            let response = ui
                .add_enabled_ui(enabled, |ui| {
                    crate::kit::icon_button(
                        ui,
                        "inspector.lut.relink",
                        Icon::FolderOpen,
                        self.tr(Text::LutLocate),
                        false,
                    )
                })
                .inner;
            self.remember_with_icon(
                "inspector.lut.relink",
                Some(Icon::FolderOpen.name()),
                &response,
            );
            if response.clicked() {
                self.actions.push(Action::ImportLut {
                    item,
                    expected: current,
                });
            }
        }
    }
    /// The item-relative playhead for a value read, or the clip start outside it.
    pub(super) fn effect_time(&self, item: ItemId) -> Tick {
        let Some((start, end)) = read::range_of(self.cx.session.project(), item) else {
            return Tick::ZERO;
        };
        let at = self.cx.playhead;
        if start <= at && at <= end {
            at.saturating_sub(start)
        } else {
            Tick::ZERO
        }
    }

    fn effect_name(&self, definition: &project::EffectDefinition) -> &'static str {
        crate::i18n::effect_label(definition.id, None).map_or(definition.name, |key| self.tr(key))
    }

    /// Adjust tab: the item's colour-grade effect, shown at neutral until the
    /// first edit adds it to the stack (`set_effect_param` creates it). The
    /// sub-tabs match the desktop reference: Basic, HSL, Curves, Color wheel.
    pub(super) fn adjust_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.id == "color_grade")
        else {
            return;
        };
        let existing = read::item(self.cx.session.project(), item).and_then(|candidate| {
            candidate
                .effects
                .iter()
                .find(|effect| effect.kind == "color_grade")
                .map(|effect| effect.id.clone())
        });
        let present = existing.is_some();
        let id = existing.unwrap_or_else(|| definition.id.to_owned());
        let at = self.effect_time(item);
        self.adjust_tabs(ui, &id);
        let tab = self.state.adjust_tab;
        match tab {
            super::super::AdjustTab::Basic => {
                for key in project::GRADE_BASIC {
                    let Some(parameter) = definition.param(key) else {
                        continue;
                    };
                    let current =
                        read::effect_param_value_at(self.cx.session.project(), item, &id, key, at)
                            .or_else(|| parameter.default.scalar())
                            .unwrap_or(0.0);
                    self.effect_scrub_field(ui, item, &id, definition.id, parameter, current);
                }
            }
            super::super::AdjustTab::Hsl => self.hsl_controls(ui, item, &id, definition, at),
            super::super::AdjustTab::Curves => self.curve_controls(ui, item, &id, present),
            super::super::AdjustTab::Wheel => self.wheel_controls(ui, item, &id, definition, at),
        }
        self.lut_controls(ui, item, &id, definition);
        self.adjust_footer(ui, item, &id, present);
    }

    /// The four measured Adjust sub-tabs. Switching one changes no project data.
    fn adjust_tabs(&mut self, ui: &mut egui::Ui, effect: &str) {
        let index = super::super::AdjustTab::ALL
            .iter()
            .position(|tab| *tab == self.state.adjust_tab)
            .unwrap_or(0);
        let ids: Vec<String> = super::super::AdjustTab::ALL
            .iter()
            .map(|tab| format!("effects.{effect}.tab.{}", tab.id()))
            .collect();
        let labels: Vec<&str> = super::super::AdjustTab::ALL
            .iter()
            .map(|tab| self.tr(tab.label()))
            .collect();
        let cells = crate::kit::segmented(ui, &ids, &labels, index);
        for (position, response) in cells.iter().enumerate() {
            if let Some(id) = ids.get(position) {
                self.remember(id.clone(), response);
            }
            if response.clicked()
                && let Some(tab) = super::super::AdjustTab::ALL.get(position)
            {
                self.state.adjust_tab = *tab;
            }
        }
        ui.add_space(theme::space::MD);
    }

    /// One hue band's hue, saturation and lightness rows.
    fn hsl_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        definition: &project::EffectDefinition,
        at: Tick,
    ) {
        let band = self.state.hsl_band.min(project::HSL_BANDS.len() - 1);
        let ids: Vec<String> = project::HSL_BANDS
            .iter()
            .map(|band| format!("effects.{effect}.hsl.{band}"))
            .collect();
        let labels: Vec<&str> = project::HSL_BANDS
            .iter()
            .map(|band| self.band_label(band))
            .collect();
        let cells = crate::kit::segmented(ui, &ids, &labels, band);
        for (position, response) in cells.iter().enumerate() {
            if let Some(id) = ids.get(position) {
                self.remember(id.clone(), response);
            }
            if response.clicked() {
                self.state.hsl_band = position;
            }
        }
        ui.add_space(theme::space::MD);
        let band = project::HSL_BANDS
            .get(self.state.hsl_band)
            .copied()
            .unwrap_or("red");
        for control in project::HSL_CONTROLS {
            let key = format!("hsl_{band}_{control}");
            let Some(parameter) = definition.param(&key) else {
                continue;
            };
            let current =
                read::effect_param_value_at(self.cx.session.project(), item, effect, &key, at)
                    .or_else(|| parameter.default.scalar())
                    .unwrap_or(0.0);
            // The band is already picked above, so the row keeps the bare control name.
            self.effect_scrub_value(
                ui,
                item,
                effect,
                definition.id,
                parameter,
                current,
                false,
                false,
            );
        }
    }

    /// The generic inspector lists every band's three parameters, so each HSL row names its
    /// band; the Adjust tab picks a band first and keeps the bare control name.
    fn hsl_row_label(&self, kind: &str, parameter: &str, label: &str) -> String {
        let band = (kind == "color_grade")
            .then(|| {
                project::HSL_BANDS
                    .iter()
                    .find(|band| parameter.starts_with(&format!("hsl_{band}_")))
            })
            .flatten();
        match band {
            Some(band) => crate::i18n::format(
                self.cx.locale,
                Text::EffectHslBandRow,
                &[("band", self.band_label(band)), ("control", label)],
            ),
            None => label.to_owned(),
        }
    }

    fn band_label(&self, band: &str) -> &'static str {
        self.tr(match band {
            "orange" => Text::EffectOrange,
            "yellow" => Text::EffectYellow,
            "green" => Text::EffectGreen,
            "cyan" => Text::EffectCyan,
            "blue" => Text::EffectBlue,
            "purple" => Text::EffectPurple,
            "magenta" => Text::EffectMagenta,
            _ => Text::EffectRed,
        })
    }

    /// Footer: copy the grade to every visual clip, or store it as an app preset.
    fn adjust_footer(&mut self, ui: &mut egui::Ui, item: ItemId, effect: &str, present: bool) {
        ui.add_space(theme::space::MD);
        let enabled = present
            && !read::item_locked(self.cx.session.project(), item)
            && !self.cx.session.transaction_active();
        let spare = theme::space::SM;
        let width = ((ui.available_width() - spare) / 2.0).max(80.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spare;
            ui.add_enabled_ui(enabled, |ui| {
                let apply = crate::kit::action_button(
                    ui,
                    &format!("effects.{effect}.apply_all"),
                    Icon::Copy,
                    self.tr(Text::ApplyToAll),
                    width,
                );
                self.remember(format!("effects.{effect}.apply_all"), &apply);
                if apply.clicked() {
                    self.actions.push(Action::ApplyGradeToAll { item });
                }
                let save = crate::kit::badge_button(
                    ui,
                    &format!("effects.{effect}.save_preset"),
                    self.tr(Text::SaveAsPreset),
                );
                self.remember(format!("effects.{effect}.save_preset"), &save);
                if save.clicked() {
                    self.actions.push(Action::SaveGradePreset { item });
                }
            });
        });
    }

    /// Three lift/gamma/gain wheels under the numeric grade rows.
    fn wheel_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        definition: &project::EffectDefinition,
        at: Tick,
    ) {
        let enabled = !read::item_locked(self.cx.session.project(), item)
            && (!self.cx.session.transaction_active() || self.state.graph_open());
        let spare = theme::space::LG * 2.0;
        let column = ((ui.available_width() - spare) / 3.0).max(72.0);
        ui.add_space(theme::space::MD);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::space::LG;
            for family in wheel::Family::ALL {
                let values = family.keys().map(|key| {
                    read::effect_param_value_at(self.cx.session.project(), item, effect, key, at)
                        .or_else(|| {
                            definition
                                .param(key)
                                .and_then(|param| param.default.scalar())
                        })
                        .unwrap_or(0.0)
                });
                // Each wheel stacks its title row over its disc in a column of its own.
                ui.vertical(|ui| {
                    self.wheel_control(ui, item, effect, family, values, column, enabled);
                });
            }
        });
        ui.add_space(theme::space::MD);
    }

    #[allow(clippy::too_many_arguments)]
    fn wheel_control(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        family: wheel::Family,
        values: [f64; 3],
        width: f32,
        enabled: bool,
    ) {
        let wheel = wheel::Wheel::from_values(family, values);
        let reset_id = format!("effects.{effect}.wheel.{}.reset", family.id());
        ui.allocate_ui_with_layout(
            Vec2::new(width, theme::size::CONTROL),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(self.tr(family.label()))
                        .size(theme::type_size::BODY)
                        .color(self.text_color()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reset = self.icon_only(
                        ui,
                        &reset_id,
                        Icon::ArrowCounterClockwise,
                        self.tr(Text::ResetValue),
                        None,
                        enabled && !family.is_neutral(values),
                    );
                    if reset {
                        self.actions.push(Action::SetEffectValues {
                            item,
                            effect: effect.to_owned(),
                            values: family
                                .keys()
                                .into_iter()
                                .zip(family.neutral())
                                .map(|(key, value)| {
                                    (key.to_owned(), project::EffectParamUpdate::Scalar(value))
                                })
                                .collect(),
                        });
                    }
                });
            },
        );
        let (area, _) =
            ui.allocate_exact_size(Vec2::new(width, WHEEL_DIAMETER), egui::Sense::hover());
        let wheel_size = (area.width() - 18.0).clamp(48.0, WHEEL_DIAMETER);
        let center = egui::pos2(area.right() - wheel_size / 2.0, area.center().y);
        let wheel_rect = Rect::from_center_size(center, Vec2::splat(wheel_size));
        let master_rect = Rect::from_min_max(
            egui::pos2(area.left() + 2.0, area.top() + 6.0),
            egui::pos2(area.left() + 8.0, area.bottom() - 6.0),
        );
        {
            let painter = ui.painter();
            let outer = wheel_rect.width() / 2.0 - 1.0;
            let inner = outer - 10.0;
            let mut mesh = egui::Mesh::default();
            const SEGMENTS: usize = 64;
            for index in 0..SEGMENTS {
                let from = index as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let to = (index + 1) as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let color = egui::Color32::from(egui::ecolor::Hsva::new(
                    index as f32 / SEGMENTS as f32,
                    0.75,
                    0.62,
                    1.0,
                ));
                let base = mesh.vertices.len() as u32;
                for (angle, radius) in [(from, inner), (from, outer), (to, outer), (to, inner)] {
                    mesh.colored_vertex(
                        center + Vec2::new(angle.cos(), -angle.sin()) * radius,
                        color,
                    );
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
            painter.add(mesh);
            painter.circle_filled(center, inner - 1.0, self.control_fill());
            let cross = egui::Stroke::new(1.0, self.secondary());
            painter.line_segment(
                [center - Vec2::new(6.0, 0.0), center + Vec2::new(6.0, 0.0)],
                cross,
            );
            painter.line_segment(
                [center - Vec2::new(0.0, 6.0), center + Vec2::new(0.0, 6.0)],
                cross,
            );
            let puck_radius = (inner - 4.0).max(1.0);
            let puck = center
                + Vec2::new(
                    (wheel.radius * wheel.hue.cos()) as f32,
                    -(wheel.radius * wheel.hue.sin()) as f32,
                ) * puck_radius;
            painter.circle_filled(puck, 5.0, egui::Color32::WHITE);
            painter.circle_stroke(puck, 5.0, egui::Stroke::new(1.0, self.control_fill()));
            painter.rect_filled(master_rect, theme::radius::FIELD, self.control_fill());
            let knob =
                master_rect.center().y - wheel.master as f32 * (master_rect.height() / 2.0 - 4.0);
            painter.circle_filled(
                egui::pos2(master_rect.center().x, knob),
                4.0,
                self.text_color(),
            );
        }
        let wheel_id = format!("effects.{effect}.wheel.{}", family.id());
        let response = ui.interact(
            wheel_rect,
            Id::unique(&wheel_id),
            egui::Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Slider, enabled, self.tr(family.label()))
        });
        self.remember(wheel_id, &response);
        self.wheel_gesture(
            item,
            effect,
            family,
            values,
            wheel,
            wheel_rect,
            master_rect,
            false,
            enabled,
            &response,
        );
        let master_id = format!("effects.{effect}.wheel.{}.master", family.id());
        let response = ui.interact(
            master_rect.expand(4.0),
            Id::unique(&master_id),
            egui::Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Slider, enabled, self.tr(family.label()))
        });
        self.remember(master_id, &response);
        self.wheel_gesture(
            item,
            effect,
            family,
            values,
            wheel,
            wheel_rect,
            master_rect.expand(4.0),
            true,
            enabled,
            &response,
        );
    }

    /// One pointer proposal on a wheel or its master; the whole drag is one gesture.
    #[allow(clippy::too_many_arguments)]
    fn wheel_gesture(
        &mut self,
        item: ItemId,
        effect: &str,
        family: wheel::Family,
        values: [f64; 3],
        wheel: wheel::Wheel,
        wheel_rect: Rect,
        master_rect: Rect,
        master: bool,
        enabled: bool,
        response: &egui::Response,
    ) {
        if !enabled {
            return;
        }
        let key = if master {
            format!("wheel:{}:master", family.id())
        } else {
            format!("wheel:{}", family.id())
        };
        if (response.dragged() || response.clicked())
            && let Some(position) = response.interact_pointer_pos()
        {
            let next = if master {
                let reach = (master_rect.height() / 2.0 - 4.0).max(1.0);
                let level = ((master_rect.center().y - position.y) / reach).clamp(-1.0, 1.0);
                wheel.with_master(f64::from(level))
            } else {
                let reach = (wheel_rect.width() / 2.0 - 11.0).max(1.0);
                let x = f64::from((position.x - wheel_rect.center().x) / reach);
                let y = f64::from((wheel_rect.center().y - position.y) / reach);
                wheel.with_puck(x, y)
            };
            if next.values(family) != values {
                if !self.state.graph_matches(item, effect, &key) {
                    let actions = self.state.begin_graph(item, effect, &key);
                    self.actions.extend(actions);
                }
                for (parameter, value) in family.keys().into_iter().zip(next.values(family)) {
                    self.set_effect_param(item, effect, parameter, value);
                }
                self.state.touch_graph();
            }
        }
        if (response.drag_stopped() || response.clicked())
            && self.state.graph_matches(item, effect, &key)
        {
            let actions = self.state.end_graph(true);
            self.actions.extend(actions);
        }
    }

    /// One channel's tone curve beside the primary controls.
    fn curve_controls(&mut self, ui: &mut egui::Ui, item: ItemId, effect: &str, present: bool) {
        let index = self.state.curve_channel;
        let channel = curve::Channel::ALL
            .get(index)
            .copied()
            .unwrap_or(curve::Channel::Red);
        let tone = read::effect_curve(self.cx.session.project(), item, effect, channel.key())
            .unwrap_or(project::ToneCurve::IDENTITY);
        let enabled = !read::item_locked(self.cx.session.project(), item)
            && (!self.cx.session.transaction_active() || self.state.graph_open());
        let reset_id = format!("effects.{effect}.curve.reset");
        ui.horizontal(|ui| {
            let reset_width = theme::size::CONTROL + theme::space::SM;
            let width = (ui.available_width() - reset_width).max(120.0);
            let ids = curve::Channel::ALL
                .map(|channel| format!("effects.{effect}.curve.{}", channel.id()));
            let labels = curve::Channel::ALL.map(|channel| self.tr(channel.label()));
            ui.allocate_ui_with_layout(
                Vec2::new(width, theme::size::CONTROL),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let cells = crate::kit::segmented(ui, &ids, &labels, index);
                    for (position, response) in cells.iter().enumerate() {
                        if let Some(id) = ids.get(position) {
                            self.remember(id.clone(), response);
                        }
                        if response.clicked() {
                            self.state.curve_channel = position;
                        }
                    }
                },
            );
            let reset = self.icon_only(
                ui,
                &reset_id,
                Icon::ArrowCounterClockwise,
                self.tr(Text::ResetValue),
                None,
                enabled && present && !tone.is_identity(),
            );
            if reset {
                self.actions.push(Action::SetEffectValues {
                    item,
                    effect: effect.to_owned(),
                    values: vec![(
                        channel.key().to_owned(),
                        project::EffectParamUpdate::Curve(project::ToneCurve::IDENTITY),
                    )],
                });
            }
        });
        self.curve_plot(ui, item, effect, channel, tone, enabled);
    }

    #[allow(clippy::too_many_arguments)]
    fn curve_plot(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        channel: curve::Channel,
        tone: project::ToneCurve,
        enabled: bool,
    ) {
        let width = ui.available_width().max(1.0);
        let height = (width * 0.55).clamp(120.0, 240.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click_and_drag());
        {
            let painter = ui.painter();
            painter.rect_filled(rect, theme::radius::FIELD, self.control_fill());
            painter.rect_stroke(
                rect,
                theme::radius::FIELD,
                egui::Stroke::new(1.0, theme::border(self.cx.dark)),
                egui::StrokeKind::Inside,
            );
            for division in 1..4 {
                let fraction = division as f32 / 4.0;
                let guide = egui::Stroke::new(1.0, theme::separator(self.cx.dark));
                let x = rect.left() + rect.width() * fraction;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    guide,
                );
                let y = rect.top() + rect.height() * fraction;
                painter.line_segment(
                    [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                    guide,
                );
            }
            painter.line_segment(
                [plot_screen(rect, [0.0, 0.0]), plot_screen(rect, [1.0, 1.0])],
                egui::Stroke::new(1.0, theme::separator(self.cx.dark)),
            );
            let points: Vec<egui::Pos2> = tone
                .points()
                .iter()
                .map(|point| plot_screen(rect, *point))
                .collect();
            if points.len() >= 2 {
                painter.add(egui::Shape::line(
                    points.clone(),
                    egui::Stroke::new(1.5, theme::accent(self.cx.dark)),
                ));
            }
            for point in points {
                painter.circle_filled(point, 4.0, theme::accent(self.cx.dark));
                painter.circle_stroke(point, 4.0, egui::Stroke::new(1.0, self.control_fill()));
            }
        }
        let plot_id = format!("effects.{effect}.curve.plot");
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Slider, enabled, self.tr(channel.label()))
        });
        self.remember(plot_id, &response);
        self.curve_gesture(ui, item, effect, channel, tone, enabled, &response, rect);
    }

    /// A click adds a point, a drag moves one, and a double-click removes an interior point.
    #[allow(clippy::too_many_arguments)]
    fn curve_gesture(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        channel: curve::Channel,
        tone: project::ToneCurve,
        enabled: bool,
        response: &egui::Response,
        rect: Rect,
    ) {
        if !enabled {
            return;
        }
        let key = format!("curve:{}", channel.key());
        let index_key = Id::unique(format!("effects.{effect}.curve.{}.drag", channel.id()));
        if response.drag_started()
            && let Some(position) = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos())
            && let Some(index) = nearest_point(rect, tone.points(), position)
            && index > 0
            && index + 1 < tone.points().len()
        {
            let actions = self.state.begin_graph(item, effect, &key);
            self.actions.extend(actions);
            ui.data_mut(|data| data.insert_temp(index_key, index));
        }
        if response.dragged()
            && self.state.graph_matches(item, effect, &key)
            && let Some(index) = ui.data(|data| data.get_temp::<usize>(index_key))
            && let Some(position) = response.interact_pointer_pos()
        {
            let point = plot_value(rect, position);
            if let Ok(next) = curve::apply(
                tone,
                curve::Edit::Move {
                    index,
                    x: point[0],
                    y: point[1],
                },
            ) && next != tone
            {
                self.write_curve(item, effect, channel, next);
                self.state.touch_graph();
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<usize>(index_key));
            if self.state.graph_matches(item, effect, &key) {
                let actions = self.state.end_graph(true);
                self.actions.extend(actions);
            }
        }
        if response.double_clicked() {
            if let Some(position) = response.interact_pointer_pos()
                && let Some(index) = nearest_point(rect, tone.points(), position)
                && index > 0
                && index + 1 < tone.points().len()
                && let Ok(next) = curve::apply(tone, curve::Edit::Remove { index })
            {
                self.one_curve_edit(item, effect, channel, next, &key);
            }
        } else if response.clicked()
            && let Some(position) = response.interact_pointer_pos()
            && nearest_point(rect, tone.points(), position).is_none()
            && near_curve(rect, tone.points(), position)
        {
            let point = plot_value(rect, position);
            if let Ok(next) = curve::apply(
                tone,
                curve::Edit::Add {
                    x: point[0],
                    y: point[1],
                },
            ) {
                self.one_curve_edit(item, effect, channel, next, &key);
            }
        }
    }

    /// A click or double-click is one complete engine gesture.
    fn one_curve_edit(
        &mut self,
        item: ItemId,
        effect: &str,
        channel: curve::Channel,
        next: project::ToneCurve,
        key: &str,
    ) {
        let actions = self.state.begin_graph(item, effect, key);
        self.actions.extend(actions);
        self.write_curve(item, effect, channel, next);
        self.state.touch_graph();
        let actions = self.state.end_graph(true);
        self.actions.extend(actions);
    }

    fn write_curve(
        &mut self,
        item: ItemId,
        effect: &str,
        channel: curve::Channel,
        next: project::ToneCurve,
    ) {
        self.actions.push(Action::SetEffectValues {
            item,
            effect: effect.to_owned(),
            values: vec![(
                channel.key().to_owned(),
                project::EffectParamUpdate::Curve(next),
            )],
        });
    }

    pub(super) fn effects_stack(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let effects = read::item(self.cx.session.project(), item)
            .map_or_else(Vec::new, |candidate| candidate.effects.clone());
        if effects.is_empty() {
            let response = ui.label(
                egui::RichText::new(self.tr(Text::EffectEmptyStack))
                    .small()
                    .color(self.secondary()),
            );
            self.remember("effects.stack.empty", &response);
            return;
        }
        let mut rows: Vec<(String, Rect)> = Vec::new();
        let mut released = false;
        for effect in &effects {
            let mut dropped = false;
            let row = ui
                .scope(|ui| {
                    ui.horizontal(|ui| {
                        // The properties body runs with zero item spacing; the row restores the
                        // kit's density gap so the eye and trash icons do not touch the name.
                        ui.spacing_mut().item_spacing.x = theme::space::MD;
                        let handle_id = format!("effects.row.{}.handle", effect.id);
                        let (rect, handle) = ui.allocate_exact_size(
                            Vec2::new(theme::size::CONTROL, theme::size::FIELD),
                            egui::Sense::click_and_drag(),
                        );
                        if handle.hovered() || handle.dragged() {
                            ui.painter().rect_filled(
                                rect,
                                theme::radius::CONTROL,
                                self.control_fill(),
                            );
                        }
                        icons::tinted(
                            Icon::ArrowsInLineHorizontal,
                            theme::icon::DENSE,
                            self.secondary(),
                        )
                        .paint_at(
                            ui,
                            Rect::from_center_size(rect.center(), Vec2::splat(theme::icon::DENSE)),
                        );
                        let handle = handle.on_hover_text(self.tr(Text::EffectReorder));
                        self.remember_with_icon(
                            handle_id,
                            Some(Icon::ArrowsInLineHorizontal.name()),
                            &handle,
                        );
                        if handle.drag_started() {
                            self.state.reorder = Some(EffectReorder {
                                item,
                                effect: effect.id.clone(),
                            });
                        }
                        if handle.drag_stopped() {
                            dropped = true;
                        }
                        let label_color = if effect.enabled {
                            self.text_color()
                        } else {
                            self.secondary()
                        };
                        let name = effect
                            .definition()
                            .ok()
                            .map_or("Effect", |definition| self.effect_name(definition));
                        ui.label(
                            egui::RichText::new(name)
                                .size(theme::type_size::CHROME)
                                .color(label_color),
                        );
                        if !effect.enabled {
                            let response = ui.label(
                                egui::RichText::new(self.tr(Text::EffectDisabled))
                                    .small()
                                    .color(self.secondary()),
                            );
                            self.remember(format!("effects.row.{}.disabled", effect.id), &response);
                        }
                        let (icon, label) = if effect.enabled {
                            (Icon::Eye, Text::EffectDisable)
                        } else {
                            (Icon::EyeSlash, Text::EffectEnable)
                        };
                        let enabled = self.icon_only(
                            ui,
                            &format!("effects.toggle.{}", effect.id),
                            icon,
                            self.tr(label),
                            Some(effect.enabled),
                            true,
                        );
                        if enabled {
                            self.actions.push(Action::ToggleEffect {
                                item,
                                effect: effect.id.clone(),
                                enabled: !effect.enabled,
                            });
                        }
                        let remove = self.tr(Text::EffectRemove);
                        if self.icon_only(
                            ui,
                            &format!("effects.remove.{}", effect.id),
                            Icon::Trash,
                            remove,
                            None,
                            true,
                        ) {
                            self.actions.push(Action::RemoveEffect {
                                item,
                                effect: effect.id.clone(),
                            });
                        }
                    });
                    if let Ok(definition) = effect.definition() {
                        let at = self.effect_time(item);
                        let spatial = spatial_effect(definition.id);
                        for parameter in definition.params {
                            match parameter.kind {
                                project::EffectParamKind::Float { .. }
                                | project::EffectParamKind::Angle => {
                                    if let Some(current) = read::effect_param_value_at(
                                        self.cx.session.project(),
                                        item,
                                        &effect.id,
                                        parameter.id,
                                        at,
                                    ) {
                                        let keyed = read::channel_has_keys(
                                            self.cx.session.project(),
                                            item,
                                            &effect_param_path(&effect.id, parameter.id),
                                        );
                                        let inside =
                                            read::range_of(self.cx.session.project(), item)
                                                .is_some_and(|(start, end)| {
                                                    start <= self.cx.playhead
                                                        && self.cx.playhead <= end
                                                });
                                        let enabled = !spatial || !keyed || inside;
                                        ui.add_enabled_ui(enabled, |ui| {
                                            self.effect_scrub_field(
                                                ui,
                                                item,
                                                &effect.id,
                                                definition.id,
                                                parameter,
                                                if spatial {
                                                    let (lo, hi) = parameter
                                                        .scalar_range()
                                                        .unwrap_or((0.0, 0.0));
                                                    current.clamp(lo, hi)
                                                } else {
                                                    current
                                                },
                                            )
                                        });
                                    }
                                }
                                project::EffectParamKind::Choice { .. }
                                | project::EffectParamKind::Bool => {
                                    if let Some(value) = effect.value(parameter.id) {
                                        self.effect_discrete_field(
                                            ui, item, &effect.id, parameter, *value,
                                        );
                                    }
                                }
                                project::EffectParamKind::Color if parameter.animatable => {
                                    self.effect_color_field(ui, item, &effect.id, parameter.id);
                                }
                                project::EffectParamKind::Color
                                | project::EffectParamKind::Point
                                | project::EffectParamKind::Lut
                                | project::EffectParamKind::Curve => {}
                            }
                        }
                    }
                })
                .response
                .rect;
            let full = Rect::from_min_max(
                egui::pos2(ui.max_rect().left(), row.top()),
                egui::pos2(ui.max_rect().right(), row.bottom()),
            );
            rows.push((effect.id.clone(), full));
            released |= dropped;
        }
        if released {
            let pointer = ui.input(|input| {
                input
                    .pointer
                    .interact_pos()
                    .or_else(|| input.pointer.latest_pos())
            });
            let target = pointer.and_then(|position| {
                rows.iter()
                    .find(|(id, rect)| {
                        self.state
                            .reorder
                            .as_ref()
                            .is_none_or(|drag| drag.effect != *id)
                            && rect.contains(position)
                    })
                    .map(|(id, _)| id.clone())
            });
            if let Some(drag) = self.state.reorder.take()
                && let Some(target) = target
            {
                self.actions.push(Action::ReorderEffect {
                    item: drag.item,
                    effect: drag.effect,
                    target,
                });
            }
        } else if self.state.reorder.is_some() && ui.input(|input| input.pointer.any_released()) {
            self.state.reorder = None;
        }
    }
    fn effect_color_field(&mut self, ui: &mut egui::Ui, item: ItemId, effect: &str, key: &str) {
        let label = match key {
            "target" => Text::EffectTargetColour,
            "replace" => Text::EffectReplacementColour,
            "black" => Text::EffectDarkColour,
            "white" => Text::EffectLightColour,
            "color" => {
                if read::item(self.cx.session.project(), item)
                    .and_then(|i| i.effects.iter().find(|e| e.id == effect))
                    .is_some_and(|e| e.kind == "strobe")
                {
                    Text::EffectFlashColour
                } else {
                    Text::EffectKeptColour
                }
            }
            _ => return,
        };
        let Some((start, _)) = read::range_of(self.cx.session.project(), item) else {
            return;
        };
        let target = ColorTarget::EffectColor {
            item,
            effect: effect.into(),
            key: key.into(),
            time: self.cx.playhead.saturating_sub(start),
        };
        let project = self.cx.session.project();
        let inside = target.inside(project);
        let locked = read::item_locked(project, item);
        let editable = !locked && (inside || !target.rgb_keys(project, false));
        let row = Rect::from_min_size(
            ui.next_widget_position(),
            Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
        );
        let id = format!("effects.{effect}.param.{key}");
        // A locked track, or a keyed colour with the playhead outside the clip.
        let blocked = self.tr(if locked {
            Text::EffectLocked
        } else {
            Text::EffectKeyframeTime
        });
        ui.add_enabled_ui(editable, |ui| {
            self.color_row(ui, &id, label, target.clone())
        })
        .response
        .on_disabled_hover_text(blocked);
        let exists = target.rgb_keys(project, true);
        let key_id = format!("{id}.keyframe");
        let neighbours = target.key_neighbours(project, Tick(self.cx.playhead.0 - start.0));
        let clicked = self.keyframe_cluster(
            ui,
            &key_id,
            super::keyframe_cluster_rect(row),
            super::ClusterState {
                start,
                keyed: exists,
                can_toggle: !locked && inside,
                neighbours,
                blocked: Some(blocked),
            },
        );
        if clicked {
            match target.toggle_rgb(project) {
                Ok(command) => {
                    self.actions.extend(self.state.close_picker());
                    self.actions.push(Action::ColorCommit {
                        command,
                        record: None,
                    });
                }
                Err(error) => self.fail(error),
            }
        }
    }
    fn effect_discrete_field(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        parameter: &project::EffectParamDefinition,
        current: project::EffectValue,
    ) {
        let label = match parameter.id {
            "channel" => Text::EffectChannel,
            "reverse" => Text::EffectReverse,
            "solid" => Text::EffectSolidColour,
            "invert" => Text::MaskInvert,
            "sharp" => Text::EffectSharp,
            "shape" => Text::EffectShape,
            _ => return,
        };
        let id = format!("effects.{effect}.param.{}", parameter.id);
        let locked = read::item_locked(self.cx.session.project(), item);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        value_ui.add_enabled_ui(!locked, |ui| {
            match (parameter.kind, current) {
                (project::EffectParamKind::Bool, project::EffectValue::Bool(mut value)) => {
                    // The admitted Properties checkbox box is 12 logical pixels.
                    ui.spacing_mut().icon_width = 12.0;
                    let response = ui.add(egui::Checkbox::without_text(&mut value));
                    response
                        .ctx
                        .accesskit_node_builder(response.id, |node| node.set_label(self.tr(label)));
                    self.remember(id.clone(), &response);
                    if response.changed() {
                        self.set_effect_param(
                            item,
                            effect,
                            parameter.id,
                            if value { 1.0 } else { 0.0 },
                        );
                    }
                }
                (
                    project::EffectParamKind::Choice { options },
                    project::EffectValue::Choice(index),
                ) => {
                    // No guessed index for missing data; invalid storage is read-only.
                    let Some(option) = options.get(index as usize) else {
                        return;
                    };
                    ui.spacing_mut().interact_size.y = theme::size::FIELD;
                    let response =
                        egui::ComboBox::from_id_salt(Id::unique(format!("{id}.item.{}", item.0)))
                            .selected_text(self.effect_choice_label(option))
                            .width(ui.available_width().min(super::appearance::FIELD_COLUMN))
                            .show_ui(ui, |ui| {
                                for (at, option) in options.iter().enumerate() {
                                    let response = ui.selectable_label(
                                        index as usize == at,
                                        self.effect_choice_label(option),
                                    );
                                    self.remember(format!("{id}.{at}"), &response);
                                    if response.clicked() {
                                        if index as usize != at {
                                            self.set_effect_param(
                                                item,
                                                effect,
                                                parameter.id,
                                                at as f64,
                                            );
                                        }
                                        ui.close();
                                    }
                                }
                            })
                            .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, !locked, self.tr(label))
                    });
                    self.remember(id.clone(), &response);
                }
                _ => {}
            }
        });
    }

    fn effect_choice_label(&self, option: &'static str) -> &'static str {
        match option {
            "Red" => self.tr(Text::EffectRed),
            "Green" => self.tr(Text::EffectGreen),
            "Blue" => self.tr(Text::EffectBlue),
            "Alpha" => self.tr(Text::Alpha),
            "Circle" => self.tr(Text::MaskCircle),
            "Triangle" => self.tr(Text::EffectShapeTriangle),
            "Square" => self.tr(Text::EffectShapeSquare),
            "Pentagon" => self.tr(Text::EffectShapePentagon),
            "Hexagon" => self.tr(Text::EffectShapeHexagon),
            "Octagon" => self.tr(Text::EffectShapeOctagon),
            _ => option,
        }
    }

    pub(super) fn effect_scrub_field(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        kind: &str,
        parameter: &project::EffectParamDefinition,
        current: f64,
    ) {
        self.effect_scrub_value(ui, item, effect, kind, parameter, current, false, true);
    }
    #[allow(clippy::too_many_arguments)]
    fn effect_scrub_value(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        kind: &str,
        parameter: &project::EffectParamDefinition,
        current: f64,
        row: bool,
        name_band: bool,
    ) {
        let spec = effect_scrub_spec(parameter);
        let spatial = spatial_effect(kind) || row;
        if spatial
            && self.actions.iter().any(|a| {
                matches!(a, Action::SetTab { .. } | Action::Activate(_))
                    || matches!(a, Action::RemoveEffect { item: target, effect: id }
                            if *target == item && id == effect)
            })
        {
            let cancel = self.state.end_scrub(false);
            self.actions.splice(0..0, cancel);
            self.state.drafts.clear();
            self.state.focus = None;
            self.state.spatial_edit = None;
            return;
        }
        let unit = if !spatial {
            ""
        } else {
            match parameter.id {
                "direction" | "angle" | "rotation" => "\u{b0}",
                "length" | "relief" | "radius" => "px",
                "contrast" | "blend" | "softness" | "highlight" | "center_x" | "center_y"
                | "shift_x" | "shift_y" => "%",
                "amount" => {
                    if kind == "bokeh_blur" {
                        "px"
                    } else {
                        "%"
                    }
                }
                _ => "",
            }
        };
        if !row {
            ui.add_space(theme::space::MD);
        }
        let label = crate::i18n::effect_label(kind, Some(parameter.id))
            .map_or(parameter.name, |key| self.tr(key));
        let label = if name_band {
            self.hsl_row_label(kind, parameter.id, label)
        } else {
            label.to_owned()
        };
        let caption = (!row).then(|| ui.label(label));
        let target = ScrubTarget::Effect {
            effect: effect.to_owned(),
            key: parameter.id.to_owned(),
            inspector: row,
        };
        let draft_key = target.draft_key();
        let widget_key = target.field_id();
        ui.horizontal(|ui| {
            let handle_id = target.widget_id();
            let (rect, handle) = ui.allocate_exact_size(
                Vec2::new(theme::size::CONTROL, theme::size::FIELD),
                egui::Sense::click_and_drag(),
            );
            if handle.hovered() || handle.dragged() {
                ui.painter()
                    .rect_filled(rect, theme::radius::CONTROL, self.control_fill());
            }
            icons::tinted(Icon::ArrowsHorizontal, theme::icon::DENSE, self.secondary()).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(theme::icon::DENSE)),
            );
            let handle = handle.on_hover_text(self.tr(Text::DragToAdjust));
            self.remember_with_icon(
                handle_id.clone(),
                Some(Icon::ArrowsHorizontal.name()),
                &handle,
            );
            if handle.drag_started() {
                self.start_scrub_target(item, target.clone(), current);
                if spatial {
                    self.state.spatial_edit = Some((item, target.clone(), self.cx.playhead));
                }
            }
            if handle.dragged()
                && let Some(mut scrub) = self.state.scrub.take()
            {
                if scrub.target == target {
                    let steps = f64::from(handle.total_drag_delta().map_or(0.0, |delta| delta.x))
                        / f64::from(spec.pixels_per_step);
                    let value = spec.quantize(scrub.start + steps * spec.step);
                    if value != scrub.last {
                        self.write_effect_param(item, effect, parameter, value);
                        scrub.last = value;
                    }
                }
                self.state.scrub = Some(scrub);
            }
            if handle.drag_stopped() {
                self.finish_scrub(true);
            }
            let reserve = if parameter.animatable {
                theme::size::KEYFRAME_CLUSTER + theme::space::MD
            } else {
                0.0
            };
            let width = if spatial {
                54.0
            } else {
                (ui.available_width() - reserve).max(60.0)
            };
            let mut next = self
                .state
                .drafts
                .get(&draft_key)
                .cloned()
                .unwrap_or_else(|| {
                    if spatial {
                        current.to_string()
                    } else {
                        spec.format(current)
                    }
                });
            let response = ui.add_sized(
                [width, if spatial { theme::size::FIELD } else { 26.0 }],
                egui::TextEdit::singleline(&mut next)
                    .char_limit(32)
                    .background_color(self.control_fill())
                    .id(Id::unique(&draft_key))
                    .hint_text(
                        egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                    ),
            );
            let response = if let Some(caption) = &caption {
                response.labelled_by(caption.id)
            } else {
                response
            };
            if row {
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::TextInput,
                        ui.is_enabled(),
                        self.tr(Text::LutStrength),
                    )
                });
            }
            self.remember(widget_key.clone(), &response);
            if spatial {
                ui.label(unit);
            }
            if response.has_focus() {
                self.state.focus = Some(draft_key.clone());
                if spatial {
                    self.state.spatial_edit = Some((item, target.clone(), self.cx.playhead));
                }
            }
            if response.changed() {
                self.state.drafts.insert(draft_key.clone(), next);
            }
            let editing =
                response.has_focus() || self.state.focus.as_deref() == Some(draft_key.as_str());
            if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.state.drafts.remove(&draft_key);
                self.state.focus = None;
                response.surrender_focus();
                return;
            }
            let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || response.lost_focus() {
                self.state.focus = None;
                if let Some(draft) = self.state.drafts.remove(&draft_key) {
                    let Ok(parsed) = draft.trim().parse::<f64>() else {
                        self.fail(Error::Invalid("scrub value"));
                        return;
                    };
                    if !parsed.is_finite() {
                        self.fail(Error::Invalid("scrub value"));
                        return;
                    }
                    let value = if spatial {
                        let (lo, hi) = parameter.scalar_range().unwrap_or((0.0, 0.0));
                        if !(lo..=hi).contains(&parsed) {
                            self.fail(Error::Invalid("effect parameter value"));
                            return;
                        }
                        parsed
                    } else {
                        spec.quantize(parsed)
                    };
                    self.write_effect_param(item, effect, parameter, value);
                }
            }
            if parameter.animatable {
                self.effect_keyframe_diamond(ui, item, effect, parameter, current);
            }
        });
    }

    /// An animated channel writes a key at the playhead; every other scalar
    /// writes the persisted value.
    fn write_effect_param(
        &mut self,
        item: ItemId,
        effect: &str,
        parameter: &project::EffectParamDefinition,
        value: f64,
    ) {
        let path = effect_param_path(effect, parameter.id);
        let target = read::item(self.cx.session.project(), item);
        if target.is_some_and(|i| {
            i.effects
                .iter()
                .any(|e| e.id == effect && spatial_effect(&e.kind))
        }) && let Ok(animation_path) = project::AnimationPath::new(&path)
            && let Some(project::Param::Value(channel)) =
                target.and_then(|i| i.animations.get(&animation_path))
            && channel.keys().is_empty()
        {
            let (before, after) = channel.extrapolation();
            match project::Channel::new(
                project::ParamValue::Scalar(value),
                Vec::new(),
                before,
                after,
            ) {
                Ok(channel) => self.actions.push(Action::SetAnimatedParam {
                    item,
                    path: animation_path,
                    param: project::Param::Value(channel),
                }),
                Err(error) => self.fail(error),
            }
            return;
        }
        if let Some(time) = self.animated_effect_time(item, &path) {
            self.actions.push(Action::SetKeyframeAt {
                item,
                field: path,
                time,
                value,
            });
        } else {
            self.set_effect_param(item, effect, parameter.id, value);
        }
    }

    fn animated_effect_time(&self, item: ItemId, path: &str) -> Option<Tick> {
        if !read::channel_has_keys(self.cx.session.project(), item, path) {
            return None;
        }
        let (start, end) = read::range_of(self.cx.session.project(), item)?;
        let at = self.cx.playhead;
        (start <= at && at <= end).then(|| at.saturating_sub(start))
    }

    /// The reference puts one diamond after each animatable effect control.
    fn effect_keyframe_diamond(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        parameter: &project::EffectParamDefinition,
        current: f64,
    ) {
        let path = effect_param_path(effect, parameter.id);
        let Some((start, end)) = read::range_of(self.cx.session.project(), item) else {
            return;
        };
        let at = self.cx.playhead;
        let inside = start <= at && at <= end;
        let relative = at.saturating_sub(start);
        let exists = read::has_key_at(self.cx.session.project(), item, &path, relative);
        let neighbours = read::neighbour_keys(
            self.cx.session.project(),
            item,
            &[path.as_str()],
            Tick(at.0 - start.0),
        );
        let id = format!("effects.{effect}.param.{}.keyframe", parameter.id);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(theme::size::KEYFRAME_CLUSTER, theme::size::CONTROL),
            egui::Sense::hover(),
        );
        let clicked = self.keyframe_cluster(
            ui,
            &id,
            rect,
            super::ClusterState {
                start,
                keyed: exists,
                can_toggle: inside,
                neighbours,
                blocked: None,
            },
        );
        if clicked && inside {
            self.actions.push(Action::ToggleKeyframe {
                item,
                fields: vec![(path, current)],
                time: relative,
            });
        }
    }
}

fn effect_param_path(effect: &str, key: &str) -> String {
    format!("effects.{effect}.params.{key}")
}

/// Plot coordinates to the panel; the curve's 0..1 domain runs up the height.
fn plot_screen(rect: Rect, [x, y]: [f64; 2]) -> egui::Pos2 {
    rect.min + Vec2::new(x as f32 * rect.width(), (1.0 - y as f32) * rect.height())
}

/// A pointer position as one normalized plot coordinate.
fn plot_value(rect: Rect, position: egui::Pos2) -> [f64; 2] {
    [
        f64::from(((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0)),
        f64::from(((rect.bottom() - position.y) / rect.height()).clamp(0.0, 1.0)),
    ]
}

/// The curve point within reach of `position`, nearest first.
fn nearest_point(rect: Rect, points: &[[f64; 2]], position: egui::Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| (index, plot_screen(rect, *point).distance(position)))
        .filter(|(_, distance)| *distance < CURVE_REACH)
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| index)
}

/// Whether a click lands on the drawn polyline for adding a point.
fn near_curve(rect: Rect, points: &[[f64; 2]], position: egui::Pos2) -> bool {
    points.windows(2).any(|pair| {
        let (Some(from), Some(to)) = (pair.first(), pair.get(1)) else {
            return false;
        };
        segment_distance(position, plot_screen(rect, *from), plot_screen(rect, *to)) < CURVE_REACH
    })
}

fn segment_distance(point: egui::Pos2, from: egui::Pos2, to: egui::Pos2) -> f32 {
    let span = to - from;
    let length = span.length_sq();
    if length <= f32::EPSILON {
        return point.distance(from);
    }
    let t = ((point - from).dot(span) / length).clamp(0.0, 1.0);
    (from + span * t).distance(point)
}
