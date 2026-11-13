//! Renders the Filters tab: the shelf navigation and the look tiles.
//!
//! The 222 star and the Favorites entry attach here once 222 lands: the star sits on the tile's
//! top-left corner (family `filters`, key `FilterRow::id`) and Favorites heads `filters_nav`.
use super::{
    Action, Align2, AssetsTab, FilterRow, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons,
    theme,
};

fn shelf_label(shelf: project::FilterShelf) -> Text {
    match shelf {
        project::FilterShelf::Featured => Text::FilterShelfFeatured,
        project::FilterShelf::Movies => Text::FilterShelfMovies,
        project::FilterShelf::Retro => Text::FilterShelfRetro,
        project::FilterShelf::Night => Text::FilterShelfNight,
        project::FilterShelf::Mono => Text::FilterShelfMono,
        project::FilterShelf::Tools => Text::FilterShelfTools,
        project::FilterShelf::Life => Text::FilterShelfLife,
    }
}

impl View<'_> {
    pub(super) fn filters_nav(&mut self, ui: &mut egui::Ui) {
        for shelf in project::FilterShelf::ALL {
            let selected = self.state.filter_shelf == shelf;
            let label = shelf_label(shelf);
            let response = ui
                .add_sized(
                    [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
                    egui::Button::new(
                        egui::RichText::new(self.tr(label))
                            .size(theme::type_size::CHROME)
                            .color(if selected {
                                theme::accent(self.cx.preferences.dark)
                            } else {
                                theme::text(self.cx.preferences.dark)
                            }),
                    )
                    .fill(if selected {
                        theme::hover(self.cx.preferences.dark)
                    } else {
                        theme::panel(self.cx.preferences.dark)
                    })
                    .stroke(egui::Stroke::NONE)
                    .truncate()
                    .right_text(""),
                )
                .on_hover_text(self.tr(label));
            self.remember(format!("filters.shelf.{}", shelf.id()), &response, None);
            if response.clicked() {
                self.state.filter_shelf = shelf;
            }
        }
    }

    pub(super) fn filters(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.search(ui, area.width());
        ui.painter().text(
            area.min + Vec2::new(0.0, 36.0),
            Align2::LEFT_TOP,
            self.tr(shelf_label(self.state.filter_shelf)),
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
        let query = self
            .state
            .searches
            .get(&AssetsTab::Filters)
            .map(|query| query.to_lowercase())
            .unwrap_or_default();
        let cx = self.cx;
        let rows: Vec<_> = cx
            .filters
            .iter()
            .filter(|row| row.shelf == self.state.filter_shelf)
            .filter(|row| row.name.to_lowercase().contains(&query))
            .collect();
        let width = theme::size::EFFECT_CARD;
        let columns = ((area.width() + theme::space::GRID) / (width + theme::space::GRID))
            .floor()
            .max(1.0) as usize;
        let tiles = Rect::from_min_max(area.min + Vec2::new(0.0, 64.0), area.max);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("filters.results")
                .max_rect(tiles),
            |ui| {
                if rows.is_empty() {
                    self.empty(
                        ui,
                        Icon::FilmStrip,
                        Text::AssetsNoMatches,
                        crate::kit::Tone::Quiet,
                    );
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("filters.tiles")
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
                            self.filter_tile(ui, row, rect);
                        }
                    });
            },
        );
    }

    fn filter_tile(&mut self, ui: &mut egui::Ui, row: &FilterRow, rect: Rect) {
        use super::super::effects::EffectPreviewStatus;
        let response = ui.interact(
            rect,
            Id::unique(("filters.tile", row.id)),
            egui::Sense::click(),
        );
        self.remember(format!("filters.tile.{}", row.id), &response, None);
        let status = match row.status {
            Some(EffectPreviewStatus::Ready) => "ready",
            Some(EffectPreviewStatus::Pending) => "pending",
            _ => "failed",
        };
        self.remember(
            format!("filters.tile.{}.{status}", row.id),
            &response,
            None,
        );
        let response = response.on_hover_text(self.tr(Text::FilterAdd));
        if response.clicked() {
            self.actions.push(Action::AddFilter(row.id));
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
        ui.painter().with_clip_rect(rect).text(
            preview.left_bottom() + Vec2::new(0.0, theme::space::XS),
            Align2::LEFT_TOP,
            row.name,
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
    }
}
