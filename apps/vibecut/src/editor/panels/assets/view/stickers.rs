//! Draws the Stickers tab: search, category tabs, the result count and See all, the Recent
//! section with Clear, and the tile grid. Shape tiles insert through their graphic commands;
//! flag tiles through the sticker insert.

use super::super::stickers::{DownloadProblem, Offer, StickerTab, Tile};
use super::super::{Action, Align2, Color32, Icon, Rect, Text, Vec2, egui, theme};
use super::View;
use crate::i18n;
use std::time::Duration;

/// Tile edge and gaps from the measured Stickers grid (`theme::size::STICKER_*`).
const TILE: f32 = theme::size::STICKER_TILE;
const GAP_X: f32 = theme::size::STICKER_GAP_X;
const GAP_Y: f32 = theme::size::STICKER_GAP_Y;

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
            for (tab, label) in [
                (StickerTab::All, Text::AssetsAll),
                (StickerTab::Emoji, Text::StickerEmoji),
                (StickerTab::Icons, Text::StickerIcons),
                (StickerTab::Flags, Text::StickerFlags),
                (StickerTab::Shapes, Text::GraphicShapes),
            ] {
                let selected = self.state.sticker_tab == tab;
                let response = ui.selectable_label(
                    selected,
                    egui::RichText::new(self.tr(label))
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL),
                );
                self.remember(format!("stickers.category.{}", tab.id()), &response, None);
                if response.clicked() && !selected {
                    self.state.sticker_tab = tab;
                    self.state.sticker_expanded = false;
                }
            }
        });
        let count = i18n::format(
            cx.preferences.locale,
            Text::StickerResultCount,
            &[(
                "count",
                &i18n::number(cx.preferences.locale, rows.total as u64),
            )],
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(count)
                    .font(theme::label_font())
                    .size(theme::type_size::SMALL_LABEL)
                    .color(theme::secondary(cx.preferences.dark)),
            );
            if !rows.expanded && rows.total > rows.results.len() {
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
                for offer in &rows.offers {
                    self.download_card(ui, offer);
                    ui.add_space(theme::space::SM);
                }
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
                    self.tile_grid(ui, &rows.recent, true, ready);
                    ui.add_space(theme::space::SM);
                }
                if rows.results.is_empty() {
                    ui.label(
                        egui::RichText::new(self.tr(Text::StickerNoResults))
                            .font(theme::label_font())
                            .color(theme::secondary(cx.preferences.dark)),
                    );
                } else {
                    self.tile_grid(ui, &rows.results, false, ready);
                }
            });
    }

    /// One collection the tab does not hold yet: its name, licence credit and size, then the
    /// download action, or the progress with cancel, or the offline and retry states.
    fn download_card(&mut self, ui: &mut egui::Ui, offer: &Offer) {
        let locale = self.cx.preferences.locale;
        let dark = self.cx.preferences.dark;
        let downloads = self.state.sticker_downloads.as_ref();
        let running = downloads
            .and_then(|downloads| downloads.active.as_ref())
            .filter(|active| active.collection == offer.collection)
            .map(|active| (active.progress, active.cancelling));
        let busy = downloads.is_some_and(|downloads| downloads.active.is_some());
        let problem = downloads
            .and_then(|downloads| downloads.problem.as_ref())
            .filter(|(collection, _)| *collection == offer.collection)
            .map(|(_, problem)| *problem);
        let kib = offer.bytes.div_ceil(1024);
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(&offer.name)
                    .font(theme::label_font())
                    .size(theme::type_size::SMALL_LABEL)
                    .color(theme::text(dark)),
            );
            let credit = i18n::format(
                locale,
                Text::StickerLicence,
                &[
                    ("author", &offer.author),
                    ("license", &offer.license),
                    ("source", &offer.source),
                ],
            );
            ui.label(caption(credit, dark));
            let size = i18n::format(
                locale,
                Text::StickerDownloadSize,
                &[("kib", &i18n::number(locale, kib))],
            );
            ui.label(caption(size, dark));
            match running {
                Some((_, true)) => {
                    ui.label(caption(self.tr(Text::StickerCancelling), dark));
                }
                Some((progress, false)) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                    let percent = (progress * 100.0).round() as u64;
                    let text = i18n::format(
                        locale,
                        Text::StickerDownloading,
                        &[("percent", &i18n::number(locale, percent))],
                    );
                    ui.label(caption(text, dark));
                    ui.add(egui::ProgressBar::new(progress).desired_width(ui.available_width()));
                    let response = ui.button(self.tr(Text::StickerDownloadCancel));
                    self.remember("stickers.download.cancel", &response, None);
                    if response.clicked() {
                        self.actions.push(Action::CancelStickerDownload);
                    }
                }
                None => {
                    match problem {
                        Some(DownloadProblem::Offline) => {
                            ui.label(caption(self.tr(Text::StickerDownloadOffline), dark));
                        }
                        Some(DownloadProblem::Failed) => {
                            ui.label(caption(self.tr(Text::StickerDownloadFailed), dark));
                        }
                        None => {}
                    }
                    let id = format!("stickers.download.{}", offer.collection);
                    let response =
                        ui.add_enabled(!busy, egui::Button::new(self.tr(Text::StickerDownload)));
                    self.remember(id, &response, None);
                    if response.clicked() {
                        self.actions
                            .push(Action::DownloadSticker(offer.collection.clone()));
                    }
                }
            }
        });
    }

    fn tile_grid(&mut self, ui: &mut egui::Ui, tiles: &[Tile], recent: bool, ready: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(GAP_X, GAP_Y);
            for tile in tiles {
                self.tile(ui, tile, recent, ready);
            }
        });
    }

    fn tile(&mut self, ui: &mut egui::Ui, tile: &Tile, recent: bool, ready: bool) {
        let dark = self.cx.preferences.dark;
        ui.vertical(|ui| {
            ui.set_width(TILE);
            let (frame, _) = ui.allocate_exact_size(Vec2::splat(TILE), egui::Sense::hover());
            ui.painter()
                .rect_filled(frame, theme::radius::UPLOAD, theme::field(dark));
            match self.state.sticker_thumbnails.texture(ui.ctx(), &tile.id) {
                Some(texture) => {
                    let size = texture.size_vec2() / ui.ctx().pixels_per_point();
                    ui.painter().image(
                        texture.id(),
                        Rect::from_center_size(frame.center(), size),
                        Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().text(
                        frame.center(),
                        Align2::CENTER_CENTER,
                        self.tr(Text::AssetsUnavailable),
                        theme::label_font(),
                        theme::text(dark),
                    );
                }
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&tile.name)
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL)
                        .color(theme::text(dark)),
                )
                .truncate(),
            );
            // A shape's id is its command id, so its card matches `graphic.insert.*` directly.
            let id = match (tile.shape, recent) {
                (true, false) => tile.id.clone(),
                (_, true) => format!("stickers.recent.insert.{}", slug(&tile.id)),
                (false, false) => format!("stickers.insert.{}", slug(&tile.id)),
            };
            if self.icon(ui, &id, Icon::Plus, Text::StickerInsert, ready) {
                self.actions.push(if tile.shape {
                    Action::InsertShape(tile.id.clone())
                } else {
                    Action::InsertSticker(tile.id.clone())
                });
            }
        });
    }
}

/// Secondary small text of the download card.
fn caption(text: impl Into<String>, dark: bool) -> egui::RichText {
    egui::RichText::new(text)
        .font(theme::label_font())
        .size(theme::type_size::SMALL_LABEL)
        .color(theme::secondary(dark))
}

fn slug(id: &str) -> String {
    id.replace(':', "-")
}
