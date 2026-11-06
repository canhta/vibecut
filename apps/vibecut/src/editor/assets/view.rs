//! Renders the assets area.
mod browser;
mod effects;
mod feedback;

use super::{AssetsTab, Color32, Editor, Rect, Vec2, egui, theme};

/// The assets panel: a scrollable vertical tab bar and the selected view.
pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let bar = egui::vec2(36.0, height);
        let mut clicked = None;
        let mut shown = None;
        ui.allocate_ui_with_layout(bar, egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(theme::space::SM);
            let output = egui::ScrollArea::vertical()
                .id_salt("assets.tabs")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for tab in AssetsTab::ALL {
                        let id = format!("assets.tab.{}", tab.id());
                        let active = editor.preferences.assets_tab == tab;
                        let label = editor.tr(tab.label());
                        if editor.icon_only(ui, &id, tab.icon(), label, None, Some(active), true) {
                            clicked = Some(tab);
                        }
                    }
                });
            shown = Some(output);
        });
        if let Some(tab) = clicked
            && let Err(error) = editor.remember_assets_tab(tab)
        {
            editor.error = Some(error.to_string());
        }
        if let Some(output) = shown {
            let dark = editor.preferences.dark;
            scroll_fades(ui, &output, dark);
        }
        let separator = theme::separator(editor.preferences.dark);
        ui.painter().line_segment(
            [
                egui::pos2(ui.cursor().min.x, ui.cursor().min.y),
                egui::pos2(ui.cursor().min.x, ui.cursor().min.y + height),
            ],
            egui::Stroke::new(1.0, separator),
        );
        ui.add_space(theme::space::LG);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| match editor.preferences.assets_tab {
                AssetsTab::Media => editor.media_view(ui),
                AssetsTab::Effects => editor.effects_view(ui),
                AssetsTab::Captions => crate::editor::captions::show(editor, ui),
                tab => editor.assets_stub(ui, tab),
            },
        );
    });
}

/// Paint top/bottom fades over a scroll view whose content continues past the edge.
fn scroll_fades<F>(ui: &egui::Ui, output: &egui::scroll_area::ScrollAreaOutput<F>, dark: bool) {
    const STRIPS: usize = 6;
    const FADE_HEIGHT: f32 = 24.0;
    let view = output.inner_rect;
    let offset = output.state.clamped_offset().y;
    let show_top = offset > 0.5;
    let show_bottom = offset + view.height() < output.content_size.y - 0.5;
    if !show_top && !show_bottom {
        return;
    }
    let base = theme::panel(dark);
    let strip = (FADE_HEIGHT / STRIPS as f32).min(view.height() / 2.0);
    for index in 0..STRIPS {
        let alpha = (255.0 * (1.0 - index as f32 / STRIPS as f32)).round() as u8;
        let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        if show_top {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    egui::pos2(view.left(), view.top() + index as f32 * strip),
                    Vec2::new(view.width(), strip),
                ),
                0.0,
                color,
            );
        }
        if show_bottom {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    egui::pos2(view.left(), view.bottom() - (index as f32 + 1.0) * strip),
                    Vec2::new(view.width(), strip),
                ),
                0.0,
                color,
            );
        }
    }
}

use super::Text;

impl Editor {
    pub(in crate::editor) fn assets_stub(&mut self, ui: &mut egui::Ui, tab: AssetsTab) {
        ui.label(
            egui::RichText::new(self.tr(tab.label()))
                .size(16.0)
                .strong(),
        );
        ui.add_space(theme::space::MD);
        ui.label(
            egui::RichText::new(self.tr(Text::PanelStub))
                .small()
                .color(self.secondary()),
        );
    }
}

mod deletion;
