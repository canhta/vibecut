//! Renders effect cards in the measured four-column grid.
use super::{
    Action, Align2, AssetsTab, EffectRow, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons,
    theme,
};

impl View<'_> {
    pub(super) fn effects(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.search(ui, area.width());
        ui.painter().text(
            area.min + Vec2::new(0.0, 36.0),
            Align2::LEFT_TOP,
            self.tr(Text::AssetsAll),
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
        let query = self
            .state
            .searches
            .get(&AssetsTab::Effects)
            .map(|query| query.to_lowercase())
            .unwrap_or_default();
        let cx = self.cx;
        let rows: Vec<_> = cx
            .effects
            .iter()
            .filter(|row| row.name.to_lowercase().contains(&query))
            .collect();
        let width = theme::size::EFFECT_CARD;
        let columns = ((area.width() + theme::space::GRID) / (width + theme::space::GRID))
            .floor()
            .max(1.0) as usize;
        let cards = Rect::from_min_max(area.min + Vec2::new(0.0, 64.0), area.max);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("effects.results")
                .max_rect(cards),
            |ui| {
                if rows.is_empty() {
                    self.empty(ui, Icon::FilmStrip, Text::AssetsNoMatches);
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("effects.tiles")
                    .show(ui, |ui| {
                        let (content, _) = ui.allocate_exact_size(
                            Vec2::new(
                                ui.available_width(),
                                rows.len().div_ceil(columns) as f32 * theme::size::EFFECT_PITCH,
                            ),
                            egui::Sense::hover(),
                        );
                        for (index, row) in rows.iter().enumerate() {
                            let rect = Rect::from_min_size(
                                content.min
                                    + Vec2::new(
                                        (index % columns) as f32 * (width + theme::space::GRID),
                                        (index / columns) as f32 * theme::size::EFFECT_PITCH,
                                    ),
                                Vec2::new(width, width + 22.0),
                            );
                            self.effect(ui, row, rect);
                        }
                    });
            },
        );
    }
    fn effect(&mut self, ui: &mut egui::Ui, row: &EffectRow, rect: Rect) {
        use crate::effect_previews::EffectPreviewStatus;
        let response = ui.interact(
            rect,
            Id::unique(("effects.tile", row.kind)),
            egui::Sense::click_and_drag(),
        );
        self.remember(format!("effects.tile.{}", row.kind), &response, None);
        let status = match row.status {
            Some(EffectPreviewStatus::Ready) => "ready",
            Some(EffectPreviewStatus::Pending) => "pending",
            _ => "failed",
        };
        self.remember(
            format!("effects.tile.{}.{status}", row.kind),
            &response,
            None,
        );
        let response = response.on_hover_text(match row.status {
            Some(EffectPreviewStatus::Pending) => self.tr(Text::EffectPreviewPending),
            Some(EffectPreviewStatus::Failed) | None => self.tr(Text::EffectPreviewFailed),
            Some(EffectPreviewStatus::Ready) => row.name,
        });
        if response.drag_started() {
            self.actions.push(Action::EffectDrag(row.kind));
        }
        let preview = Rect::from_min_size(
            rect.min,
            Vec2::new(theme::size::EFFECT_CARD, theme::size::EFFECT_THUMB_HEIGHT),
        );
        ui.painter().rect_filled(
            preview,
            theme::radius::UPLOAD,
            theme::field(self.cx.preferences.dark),
        );
        if let Some(texture) = &row.texture {
            egui::Image::new((texture.id(), preview.size()))
                .corner_radius(theme::radius::UPLOAD)
                .paint_at(ui, preview);
        } else {
            let glyph = if status == "pending" {
                Icon::ArrowClockwise
            } else {
                Icon::FilmStrip
            };
            icons::tinted(glyph, 24.0, theme::secondary(self.cx.preferences.dark)).paint_at(
                ui,
                Rect::from_center_size(preview.center(), Vec2::splat(24.0)),
            );
        }
        if status == "failed" {
            let retry = Rect::from_min_size(
                preview.left_bottom() - Vec2::new(0.0, 24.0),
                Vec2::splat(24.0),
            );
            let response = ui
                .interact(
                    retry,
                    Id::unique(("effects.preview.retry", row.kind)),
                    egui::Sense::click(),
                )
                .on_hover_text(self.tr(Text::EffectPreviewRetry));
            self.remember(
                format!("effects.preview.retry.{}", row.kind),
                &response,
                Some(Icon::ArrowClockwise),
            );
            icons::tinted(
                Icon::ArrowClockwise,
                16.0,
                theme::text(self.cx.preferences.dark),
            )
            .paint_at(ui, retry.shrink(4.0));
            if response.clicked() {
                self.actions.push(Action::RetryEffect(row.kind));
            }
        }
        let plus = Rect::from_min_size(
            preview.right_bottom() - Vec2::splat(24.0),
            Vec2::splat(24.0),
        );
        let response = ui
            .interact(
                plus,
                Id::unique(("effects.add", row.kind)),
                egui::Sense::click(),
            )
            .on_hover_text(self.tr(Text::EffectAdd));
        self.remember(
            format!("effects.add.{}", row.kind),
            &response,
            Some(Icon::Plus),
        );
        ui.painter()
            .circle_filled(plus.center(), 8.0, theme::scrim());
        icons::tinted(Icon::Plus, 12.0, theme::on_scrim()).paint_at(ui, plus.shrink(6.0));
        if response.clicked() {
            self.actions.push(Action::AddEffect(row.kind));
        }
        ui.painter().with_clip_rect(rect).text(
            preview.left_bottom() + Vec2::new(0.0, theme::space::XS),
            Align2::LEFT_TOP,
            row.name,
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
    }
}
