//! Renders the Brand assets page: the active kit's files by section, each a click to insert.
//!
//! The header row follows capture 111016 (the kit's name at the left, the refresh icon at the
//! right; the reference's settings gear opens its online space and is not built). Rows are name
//! rows like the Favorites page, ours: the reference shows no filled page.
use super::{Action, Icon, Rect, Text, Vec2, View, egui, theme};
use crate::editor::panels::assets::media_panel::brand::Listing;
use crate::i18n;
use engine::control::{BrandEntryState, BrandMediaSection};

/// The file sections in the Home page's order, each with its label.
const SECTIONS: [(BrandMediaSection, &str, Text); 5] = [
    (BrandMediaSection::Video, "videos", Text::BrandKitVideos),
    (BrandMediaSection::Image, "images", Text::BrandKitImages),
    (BrandMediaSection::Sticker, "stickers", Text::Stickers),
    (
        BrandMediaSection::Background,
        "backgrounds",
        Text::BrandKitBackgrounds,
    ),
    (BrandMediaSection::Music, "music", Text::Music),
];

impl View<'_> {
    pub(in crate::editor::panels::assets::view) fn brand_assets(&mut self, ui: &mut egui::Ui) {
        let dark = self.cx.preferences.dark;
        let area = ui.available_rect_before_wrap();
        let Some(brand) = &self.state.brand else {
            self.empty(
                ui,
                Icon::BookmarkSimple,
                Text::BrandKitUnavailable,
                crate::kit::Tone::Quiet,
            );
            return;
        };
        let copying = brand.copying();
        let kit = match &brand.listing {
            Listing::Ready(kit) => Some(kit.clone()),
            Listing::Loading | Listing::Failed => None,
        };
        let failed = matches!(brand.listing, Listing::Failed);
        let loading = matches!(brand.listing, Listing::Loading);
        let header = Rect::from_min_size(area.min, Vec2::new(area.width(), theme::size::CONTROL));
        let name = kit.as_ref().and_then(|kit| kit.as_ref()).map_or_else(
            || self.tr(Text::BrandKit).to_owned(),
            |kit| kit.name.clone(),
        );
        ui.painter().text(
            egui::pos2(header.left(), header.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(theme::type_size::BODY),
            theme::text(dark),
        );
        let refresh = Rect::from_min_size(
            egui::pos2(header.right() - theme::size::CONTROL, header.top()),
            Vec2::splat(theme::size::CONTROL),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(refresh), |ui| {
            if self.icon(
                ui,
                "media.brand.refresh",
                Icon::ArrowClockwise,
                Text::BrandAssetsRefresh,
                !loading,
            ) {
                self.actions.push(Action::RefreshBrand);
            }
        });
        let body = Rect::from_min_max(
            egui::pos2(area.left(), header.bottom() + theme::space::MD),
            area.max,
        );
        if loading {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                self.empty(
                    ui,
                    Icon::ArrowClockwise,
                    Text::Loading,
                    crate::kit::Tone::Quiet,
                );
            });
            return;
        }
        if failed {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                self.empty(
                    ui,
                    Icon::BookmarkSimple,
                    Text::BrandKitUnavailable,
                    crate::kit::Tone::Error,
                );
            });
            return;
        }
        let media = kit.flatten().map(|kit| kit.media).unwrap_or_default();
        if media.is_empty() {
            self.empty_page(ui, body, Icon::BookmarkSimple, Text::BrandAssetsEmpty, None);
            return;
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.brand.body")
                .max_rect(body),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("media.brand.list")
                    .show(ui, |ui| {
                        for (section, key, label) in SECTIONS {
                            let rows: Vec<_> = media
                                .iter()
                                .filter(|entry| entry.section == section)
                                .collect();
                            if rows.is_empty() {
                                continue;
                            }
                            let count = i18n::number(self.cx.preferences.locale, rows.len() as u64);
                            let response = ui.label(
                                egui::RichText::new(format!("{}  {count}", self.tr(label)))
                                    .font(theme::label_font())
                                    .size(theme::type_size::BODY)
                                    .color(theme::text(dark)),
                            );
                            self.remember(format!("media.brand.section.{key}"), &response, None);
                            for entry in rows {
                                let available = entry.state == BrandEntryState::Available;
                                let label = match entry.state {
                                    BrandEntryState::Available => entry.name.clone(),
                                    BrandEntryState::Missing => format!(
                                        "{}  {}",
                                        entry.name,
                                        self.tr(Text::BrandKitEntryMissing)
                                    ),
                                    BrandEntryState::Corrupt => format!(
                                        "{}  {}",
                                        entry.name,
                                        self.tr(Text::BrandKitEntryCorrupt)
                                    ),
                                };
                                let response = ui.add_enabled(
                                    available && !copying && self.cx.insertion_ready,
                                    egui::Button::new(
                                        egui::RichText::new(label)
                                            .size(theme::type_size::BODY)
                                            .color(theme::text(dark)),
                                    )
                                    .fill(theme::hover(dark))
                                    .stroke(egui::Stroke::NONE)
                                    .truncate()
                                    .min_size(Vec2::new(
                                        ui.available_width(),
                                        theme::size::CONTROL,
                                    )),
                                );
                                self.remember(format!("media.brand.{}", entry.id), &response, None);
                                if response.clicked() {
                                    self.actions.push(Action::InsertBrandMedia(entry.id));
                                }
                            }
                            ui.add_space(theme::space::MD);
                        }
                    });
            },
        );
    }
}
