//! Renders a clip's playback controls: the Audio tab's volume and fades, and the constant
//! speed rows. Mute lives on the track header and the clip menu.
use super::Panel;
use crate::editor::properties::{
    AUDIO_GAIN_SCRUB, Icon, ItemId, SPEED_SCRUB, ScrubSpec, Text, egui, read, theme,
};

/// The reference's fade range, in seconds.
const FADE_MAX_SECONDS: f64 = 10.0;

impl Panel<'_, '_> {
    /// The Audio section of a video with source audio or an audio clip. Locked
    /// tracks show the same rows disabled.
    pub(super) fn audio_section(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let editable = !read::item_locked(self.cx.session.project(), item);
        self.panel_section(
            ui,
            "inspector.section.audio",
            "inspector.audio",
            Text::Audio,
            true,
            false,
            |panel, ui| {
                ui.add_enabled_ui(editable, |ui| {
                    panel.slider_row(ui, item, AUDIO_GAIN_SCRUB);
                    let fade_in = panel.fade_scrub(item, project::FadeEdge::In);
                    panel.slider_row(ui, item, fade_in);
                    let fade_out = panel.fade_scrub(item, project::FadeEdge::Out);
                    panel.slider_row(ui, item, fade_out);
                });
            },
        );
    }

    /// One fade edge's scrub contract: 0 to 10 s, capped at the room the clip
    /// leaves beside the other fade.
    fn fade_scrub(&self, item: ItemId, edge: project::FadeEdge) -> ScrubSpec {
        let room = read::fade_room_seconds(self.cx.session.project(), item, edge)
            .unwrap_or(FADE_MAX_SECONDS);
        let (field, label) = match edge {
            project::FadeEdge::In => ("audio_fade_in", Text::FadeIn),
            project::FadeEdge::Out => ("audio_fade_out", Text::FadeOut),
        };
        ScrubSpec {
            field,
            label,
            min: 0.0,
            max: room.min(FADE_MAX_SECONDS),
            step: 0.1,
            pixels_per_step: 8.0,
            decimals: 1,
            suffix: "s",
            scale: 1.0,
        }
    }
}

impl Panel<'_, '_> {
    /// Constant speed with reset and the pitch policy; both go through `timeline.set_speed`.
    pub(super) fn speed_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let project = self.cx.session.project();
        let Some((current, maintain_pitch, has_audio)) = read::retime_of(project, item) else {
            return;
        };
        self.scrub_field(ui, item, SPEED_SCRUB);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::SpeedDuration));
        if let Some((source, output)) = read::retime_seconds(project, item) {
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
                current != 1.0,
            );
            if reset {
                self.set_speed(item, 1.0, maintain_pitch);
            }
        });
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::SpeedReverse));
        let mut reversed = read::retime_reversed(project, item).unwrap_or(false);
        let response = value_ui.add(egui::Checkbox::without_text(&mut reversed));
        let name = self.tr(Text::SpeedReverse);
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label(name));
        self.remember("inspector.speed.reverse", &response);
        if response.changed() {
            self.set_reverse(item, reversed);
        }
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
            if response.changed() {
                self.set_speed(item, current, !follows);
            }
        }
    }
}
