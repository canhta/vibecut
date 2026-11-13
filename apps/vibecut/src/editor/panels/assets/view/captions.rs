//! Renders the Captions tab's sub-navigation pages.
use super::super::CaptionsSection;
use super::{Color32, Text, View, egui, theme};

impl View<'_> {
    pub(super) fn captions_nav(&mut self, ui: &mut egui::Ui) {
        for (section, label, id) in [
            (
                CaptionsSection::Auto,
                Text::SubAutoCaptions,
                "captions.section.auto",
            ),
            (
                CaptionsSection::Add,
                Text::SubAddCaptions,
                "captions.section.add",
            ),
            (
                CaptionsSection::Translate,
                Text::SubTranslate,
                "captions.section.translate",
            ),
        ] {
            let selected = self.state.captions_section == section;
            let response = ui.add_sized(
                [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
                egui::Button::new(
                    egui::RichText::new(self.tr(label))
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL)
                        .color(if selected {
                            theme::accent(self.cx.preferences.dark)
                        } else {
                            theme::text(self.cx.preferences.dark)
                        }),
                )
                .fill(if selected {
                    theme::hover(self.cx.preferences.dark)
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(egui::Stroke::NONE)
                .right_text(""),
            );
            self.remember(id, &response, None);
            if response.clicked() && !selected {
                self.state.captions_section = section;
            }
        }
    }
}
