//! Renders speed controls.
use crate::editor::properties::{Editor, Icon, ItemId, SPEED_SCRUB, Text, egui, theme};

impl Editor {
    /// Constant speed with reset and the pitch policy; both go through `timeline.set_speed`.
    pub(in crate::editor) fn speed_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some((current, maintain_pitch, has_audio)) = self.retime_of(item) else {
            return;
        };
        self.scrub_field(ui, item, SPEED_SCRUB);
        ui.add_space(theme::space::MD);
        if let Some((source, output)) = self.retime_seconds(item) {
            // The reference shows the source duration and the retimed duration.
            ui.horizontal(|ui| {
                ui.label(self.tr(Text::SpeedDuration));
                let response = ui.label(
                    egui::RichText::new(format!("{source:.2}s → {output:.2}s"))
                        .color(self.secondary()),
                );
                self.remember("inspector.speed.duration", &response);
            });
            ui.add_space(theme::space::MD);
        }
        ui.horizontal(|ui| {
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
            ui.add_space(theme::space::MD);
            // "Change pitch" on lets pitch follow speed, so it is the inverse of maintain_pitch.
            let mut follows = !maintain_pitch;
            let response = ui.add(egui::Checkbox::new(
                &mut follows,
                self.tr(Text::ChangePitch),
            ));
            self.remember("inspector.speed.pitch", &response);
            if response.changed()
                && let Err(error) = self.set_speed(item, current, !follows)
            {
                self.error = Some(error.to_string());
            }
        }
    }
}
