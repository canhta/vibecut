//! Renders shared selection properties.
use crate::i18n;
use crate::kit;

use super::Panel;
use crate::editor::properties::{Id, SelectionScope, Text, egui, shared_fields, theme};

impl Panel<'_, '_> {
    pub(super) fn shared_inspector(&mut self, ui: &mut egui::Ui) {
        if self.state.selection != *self.cx.selection() {
            self.state.drafts.clear();
            self.state.font_picker.close();
            self.state.selection = self.cx.selection().clone();
        }
        ui.label(i18n::format_count(
            self.cx.locale,
            self.cx.selection().len() as u64,
            Text::SelectedCountOne,
            Text::SelectedCountOther,
        ));
        match self.cx.scope {
            SelectionScope::MaskPoints => {
                ui.label(self.tr(Text::MaskPoints));
            }
            SelectionScope::Keyframes => {
                ui.label(self.tr(Text::Keyframes));
            }
            _ => {}
        }
        let fields = shared_fields(self.cx.session.project(), self.cx.selection());
        if fields.is_empty() {
            ui.label(self.tr(Text::NoSharedFields));
        }
        self.shared_field_controls(ui, fields);
        self.text_style_rows(ui);
        let items = self.cx.selection().iter().copied().collect::<Vec<_>>();
        self.arrange_controls(ui, &items);
    }
    pub(super) fn shared_field_controls(
        &mut self,
        ui: &mut egui::Ui,
        fields: impl IntoIterator<Item = crate::editor::properties::SharedField>,
    ) {
        for field in fields {
            // The text style rows own these: colour parts, toggles and glow.
            if matches!(
                field.id,
                "text_background" | "text_stroke" | "text_shadow" | "text_glow"
            ) {
                continue;
            }
            if field.id == "muted" {
                self.audio_mute_control(ui, &field);
                continue;
            }
            if field.id == "blendMode" {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::BlendMode));
                let current = field.value.as_ref().and_then(|v| {
                    serde_json::from_value::<project::BlendMode>(serde_json::json!(v)).ok()
                });
                let label = current
                    .map(|m| self.tr(i18n::blend_label(m)))
                    .unwrap_or_else(|| self.tr(Text::EmptyValue));
                let response = value_ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = theme::size::FIELD;
                        egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                            .selected_text(label)
                            .width(ui.available_width().min(150.0))
                            .show_ui(ui, |ui| {
                                for mode in project::BlendMode::ALL {
                                    let response = ui.selectable_label(
                                        current == Some(mode),
                                        self.tr(i18n::blend_label(mode)),
                                    );
                                    self.remember(
                                        format!("inspector.blendMode.{}", mode.id()),
                                        &response,
                                    );
                                    if response.clicked() {
                                        self.set_shared_field("blendMode", mode.id());
                                        ui.close();
                                    }
                                }
                            })
                            .response
                    })
                    .inner;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(Text::BlendMode))
                });
                self.remember("inspector.blendMode", &response);
                continue;
            }
            if field.id == "text_font" {
                let current = field.value.clone();
                let label = current
                    .clone()
                    .unwrap_or_else(|| self.tr(Text::EmptyValue).to_owned());
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::TextFont));
                let dark = self.cx.dark;
                // A left-aligned field: the family in its own face, the popover on click.
                let (rect, response) = value_ui.allocate_exact_size(
                    egui::vec2(value_ui.available_width(), theme::size::FIELD),
                    egui::Sense::click(),
                );
                let fill = if response.hovered() {
                    theme::hover(dark)
                } else {
                    theme::field(dark)
                };
                let face = current
                    .as_deref()
                    .map_or(egui::FontFamily::Proportional, |family| {
                        crate::editor::properties::font_picker::label_family(value_ui.ctx(), family)
                    });
                value_ui
                    .painter()
                    .rect_filled(rect, theme::radius::FIELD, fill);
                value_ui.painter().with_clip_rect(rect).text(
                    egui::pos2(rect.left() + theme::space::MD, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::new(theme::type_size::BODY, face),
                    theme::text(dark),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(Text::TextFont))
                });
                self.remember("inspector.text_font", &response);
                if response.clicked() {
                    let anchor = response.rect.left_bottom() + egui::vec2(0.0, theme::space::XS);
                    self.state.font_picker.open(anchor, current);
                }
                self.compatible_count(ui, field.compatible);
                continue;
            }
            let (label, id) = match field.id {
                "opacity" => (Text::Opacity, "inspector.opacity"),
                "volume" => (Text::AudioGain, "inspector.volume"),
                "text_content" => (Text::TextContent, "inspector.text_content"),
                "text_size" => (Text::TextSize, "inspector.text_size"),
                _ => (Text::TextColor, "inspector.text_color"),
            };
            let mut value = self
                .state
                .drafts
                .get(field.id)
                .cloned()
                .unwrap_or_else(|| field.value.clone().unwrap_or_default());
            let multiline = field.id == "text_content";
            let response = if multiline {
                let caption =
                    ui.label(egui::RichText::new(self.tr(label)).size(theme::type_size::BODY));
                ui.add_space(theme::space::XS);
                ui.add_sized(
                    [ui.available_width(), 56.0],
                    egui::TextEdit::multiline(&mut value)
                        .char_limit(4096)
                        .background_color(self.control_fill())
                        .id(Id::unique(id)),
                )
                .labelled_by(caption.id)
            } else {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(label));
                let width = value_ui.available_width().min(150.0);
                kit::value_field(&mut value_ui, id, &mut value, width, None, self.tr(label))
            };
            self.remember(id, &response);
            if response.changed() {
                self.state.drafts.insert(field.id.to_owned(), value.clone());
            }
            self.compatible_count(ui, field.compatible);
            let commit_key = response.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Enter) && (!multiline || i.modifiers.command)
                });
            if (response.lost_focus() || commit_key) && self.state.drafts.contains_key(field.id) {
                self.set_shared_field(field.id, &value);
                self.state.drafts.remove(field.id);
            }
        }
    }
    /// The Background checkbox: on adds a box in the last colour (black 63% when none), off
    /// removes it. Its colour is a separate row, so a box has one colour control.
    pub(super) fn text_background_toggle(
        &mut self,
        ui: &mut egui::Ui,
        field: &crate::editor::properties::SharedField,
    ) {
        let mut checked = field.value.as_deref().is_some_and(|value| value != "none");
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::TextBackground));
        let response = value_ui
            .add(egui::Checkbox::without_text(&mut checked).indeterminate(field.value.is_none()));
        let name = self.tr(Text::TextBackground);
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label(name));
        self.remember("inspector.text_background", &response);
        if response.changed() {
            let color = field
                .value
                .as_deref()
                .filter(|value| *value != "none")
                .unwrap_or("#000000A0");
            let value = if checked { color } else { "none" };
            self.set_shared_field("text_background", value);
        }
    }

    pub(super) fn audio_mute_control(
        &mut self,
        ui: &mut egui::Ui,
        field: &crate::editor::properties::SharedField,
    ) {
        ui.add_space(theme::space::LG);
        let mut muted = field.value.as_deref() == Some("true");
        let response = ui.add(
            egui::Checkbox::new(&mut muted, self.tr(Text::AudioMuted))
                .indeterminate(field.value.is_none()),
        );
        self.remember("inspector.muted", &response);
        if response.changed() {
            self.set_shared_field("muted", &muted.to_string());
        }
        if self.cx.selection().len() > 1 {
            self.compatible_count(ui, field.compatible);
        }
    }
    pub(super) fn compatible_count(&self, ui: &mut egui::Ui, compatible: usize) {
        if self.cx.selection().len() < 2 {
            return;
        }
        let compatible = i18n::number(self.cx.locale, compatible as u64);
        let total = i18n::number(self.cx.locale, self.cx.selection().len() as u64);
        ui.label(
            egui::RichText::new(i18n::format(
                self.cx.locale,
                Text::CompatibleCount,
                &[("compatible", &compatible), ("total", &total)],
            ))
            .small()
            .color(self.secondary()),
        );
    }
}
