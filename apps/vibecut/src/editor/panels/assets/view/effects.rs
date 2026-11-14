//! Renders effect cards in the measured four-column grid.
use super::{
    Action, Align2, AssetsTab, EffectRow, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons,
    theme,
};

/// Library shelves in the order the panel lists them. Every shelf the effect
/// catalogue uses is listed; empty shelves keep their place and show the empty state.
const SHELVES: [(project::EffectShelf, Text); 6] = [
    (
        project::EffectShelf::BlurDetail,
        Text::EffectShelfBlurDetail,
    ),
    (project::EffectShelf::Colour, Text::EffectShelfColour),
    (project::EffectShelf::Distort, Text::EffectShelfDistort),
    (project::EffectShelf::GlowLight, Text::EffectShelfGlowLight),
    (project::EffectShelf::Stylize, Text::EffectShelfStylize),
    (
        project::EffectShelf::OverlayKey,
        Text::EffectShelfOverlayKey,
    ),
];
fn shelf_label(shelf: project::EffectShelf) -> Text {
    SHELVES
        .iter()
        .find(|(candidate, _)| *candidate == shelf)
        .map_or(Text::AssetsAll, |(_, label)| *label)
}
fn shelf_id(shelf: project::EffectShelf) -> &'static str {
    match shelf {
        project::EffectShelf::BlurDetail => "blur_detail",
        project::EffectShelf::Colour => "colour",
        project::EffectShelf::Distort => "distort",
        project::EffectShelf::Stylize => "stylize",
        project::EffectShelf::GlowLight => "glow_light",
        project::EffectShelf::OverlayKey => "overlay_key",
        project::EffectShelf::Inspector => "inspector",
    }
}

impl View<'_> {
    /// One left sub-navigation button, shared by the catalogue tabs.
    pub(super) fn subnav_button(
        &mut self,
        ui: &mut egui::Ui,
        label: Text,
        selected: bool,
    ) -> egui::Response {
        ui.add_sized(
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
        .on_hover_text(self.tr(label))
    }
    /// A collapsible group header: 104 x 22 on the hover fill, the label in the accent while
    /// `active`, and a caret that shows the open state (down when open, right when closed).
    fn effects_group_row(
        &mut self,
        ui: &mut egui::Ui,
        label: Text,
        open: bool,
        active: bool,
    ) -> egui::Response {
        let dark = self.cx.preferences.dark;
        let label = self.tr(label);
        let response = ui.add_sized(
            [theme::size::SUBNAV_BUTTON, theme::size::SUBNAV_GROUP],
            egui::Button::new(
                egui::RichText::new(label)
                    .size(theme::type_size::CHROME)
                    .color(if active {
                        theme::accent(dark)
                    } else {
                        theme::text(dark)
                    }),
            )
            .fill(theme::hover(dark))
            .stroke(egui::Stroke::NONE)
            .truncate()
            .right_text(""),
        );
        let glyph = if open { Icon::CaretDown } else { Icon::CaretRight };
        icons::tinted(glyph, 12.0, theme::text(dark)).paint_at(
            ui,
            Rect::from_center_size(
                egui::pos2(response.rect.right() - 12.0, response.rect.center().y),
                Vec2::splat(12.0),
            ),
        );
        response.on_hover_text(label)
    }
    pub(super) fn effects_nav(&mut self, ui: &mut egui::Ui) {
        let favorites = self.state.effect_favorites;
        let response = self.subnav_button(ui, Text::AssetsFavorites, favorites);
        self.remember("effects.category.favorites".to_owned(), &response, None);
        if response.clicked() {
            self.state.effect_favorites = true;
        }
        // The library shelves sit under one collapsible group, as the reference groups its
        // categories; the group header is active while a shelf (not Favorites) is shown.
        let open = self.state.effect_video_open;
        let response = self.effects_group_row(ui, Text::EffectGroupVideo, open, !favorites);
        self.remember("effects.group.video", &response, None);
        if response.clicked() {
            self.state.effect_video_open = !open;
        }
        if !self.state.effect_video_open {
            return;
        }
        let shelves = std::iter::once((None, Text::AssetsAll))
            .chain(SHELVES.iter().map(|(shelf, label)| (Some(*shelf), *label)));
        for (shelf, label) in shelves {
            let selected = !favorites && self.state.effect_shelf == shelf;
            let response = self.subnav_button(ui, label, selected);
            self.remember(
                format!("effects.category.{}", shelf.map_or("all", shelf_id)),
                &response,
                None,
            );
            if response.clicked() {
                self.state.effect_favorites = false;
                self.state.effect_shelf = shelf;
            }
        }
    }
    pub(super) fn effects(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.search(ui, area.width());
        let favorites = self.state.effect_favorites;
        // The default list names its first section, as the reference names Trending.
        let title = if favorites {
            Text::AssetsFavorites
        } else {
            let first = self.cx.effects.first().map(|row| row.shelf);
            self.state.effect_shelf.or(first).map_or(Text::AssetsAll, shelf_label)
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
            .get(&AssetsTab::Effects)
            .map(|query| query.to_lowercase())
            .unwrap_or_default();
        let cx = self.cx;
        let rows: Vec<_> = cx
            .effects
            .iter()
            .filter(|row| {
                if favorites {
                    cx.preferences
                        .catalog_favorites
                        .contains("effects", row.kind)
                } else {
                    self.state
                        .effect_shelf
                        .is_none_or(|shelf| shelf == row.shelf)
                }
            })
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
                    let text = if favorites {
                        Text::AssetsFavoritesEmpty
                    } else {
                        Text::AssetsNoMatches
                    };
                    self.empty(ui, Icon::FilmStrip, text, crate::kit::Tone::Quiet);
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
        use super::super::effects::EffectPreviewStatus;
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
        let starred = self
            .cx
            .preferences
            .catalog_favorites
            .contains("effects", row.kind);
        let star = Rect::from_min_size(
            preview.right_top() - Vec2::new(24.0, 0.0),
            Vec2::splat(24.0),
        );
        let label = self.tr(if starred {
            Text::AssetsFavoriteRemove
        } else {
            Text::AssetsFavoriteAdd
        });
        let response = ui
            .interact(
                star,
                Id::unique(("effects.favorite", row.kind)),
                egui::Sense::click(),
            )
            .on_hover_text(label);
        self.remember(format!("effects.favorite.{}", row.kind), &response, None);
        ui.painter().text(
            star.center(),
            Align2::CENTER_CENTER,
            if starred { "★" } else { "☆" },
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
        if response.clicked() {
            self.actions.push(Action::ToggleFavorite {
                family: "effects",
                id: row.kind.to_owned(),
                favorite: !starred,
            });
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
