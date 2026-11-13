//! Renders the Filters tab: the shelf navigation and the look tiles.
//!
//! The star sits on the tile's top-left corner (family `filters`, key `FilterRow::id`) and
//! Favorites heads `filters_nav`.
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
        let favorites = self.state.filter_favorites;
        let response = self.subnav_button(ui, Text::AssetsFavorites, favorites);
        self.remember("filters.shelf.favorites".to_owned(), &response, None);
        if response.clicked() {
            self.state.filter_favorites = true;
        }
        for shelf in project::FilterShelf::ALL {
            let selected = !favorites && self.state.filter_shelf == shelf;
            let response = self.subnav_button(ui, shelf_label(shelf), selected);
            self.remember(format!("filters.shelf.{}", shelf.id()), &response, None);
            if response.clicked() {
                self.state.filter_favorites = false;
                self.state.filter_shelf = shelf;
            }
        }
    }

    pub(super) fn filters(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.search(ui, area.width());
        let favorites = self.state.filter_favorites;
        let title = if favorites {
            Text::AssetsFavorites
        } else {
            shelf_label(self.state.filter_shelf)
        };
        ui.painter().text(
            area.min + Vec2::new(0.0, 36.0),
            Align2::LEFT_TOP,
            self.tr(title),
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
            .filter(|row| {
                if favorites {
                    cx.preferences.catalog_favorites.contains("filters", row.id)
                } else {
                    row.shelf == self.state.filter_shelf
                }
            })
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
                    let text = if favorites {
                        Text::AssetsFavoritesEmpty
                    } else {
                        Text::AssetsNoMatches
                    };
                    self.empty(ui, Icon::FilmStrip, text, crate::kit::Tone::Quiet);
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
        self.filter_star(ui, row, preview);
        ui.painter().with_clip_rect(rect).text(
            preview.left_bottom() + Vec2::new(0.0, theme::space::XS),
            Align2::LEFT_TOP,
            row.name,
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
    }

    /// The favourite star on the tile's top-left corner.
    fn filter_star(&mut self, ui: &mut egui::Ui, row: &FilterRow, preview: Rect) {
        let starred = self
            .cx
            .preferences
            .catalog_favorites
            .contains("filters", row.id);
        let star = Rect::from_min_size(preview.left_top(), Vec2::splat(24.0));
        let label = self.tr(if starred {
            Text::AssetsFavoriteRemove
        } else {
            Text::AssetsFavoriteAdd
        });
        let response = ui
            .interact(
                star,
                Id::unique(("filters.favorite", row.id)),
                egui::Sense::click(),
            )
            .on_hover_text(label);
        self.remember(format!("filters.favorite.{}", row.id), &response, None);
        ui.painter().text(
            star.center(),
            Align2::CENTER_CENTER,
            if starred { "★" } else { "☆" },
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
        if response.clicked() {
            self.actions.push(Action::ToggleFavorite {
                family: "filters",
                id: row.id.to_owned(),
                favorite: !starred,
            });
        }
    }
}
