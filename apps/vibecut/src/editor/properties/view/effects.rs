//! Renders effect parameters.
use crate::editor::properties::{
    BUTTON_ICON, Editor, EffectReorder, Error, Icon, Id, ItemId, Rect, ScrubTarget, Text, Vec2,
    effect_definition, effect_scrub_spec, egui, icons, theme,
};

impl Editor {
    pub(in crate::editor) fn effects_stack(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let effects = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
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
                            self.effect_reorder = Some(EffectReorder {
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
                            None,
                            Some(effect.enabled),
                            true,
                        );
                        if enabled {
                            self.toggle_effect(item, &effect.id, !effect.enabled);
                        }
                        let remove = self.tr(Text::EffectRemove);
                        if self.icon_only(
                            ui,
                            &format!("effects.remove.{}", effect.id),
                            Icon::Trash,
                            remove,
                            None,
                            None,
                            true,
                        ) {
                            self.remove_effect(item, &effect.id);
                        }
                    });
                    if let Some(definition) = effect_definition(&effect.params) {
                        for parameter in definition.params {
                            if self
                                .effect_param_value(item, &effect.id, parameter.key)
                                .is_some()
                            {
                                self.effect_scrub_field(ui, item, &effect.id, parameter);
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
                        self.effect_reorder
                            .as_ref()
                            .is_none_or(|drag| drag.effect != *id)
                            && rect.contains(position)
                    })
                    .map(|(id, _)| id.clone())
            });
            if let Some(drag) = self.effect_reorder.take()
                && let Some(target) = target
            {
                self.reorder_effect(drag.item, &drag.effect, &target);
            }
        } else if self.effect_reorder.is_some() && ui.input(|input| input.pointer.any_released()) {
            self.effect_reorder = None;
        }
    }
    pub(in crate::editor) fn effect_scrub_field(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        parameter: &project::EffectParamDefinition,
    ) {
        let Some(current) = self.effect_param_value(item, effect, parameter.key) else {
            return;
        };
        let spec = effect_scrub_spec(parameter);
        ui.add_space(theme::space::MD);
        let label = match parameter.key {
            "intensity" => self.tr(Text::EffectIntensity),
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
                && let Some(mut scrub) = self.scrub.take()
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
                        match self.set_effect_param(item, effect, parameter.key, value) {
                            Ok(()) => scrub.last = value,
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                self.scrub = Some(scrub);
            }
            if handle.drag_stopped() {
                self.finish_scrub(true);
            }
            let width = ui.available_width().max(60.0);
            let mut next = self
                .property_drafts
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
                self.property_focus = Some(draft_key.clone());
            }
            if response.changed() {
                self.property_drafts.insert(draft_key.clone(), next);
            }
            let editing =
                response.has_focus() || self.property_focus.as_deref() == Some(draft_key.as_str());
            if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.property_drafts.remove(&draft_key);
                self.property_focus = None;
                response.surrender_focus();
                return;
            }
            let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || response.lost_focus() {
                self.property_focus = None;
                if let Some(draft) = self.property_drafts.remove(&draft_key) {
                    let Ok(parsed) = draft.trim().parse::<f64>() else {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    };
                    if !parsed.is_finite() {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    }
                    let value = spec.quantize(parsed);
                    if let Err(error) = self.set_effect_param(item, effect, parameter.key, value) {
                        self.error = Some(error.to_string());
                    }
                }
            }
        });
    }
}
