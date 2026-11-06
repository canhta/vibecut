//! Small Home-only drawing helpers: automation ids, fonts, durations and quiet buttons.

use crate::{
    egui,
    kit::{self, Icon},
};
use std::sync::Arc;
use time::Tick;

/// One line of text cut with an ellipsis at `max_width`, ready to paint.
pub(super) fn line(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    ui.painter().layout_job(job)
}

/// Stable automation id on a widget's accessibility node.
pub(super) fn set_author_id(response: &egui::Response, id: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
}

/// The bundled semibold face at an explicit size.
pub(super) fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("Inter-SemiBold".into()))
}

/// `m:ss` below one hour, `h:mm:ss` from one hour; negative ticks read as zero.
pub(super) fn display_time(at: Tick) -> String {
    let seconds = at.seconds().max(0.0) as u64;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Flat icon-sized menu buttons: transparent until hovered, 4 px corners.
pub(super) fn quiet_buttons(ui: &mut egui::Ui) {
    let dark = ui.visuals().dark_mode;
    let spacing = ui.spacing_mut();
    spacing.button_padding = egui::vec2(4.0, 4.0);
    spacing.interact_size = egui::Vec2::splat(kit::size::CONTROL);
    let radius = egui::CornerRadius::same(kit::radius::CONTROL);
    let widgets = &mut ui.visuals_mut().widgets;
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = radius;
        state.bg_stroke = egui::Stroke::NONE;
    }
    widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    widgets.hovered.weak_bg_fill = kit::hover(dark);
    widgets.active.weak_bg_fill = kit::hover(dark);
    widgets.open.weak_bg_fill = kit::hover(dark);
}

/// An icon-and-label button for dialog footers.
pub(super) fn action_button(
    ui: &mut egui::Ui,
    id: &str,
    icon: Icon,
    label: &str,
    enabled: bool,
) -> bool {
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let response = ui
        .push_id(id, |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::image_and_text(
                    kit::tinted(icon, kit::icon::DENSE, color),
                    egui::RichText::new(label).color(color),
                )
                .min_size(egui::vec2(28.0, 26.0)),
            )
        })
        .inner;
    set_author_id(&response, id);
    response.clicked()
}

/// A menu row with a leading icon.
pub(super) fn menu_action(ui: &mut egui::Ui, id: &str, icon: Icon, label: &str) -> bool {
    let response = ui
        .push_id(id, |ui| {
            ui.add(egui::Button::image_and_text(
                kit::tinted(icon, kit::icon::DENSE, ui.visuals().text_color()),
                label,
            ))
        })
        .inner;
    set_author_id(&response, id);
    response.clicked()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn duration_display_is_stable_for_boundaries() {
        assert_eq!(display_time(Tick::ZERO), "00:00");
        assert_eq!(display_time(Tick(-1)), "00:00");
        assert_eq!(
            display_time(Tick::from_seconds_f64(65.9, time::Rounding::Nearest).unwrap()),
            "01:05"
        );
        assert_eq!(
            display_time(Tick::from_seconds_f64(3661.5, time::Rounding::Nearest).unwrap()),
            "1:01:01"
        );
    }
}
