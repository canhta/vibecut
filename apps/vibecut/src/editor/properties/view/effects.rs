//! Renders effect parameters.
use super::Panel;
use crate::editor::properties::{
    Action, BUTTON_ICON, EffectReorder, Error, Icon, Id, ItemId, Rect, ScrubTarget, Text, Vec2,
    effect_definition, effect_scrub_spec, egui, icons, read, theme,
};

impl Panel<'_, '_> {
    fn effect_name(&self, definition: &project::EffectDefinition) -> &'static str {
        match definition.kind {
            "blur" => self.tr(Text::EffectBlur),
            "color_grade" => self.tr(Text::EffectColorGrade),
            _ => definition.name,
        }
    }

    /// Adjust tab: the item's colour-grade effect, shown at neutral until the
    /// first edit adds it to the stack (`set_effect_param` creates it).
    pub(super) fn adjust_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.kind == "color_grade")
        else {
            return;
        };
        let existing = read::item(self.cx.session.project(), item).and_then(|candidate| {
            candidate
                .effects
                .iter()
                .find(|effect| matches!(effect.params, project::EffectParams::ColorGrade { .. }))
                .map(|effect| effect.id.clone())
        });
        let id = existing.unwrap_or_else(|| definition.kind.to_owned());
        for parameter in definition.params {
            let current =
                read::effect_param_value(self.cx.session.project(), item, &id, parameter.key)
                    .unwrap_or(parameter.default);
            self.effect_scrub_field(ui, item, &id, parameter, current);
        }
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
                        icons::tinted(Icon::ArrowsInLineHorizontal, BUTTON_ICON, self.secondary())
                            .paint_at(
                                ui,
                                Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
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
                        let name = effect_definition(&effect.params)
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
                    if let Some(definition) = effect_definition(&effect.params) {
                        for parameter in definition.params {
                            if let Some(current) = read::effect_param_value(
                                self.cx.session.project(),
                                item,
                                &effect.id,
                                parameter.key,
                            ) {
                                self.effect_scrub_field(ui, item, &effect.id, parameter, current);
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
    pub(super) fn effect_scrub_field(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        parameter: &project::EffectParamDefinition,
        current: f64,
    ) {
        let spec = effect_scrub_spec(parameter);
        ui.add_space(theme::space::MD);
        let label = match parameter.key {
            "intensity" => self.tr(Text::EffectIntensity),
            "brightness" => self.tr(Text::Brightness),
            "contrast" => self.tr(Text::Contrast),
            "saturation" => self.tr(Text::Saturation),
            "hue" => self.tr(Text::Hue),
            _ => parameter.name,
        };
        let caption = ui.label(label);
        let draft_key = format!("effects.{effect}.param.{}", parameter.key);
        ui.horizontal(|ui| {
            let handle_id = format!("effects.{effect}.param.{}.scrub", parameter.key);
            let (rect, handle) = ui.allocate_exact_size(
                Vec2::new(theme::size::CONTROL, theme::size::FIELD),
                egui::Sense::click_and_drag(),
            );
            if handle.hovered() || handle.dragged() {
                ui.painter()
                    .rect_filled(rect, theme::radius::CONTROL, self.control_fill());
            }
            icons::tinted(Icon::ArrowsHorizontal, BUTTON_ICON, self.secondary()).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
            );
            let handle = handle.on_hover_text(self.tr(Text::DragToAdjust));
            self.remember_with_icon(
                handle_id.clone(),
                Some(Icon::ArrowsHorizontal.name()),
                &handle,
            );
            if handle.drag_started() {
                self.start_effect_scrub(item, effect, parameter.key, current);
            }
            if handle.dragged()
                && let Some(mut scrub) = self.state.scrub.take()
            {
                let target = ScrubTarget::Effect {
                    effect: effect.to_owned(),
                    key: parameter.key.to_owned(),
                };
                if scrub.target == target {
                    let steps = f64::from(handle.total_drag_delta().map_or(0.0, |delta| delta.x))
                        / f64::from(spec.pixels_per_step);
                    let value = spec.quantize(scrub.start + steps * spec.step);
                    if value != scrub.last {
                        self.set_effect_param(item, effect, parameter.key, value);
                        scrub.last = value;
                    }
                }
                self.state.scrub = Some(scrub);
            }
            if handle.drag_stopped() {
                self.finish_scrub(true);
            }
            let width = ui.available_width().max(60.0);
            let mut next = self
                .state
                .drafts
                .get(&draft_key)
                .cloned()
                .unwrap_or_else(|| spec.format(current));
            let response = ui
                .add_sized(
                    [width, 26.0],
                    egui::TextEdit::singleline(&mut next)
                        .char_limit(32)
                        .background_color(self.control_fill())
                        .id(Id::unique(&draft_key))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
                .labelled_by(caption.id);
            self.remember(draft_key.clone(), &response);
            if response.has_focus() {
                self.state.focus = Some(draft_key.clone());
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
                    let value = spec.quantize(parsed);
                    self.set_effect_param(item, effect, parameter.key, value);
                }
            }
        });
    }
}
