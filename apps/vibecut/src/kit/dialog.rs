//! Renders the frame, title and footer buttons a modal dialog shares.
//!
//! Callers own the content, the width, the automation IDs they record and the
//! decision a button stands for.

use super::widgets::author;
use super::{radius, size, space, type_size};
use eframe::egui::{self, Response, RichText, Ui, Vec2};

/// The modal frame: window fill, 12 px corners and the 24 px inset
/// the export capture measures.
pub(crate) fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.global_style())
        .corner_radius(radius::DIALOG)
        .inner_margin(space::DIALOG)
}

/// The dialog heading, 18 px semibold.
pub(crate) fn dialog_title(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).strong().size(type_size::DIALOG_TITLE))
}

/// A 90x34 footer button; `primary` fills it with the accent. The caller wraps
/// it in `add_enabled_ui` for a disabled state.
pub(crate) fn dialog_button(ui: &mut Ui, id: &str, label: &str, primary: bool) -> Response {
    let dark = ui.visuals().dark_mode;
    let button = if primary {
        egui::Button::new(RichText::new(label).color(super::on_accent(dark)))
            .fill(super::accent(dark))
    } else {
        egui::Button::new(label)
    }
    .min_size(Vec2::new(
        size::DIALOG_BUTTON_WIDTH,
        size::DIALOG_BUTTON_HEIGHT,
    ));
    let response = ui.add(button);
    author(&response, id);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn footer_buttons_click_only_when_enabled_and_keep_their_size() {
        for primary in [false, true] {
            for enabled in [false, true] {
                let mut harness = Harness::builder()
                    .with_context_setup(crate::kit::install_fonts)
                    .build_ui_state(
                        move |ui, clicks: &mut u32| {
                            ui.add_enabled_ui(enabled, |ui| {
                                if dialog_button(ui, "dialog.ok", "Save", primary).clicked() {
                                    *clicks += 1;
                                }
                            });
                        },
                        0,
                    );
                harness.run();
                let button = harness.get_by_label("Save");
                let rect = button.rect();
                assert!(
                    rect.width() >= size::DIALOG_BUTTON_WIDTH
                        && (rect.height() - size::DIALOG_BUTTON_HEIGHT).abs() < 0.5,
                    "{primary}/{enabled}: {rect:?}"
                );
                button.click();
                harness.run();
                assert_eq!(*harness.state(), u32::from(enabled), "{primary}/{enabled}");
            }
        }
    }
}
