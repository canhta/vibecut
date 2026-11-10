//! Renders effect parameters.
use super::Panel;
use crate::editor::properties::{
    Action, BUTTON_ICON, ColorTarget, EffectReorder, Error, Icon, Id, ItemId, Rect, ScrubTarget,
    Text, Tick, Vec2, effect_scrub_spec, egui, icons, read, theme,
};

impl Panel<'_, '_> {
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
        match definition.id {
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
        let id = existing.unwrap_or_else(|| definition.id.to_owned());
        let at = self.effect_time(item);
        for parameter in definition.params {
            let current =
                read::effect_param_value_at(self.cx.session.project(), item, &id, parameter.id, at)
                    .or_else(|| parameter.default.scalar())
                    .unwrap_or(0.0);
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
                                        self.effect_scrub_field(
                                            ui, item, &effect.id, parameter, current,
                                        );
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
                                | project::EffectParamKind::Point => {}
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
        ui.add_enabled_ui(editable, |ui| {
            self.color_row(ui, &id, label, target.clone())
        })
        .response
        .on_disabled_hover_text(self.tr(if locked {
            Text::EffectLocked
        } else {
            Text::EffectKeyframeTime
        }));
        let exists = target.rgb_keys(project, true);
        let key_id = format!("{id}.keyframe");
        let rect = Rect::from_min_size(
            egui::pos2(
                row.left() + super::VALUE_COLUMN - theme::size::CONTROL - theme::space::XS,
                row.top(),
            ),
            Vec2::new(theme::size::CONTROL, row.height()),
        );
        let mut strip = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let response = strip
            .add_enabled_ui(!locked && inside, |ui| {
                crate::kit::icon_button(
                    ui,
                    &key_id,
                    Icon::Diamond,
                    self.tr(if exists {
                        Text::RemoveKeyframe
                    } else {
                        Text::AddKeyframe
                    }),
                    exists,
                )
            })
            .inner
            .on_disabled_hover_text(self.tr(if locked {
                Text::EffectLocked
            } else {
                Text::EffectKeyframeTime
            }));
        self.remember_with_icon(key_id, Some(Icon::Diamond.name()), &response);
        if response.clicked() {
            match target.toggle_rgb(project) {
                Ok(command) => {
                    self.actions.extend(self.state.close_picker());
                    self.actions.push(Action::ColorCommit {
                        command,
                        recent: self.cx.preferences.recent_colors.clone(),
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
            _ => option,
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
        let label = match parameter.id {
            "intensity" => self.tr(Text::EffectIntensity),
            "shadow" => self.tr(Text::EffectShadow),
            "brightness" => self.tr(Text::Brightness),
            "contrast" => self.tr(Text::Contrast),
            "saturation" => self.tr(Text::Saturation),
            "hue" => self.tr(Text::Hue),
            _ => parameter.name,
        };
        let caption = ui.label(label);
        let draft_key = format!("effects.{effect}.param.{}", parameter.id);
        ui.horizontal(|ui| {
            let handle_id = format!("effects.{effect}.param.{}.scrub", parameter.id);
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
                self.start_effect_scrub(item, effect, parameter.id, current);
            }
            if handle.dragged()
                && let Some(mut scrub) = self.state.scrub.take()
            {
                let target = ScrubTarget::Effect {
                    effect: effect.to_owned(),
                    key: parameter.id.to_owned(),
                };
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
                theme::size::CONTROL + theme::space::XS
            } else {
                0.0
            };
            let width = (ui.available_width() - reserve).max(60.0);
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
        let label = self.tr(if exists {
            Text::RemoveKeyframe
        } else {
            Text::AddKeyframe
        });
        let id = format!("effects.{effect}.param.{}.keyframe", parameter.id);
        let response = ui
            .add_enabled_ui(inside, |ui| {
                crate::kit::icon_button(ui, &id, Icon::Diamond, label, exists)
            })
            .inner;
        self.remember_with_icon(id, Some(Icon::Diamond.name()), &response);
        if response.clicked() && inside {
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
