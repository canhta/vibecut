//! Draws the Stickers tab: search, category tabs, the result count and See all, the Recent
//! section with Clear, and the sticker grid. Every tile inserts through one action.

use super::super::{Action, Align2, Color32, Icon, Rect, Text, Vec2, egui, theme};
use super::View;
use crate::i18n;
use engine::control::{StickerCategory, StickerEntry};

/// Tile edge, the measured Effects card width the Stickers grid reuses until its own is measured.
const TILE: f32 = 88.0;
/// Horizontal and vertical gap between tiles, from the same measured grid.
const GAP: f32 = 15.0;

impl View<'_> {
    pub(super) fn stickers(&mut self, ui: &mut egui::Ui) {
        let cx = self.cx;
        let Some(rows) = cx.stickers.as_ref() else {
            return;
        };
        let ready = cx.insertion_ready;
        let width = ui.available_width();
        self.search(ui, width);
        ui.add_space(theme::space::SM);
        ui.horizontal(|ui| {
            for (category, label) in [
                (StickerCategory::All, Text::AssetsAll),
                (StickerCategory::Flags, Text::StickerFlags),
                (StickerCategory::Shapes, Text::GraphicShapes),
            ] {
                let selected = self.state.sticker_category == category;
                let response = ui.selectable_label(
                    selected,
                    egui::RichText::new(self.tr(label))
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL),
                );
                self.remember(
                    format!("stickers.category.{}", category_id(category)),
                    &response,
                    None,
                );
                if response.clicked() && !selected {
                    self.state.sticker_category = category;
                    self.state.sticker_expanded = false;
                }
            }
        });
        let count = i18n::format(
            cx.preferences.locale,
            Text::StickerResultCount,
            &[(
                "count",
                &i18n::number(cx.preferences.locale, rows.page.total),
            )],
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(count)
                    .font(theme::label_font())
                    .size(theme::type_size::SMALL_LABEL)
                    .color(theme::secondary(cx.preferences.dark)),
            );
            if !rows.expanded && rows.page.total > rows.page.results.len() as u64 {
                let response = ui.button(self.tr(Text::StickerSeeAll));
                self.remember("stickers.see_all", &response, None);
                if response.clicked() {
                    self.state.sticker_expanded = true;
                }
            }
        });
        ui.add_space(theme::space::SM);
        egui::ScrollArea::vertical()
            .id_salt("stickers.list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !rows.recent.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::StickerRecent))
                                .font(theme::label_font())
                                .size(theme::type_size::SMALL_LABEL)
                                .color(theme::text(cx.preferences.dark)),
                        );
                        let response = ui.button(self.tr(Text::StickerClear));
                        self.remember("stickers.recent.clear", &response, None);
                        if response.clicked() {
                            self.actions.push(Action::ClearStickerRecents);
                        }
                    });
                    self.sticker_grid(ui, &rows.recent, "stickers.recent.insert", ready);
                    ui.add_space(theme::space::SM);
                }
                if rows.page.results.is_empty() {
                    ui.label(
                        egui::RichText::new(self.tr(Text::StickerNoResults))
                            .font(theme::label_font())
                            .color(theme::secondary(cx.preferences.dark)),
                    );
                } else {
                    self.sticker_grid(ui, &rows.page.results, "stickers.insert", ready);
                }
            });
    }

    fn sticker_grid(
        &mut self,
        ui: &mut egui::Ui,
        entries: &[StickerEntry],
        prefix: &str,
        ready: bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(GAP);
            for entry in entries {
                self.sticker_tile(ui, entry, prefix, ready);
            }
        });
    }

    fn sticker_tile(&mut self, ui: &mut egui::Ui, entry: &StickerEntry, prefix: &str, ready: bool) {
        let dark = self.cx.preferences.dark;
        ui.vertical(|ui| {
            ui.set_width(TILE);
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(TILE), egui::Sense::hover());
            ui.painter()
                .rect_filled(tile, theme::radius::UPLOAD, theme::field(dark));
            match self.state.sticker_thumbnails.texture(ui.ctx(), &entry.id) {
                Some(texture) => {
                    let size = texture.size_vec2() / ui.ctx().pixels_per_point();
                    ui.painter().image(
                        texture.id(),
                        Rect::from_center_size(tile.center(), size),
                        Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().text(
                        tile.center(),
                        Align2::CENTER_CENTER,
                        self.tr(Text::AssetsUnavailable),
                        theme::label_font(),
                        theme::text(dark),
                    );
                }
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&entry.name)
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL)
                        .color(theme::text(dark)),
                )
                .truncate(),
            );
            let id = format!("{prefix}.{}", entry.id.replace(':', "-"));
            if self.icon(ui, &id, Icon::Plus, Text::StickerInsert, ready) {
                self.actions.push(Action::InsertSticker(entry.id.clone()));
            }
        });
    }
}

fn category_id(category: StickerCategory) -> &'static str {
    match category {
        StickerCategory::All => "all",
        StickerCategory::Flags => "flags",
        StickerCategory::Shapes => "shapes",
    }
}
