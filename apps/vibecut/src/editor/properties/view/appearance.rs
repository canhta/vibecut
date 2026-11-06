//! Renders appearance controls.
use crate::editor::properties::{
    BUTTON_ICON, Color32, ColorTarget, Editor, Icon, Id, ItemId, ItemParams, Text, catalog, egui,
    icons, theme,
};

impl Editor {
    pub(in crate::editor) fn blend_control(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(current) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .map(|item| item.blend_mode)
        else {
            return;
        };
        ui.add_space(theme::space::LG);
        let caption = ui.label(self.tr(Text::BlendMode));
        let label = self.tr(catalog::blend_label(current));
        let response = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = 26.0;
                egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                    .selected_text(label)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for mode in project::BlendMode::ALL {
                            let response = ui.selectable_label(
                                current == mode,
                                self.tr(catalog::blend_label(mode)),
                            );
                            self.remember(format!("inspector.blendMode.{}", mode.id()), &response);
                            if response.clicked() {
                                if let Err(error) =
                                    self.set_item_field(item, "blendMode", mode.id())
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
    }
    pub(in crate::editor) fn color_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: Text,
        target: ColorTarget,
    ) {
        let Ok(color) = target.color(self.session.project()) else {
            return;
        };
        let [r, g, b, a] = color.0;
        let response = ui.add(egui::Button::image_and_text(
            icons::tinted(
                Icon::CircleHalf,
                BUTTON_ICON,
                Color32::from_rgba_unmultiplied(r, g, b, a),
            ),
            self.tr(label),
        ));
        self.remember_with_icon(id, Some(Icon::CircleHalf.name()), &response);
        if response.clicked()
            && let Err(error) = self.open_color_picker(target)
        {
            self.error = Some(error.to_string());
        }
    }
    pub(in crate::editor) fn text_color_field(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(value) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .and_then(|item| match &item.content {
                ItemParams::Text(params) => {
                    let [r, g, b, a] = params.color.0;
                    Some(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"))
                }
                _ => None,
            })
        else {
            return;
        };
        ui.add_space(theme::space::LG);
        let caption = ui.label(self.tr(Text::TextColor));
        let draft = self
            .property_drafts
            .get("text_color")
            .cloned()
            .unwrap_or(value);
        let mut next = draft;
        let response = ui
            .add_sized(
                [ui.available_width(), 26.0],
                egui::TextEdit::singleline(&mut next)
                    .char_limit(64)
                    .background_color(self.control_fill())
                    .id(Id::unique("inspector.text_color"))
                    .hint_text(
                        egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                    ),
            )
            .labelled_by(caption.id);
        self.remember("inspector.text_color", &response);
        if response.has_focus() {
            self.property_focus = Some("text_color".to_owned());
        }
        if response.changed() {
            self.property_drafts
                .insert("text_color".to_owned(), next.clone());
        }
        let editing = response.has_focus() || self.property_focus.as_deref() == Some("text_color");
        if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.property_drafts.remove("text_color");
            self.property_focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.property_focus = None;
            if let Some(draft) = self.property_drafts.remove("text_color")
                && let Err(error) = self.set_item_field(item, "text_color", &draft)
            {
                self.error = Some(error.to_string());
            }
        }
    }
}
