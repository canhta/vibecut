//! Renders speed controls.
use crate::editor::properties::{Editor, Icon, ItemId, SPEED_SCRUB, Text, egui, theme};

impl Editor {
    /// Constant speed with reset and the pitch policy; both go through `timeline.set_speed`.
    pub(in crate::editor) fn speed_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some((current, maintain_pitch, has_audio)) = self.retime_of(item) else {
            return;
        };
        self.scrub_field(ui, item, SPEED_SCRUB);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::SpeedDuration));
        if let Some((source, output)) = self.retime_seconds(item) {
            let response = value_ui.label(
                egui::RichText::new(format!("{source:.2}s → {output:.2}s"))
                    .size(theme::type_size::BODY)
                    .color(self.secondary()),
            );
            self.remember("inspector.speed.duration", &response);
        }
        value_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let reset = self.icon_only(
                ui,
                "inspector.speed.reset",
                Icon::ArrowCounterClockwise,
                self.tr(Text::SpeedReset),
                None,
                None,
                current != 1.0,
            );
            if reset && let Err(error) = self.set_speed(item, 1.0, maintain_pitch) {
                self.error = Some(error.to_string());
            }
        });
        if has_audio {
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::ChangePitch));
            // "Change pitch" on lets pitch follow speed, so it is the inverse of maintain_pitch.
            let mut follows = !maintain_pitch;
            let response = value_ui.add(egui::Checkbox::without_text(&mut follows));
            let name = self.tr(Text::ChangePitch);
            response
                .ctx
                .accesskit_node_builder(response.id, |node| node.set_label(name));
            self.remember("inspector.speed.pitch", &response);
            if response.changed()
                && let Err(error) = self.set_speed(item, current, !follows)
            {
                self.error = Some(error.to_string());
            }
        }
    }
}
