//! Renders shared selection properties.

use crate::editor::properties::{
    Editor, Id, SelectionScope, Text, catalog, egui, shared_fields, theme,
};

impl Editor {
    pub(in crate::editor) fn shared_inspector(&mut self, ui: &mut egui::Ui) {
        if self.property_selection != self.selection {
            self.property_drafts.clear();
            self.property_selection = self.selection.clone();
        }
        ui.label(catalog::format_count(
            self.preferences.locale,
            self.selection.len() as u64,
            Text::SelectedCountOne,
            Text::SelectedCountOther,
        ));
        match self.selection_scopes.active(&self.selection) {
            SelectionScope::MaskPoints => {
                ui.label(self.tr(Text::MaskPoints));
            }
            SelectionScope::Keyframes => {
                ui.label(self.tr(Text::Keyframes));
            }
            _ => {}
        }
        let fields = shared_fields(self.session.project(), &self.selection);
        if fields.is_empty() {
            ui.label(self.tr(Text::NoSharedFields));
        }
        self.shared_field_controls(ui, fields);
    }
    pub(in crate::editor) fn shared_field_controls(
        &mut self,
        ui: &mut egui::Ui,
        fields: impl IntoIterator<Item = crate::selection::SharedField>,
    ) {
        for field in fields {
            if field.id == "audio_muted" {
                self.audio_mute_control(ui, &field);
                continue;
            }
            if field.id == "blendMode" {
                ui.add_space(theme::space::LG);
                let caption = ui.label(self.tr(Text::BlendMode));
                let current = field.value.as_ref().and_then(|v| {
                    serde_json::from_value::<project::BlendMode>(serde_json::json!(v)).ok()
                });
                let label = current
                    .map(|m| self.tr(catalog::blend_label(m)))
                    .unwrap_or_else(|| self.tr(Text::EmptyValue));
                let response = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = 26.0;
                        egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                            .selected_text(label)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for mode in project::BlendMode::ALL {
                                    let response = ui.selectable_label(
                                        current == Some(mode),
                                        self.tr(catalog::blend_label(mode)),
                                    );
                                    self.remember(
                                        format!("inspector.blendMode.{}", mode.id()),
                                        &response,
                                    );
                                    if response.clicked() {
                                        if let Err(error) =
                                            self.set_shared_field("blendMode", mode.id())
                                        {
                                            self.error = Some(error.to_string());
                                        }
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .labelled_by(caption.id)
                    })
                    .inner;
                self.remember("inspector.blendMode", &response);
                continue;
            }
            ui.add_space(theme::space::MD);
            if field.id == "text_font" {
                let current = field
                    .value
                    .clone()
                    .unwrap_or_else(|| self.tr(Text::EmptyValue).to_owned());
                let mut chosen = None;
                let caption = ui.label(self.tr(Text::TextFont));
                let response = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = 26.0;
                        egui::ComboBox::from_id_salt(Id::unique("inspector.text_font"))
                            .selected_text(current)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for (family, _) in text::families() {
                                    let selected = field.value.as_deref() == Some(family.as_str());
                                    let response = ui.selectable_label(selected, family.as_str());
                                    self.remember(
                                        format!("inspector.text_font.{family}"),
                                        &response,
                                    );
                                    if response.clicked() {
                                        chosen = Some(family);
                                    }
                                }
                            })
                            .response
                            .labelled_by(caption.id)
                    })
                    .inner;
                self.remember("inspector.text_font", &response);
                if let Some(font) = chosen
                    && let Err(error) = self.set_shared_field("text_font", &font)
                {
                    self.error = Some(error.to_string());
                }
                self.compatible_count(ui, field.compatible);
                continue;
            }
            if field.id == "text_background" {
                let enabled = field.value.as_deref().is_some_and(|value| value != "none");
                let mut checked = enabled;
                let response = ui.add(
                    egui::Checkbox::new(&mut checked, self.tr(Text::TextBackground))
                        .indeterminate(field.value.is_none()),
                );
                self.remember("inspector.text_background", &response);
                if response.changed() {
                    let color = field
                        .value
                        .as_deref()
                        .filter(|value| *value != "none")
                        .unwrap_or("#000000A0");
                    let value = if checked { color } else { "none" };
                    if let Err(error) = self.set_shared_field("text_background", value) {
                        self.error = Some(error.to_string());
                    }
                }
                if enabled {
                    let id = "inspector.text_background_color";
                    ui.label(self.tr(Text::BackgroundColor));
                    let mut value = self
                        .property_drafts
                        .get("text_background_color")
                        .cloned()
                        .or_else(|| field.value.clone())
                        .unwrap_or_default();
                    let response = ui.add_sized(
                        [ui.available_width(), 26.0],
                        egui::TextEdit::singleline(&mut value)
                            .char_limit(64)
                            .background_color(self.control_fill())
                            .id(Id::unique(id))
                            .hint_text(
                                egui::RichText::new(self.tr(Text::EmptyValue))
                                    .color(self.secondary()),
                            ),
                    );
                    self.remember(id, &response);
                    if response.changed() {
                        self.property_drafts
                            .insert("text_background_color".to_owned(), value.clone());
                    }
                    if (response.lost_focus()
                        || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                        && self.property_drafts.contains_key("text_background_color")
                    {
                        let result = self.set_shared_field("text_background_color", &value);
                        self.property_drafts.remove("text_background_color");
                        if let Err(e) = result {
                            self.error = Some(e.to_string());
                        }
                    }
                }
                self.compatible_count(ui, field.compatible);
                continue;
            }
            let (label, id) = match field.id {
                "opacity" => (Text::Opacity, "inspector.opacity"),
                "audio_gain" => (Text::AudioGain, "inspector.audio_gain"),
                "text_content" => (Text::TextContent, "inspector.text_content"),
                "text_size" => (Text::TextSize, "inspector.text_size"),
                "text_background" => (Text::BackgroundColor, "inspector.text_background_color"),
                "text_position_x" => (Text::TextPositionX, "inspector.text_position_x"),
                "text_position_y" => (Text::TextPositionY, "inspector.text_position_y"),
                _ => (Text::TextColor, "inspector.text_color"),
            };
            let caption = ui.label(self.tr(label));
            let mut value = self
                .property_drafts
                .get(field.id)
                .cloned()
                .unwrap_or_else(|| field.value.clone().unwrap_or_default());
            let multiline = field.id == "text_content";
            let response = if multiline {
                ui.add_sized(
                    [ui.available_width(), 48.0],
                    egui::TextEdit::multiline(&mut value)
                        .char_limit(4096)
                        .background_color(self.control_fill())
                        .id(Id::unique(id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
            } else {
                ui.add_sized(
                    [ui.available_width(), 26.0],
                    egui::TextEdit::singleline(&mut value)
                        .char_limit(64)
                        .background_color(self.control_fill())
                        .id(Id::unique(id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
            }
            .labelled_by(caption.id);
            self.remember(id, &response);
            if response.changed() {
                self.property_drafts
                    .insert(field.id.to_owned(), value.clone());
            }
            self.compatible_count(ui, field.compatible);
            let commit_key = response.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Enter) && (!multiline || i.modifiers.command)
                });
            if (response.lost_focus() || commit_key) && self.property_drafts.contains_key(field.id)
            {
                let result = self.set_shared_field(field.id, &value);
                self.property_drafts.remove(field.id);
                if let Err(e) = result {
                    self.error = Some(e.to_string());
                }
            }
        }
    }
    pub(in crate::editor) fn audio_mute_control(
        &mut self,
        ui: &mut egui::Ui,
        field: &crate::selection::SharedField,
    ) {
        ui.add_space(theme::space::LG);
        let mut muted = field.value.as_deref() == Some("true");
        let response = ui.add(
            egui::Checkbox::new(&mut muted, self.tr(Text::AudioMuted))
                .indeterminate(field.value.is_none()),
        );
        self.remember("inspector.audio_muted", &response);
        if response.changed()
            && let Err(error) = self.set_shared_field("audio_muted", &muted.to_string())
        {
            self.error = Some(error.to_string());
        }
        if self.selection.len() > 1 {
            self.compatible_count(ui, field.compatible);
        }
    }
    pub(in crate::editor) fn compatible_count(&self, ui: &mut egui::Ui, compatible: usize) {
        let compatible = catalog::number(self.preferences.locale, compatible as u64);
        let total = catalog::number(self.preferences.locale, self.selection.len() as u64);
        ui.label(
            egui::RichText::new(catalog::format(
                self.preferences.locale,
                Text::CompatibleCount,
                &[("compatible", &compatible), ("total", &total)],
            ))
            .small()
            .color(self.secondary()),
        );
    }
}
