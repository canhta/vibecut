//! Renders the Audio tab: Volume, then Fade in and Fade out. Each is a slider
//! with a field; mute lives on the track header and the clip menu.
use super::Panel;
use crate::editor::properties::{AUDIO_GAIN_SCRUB, ItemId, ScrubSpec, Text, egui, read};

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
        }
    }
}
