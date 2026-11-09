//! Renders asset feedback.

use crate::editor::assets::{Editor, Icon, Id, Rect, Text, Vec2, egui, theme};
use crate::i18n;

impl Editor {
    /// Non-modal import overlay over the media panel. Import controls are
    /// already disabled while the batch runs; cancel remains available.
    pub(in crate::editor) fn import_overlay(&mut self, ctx: &egui::Context) {
        if !self.importing() {
            return;
        }
        let rect = self
            .panels
            .get("media")
            .copied()
            .unwrap_or_else(|| Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 200.0)));
        let percent = (self.import_fraction() * 100.0).clamp(0.0, 100.0) as u64;
        let width = (rect.width() - 24.0).max(48.0);
        egui::Area::new(Id::unique("import.overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min + Vec2::new(12.0, theme::size::TAB_STRIP + 12.0))
            .show(ctx, |ui| {
                ui.set_width(width);
                egui::Frame::new()
                    .fill(self.panel_fill())
                    .stroke(egui::Stroke::new(1.0, self.border()))
                    .corner_radius(theme::radius::CONTROL)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width((width - 24.0).max(24.0));
                        ui.horizontal(|ui| {
                            let value = i18n::number(self.preferences.locale, percent);
                            let status = if width < 240.0 {
                                format!("{value}%")
                            } else {
                                i18n::format(
                                    self.preferences.locale,
                                    Text::ProcessingPercent,
                                    &[("status", self.tr(Text::Processing)), ("percent", &value)],
                                )
                            };
                            ui.label(egui::RichText::new(status).size(theme::type_size::BODY));
                            if self.icon_only(
                                ui,
                                "import.cancel",
                                Icon::X,
                                self.tr(Text::Cancel),
                                None,
                                None,
                                true,
                            ) {
                                self.action("import.cancel");
                            }
                        });
                        let (bar, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 2.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(bar, 0.0, self.control_fill());
                        ui.painter().rect_filled(
                            Rect::from_min_size(
                                bar.min,
                                Vec2::new(bar.width() * percent as f32 / 100.0, bar.height()),
                            ),
                            0.0,
                            self.accent(),
                        );
                        ui.label(
                            egui::RichText::new(i18n::format(
                                self.preferences.locale,
                                Text::ImportProgress,
                                &[
                                    (
                                        "done",
                                        &i18n::number(
                                            self.preferences.locale,
                                            self.import_completed as u64,
                                        ),
                                    ),
                                    (
                                        "total",
                                        &i18n::number(
                                            self.preferences.locale,
                                            self.import_total as u64,
                                        ),
                                    ),
                                ],
                            ))
                            .size(theme::type_size::BODY)
                            .color(self.secondary()),
                        );
                    });
            });
    }
    pub(in crate::editor) fn drop_overlay(&self, ctx: &egui::Context, hovering: bool) {
        if !hovering || self.importing() {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(Id::unique("import.drop"))
            .order(egui::Order::Tooltip)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_width(screen.width());
                ui.set_height(screen.height());
                ui.painter()
                    .rect_filled(screen, theme::radius::PANEL, theme::scrim());
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(screen.shrink(48.0)).layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    ),
                    |ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::DropMedia))
                                .size(18.0)
                                .color(theme::on_scrim()),
                        );
                    },
                );
            });
    }
    pub(in crate::editor) fn toast(&self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        let text = toast.text.clone();
        let detail = toast.detail.clone();
        egui::Area::new(Id::unique("import.toast"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.panel_fill())
                    .stroke(egui::Stroke::new(1.0, self.border()))
                    .corner_radius(12)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_max_width(320.0);
                        let response = ui.label(
                            egui::RichText::new(text)
                                .size(theme::type_size::CHROME)
                                .strong(),
                        );
                        response.ctx.accesskit_node_builder(response.id, |node| {
                            node.set_author_id("import.toast");
                        });
                        if let Some(detail) = detail {
                            ui.label(egui::RichText::new(detail).small().color(self.secondary()));
                        }
                    });
            });
    }
}
