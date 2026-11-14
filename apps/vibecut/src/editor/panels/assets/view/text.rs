//! Draws the Text tab: the Styles and Templates pages, search, the category chips with the star
//! chip for favourites, the All/Commercial filter and the tile grid. A style tile applies to the
//! selected text items; a template tile inserts a new item at the playhead.

use super::super::text::{Entry, TextChip, TextPage, Tile, tiles};
use super::super::{Action, Align2, AssetsTab, Color32, Rect, Text, Vec2, egui, theme};
use super::View;

/// Tile edge and gaps from the measured Text Effects grid (`theme::size::TEXT_*`).
const TILE: f32 = theme::size::TEXT_TILE;
const GAP_X: f32 = theme::size::TEXT_GAP_X;
const GAP_Y: f32 = theme::size::TEXT_GAP_Y;
/// The star hot spot in a tile's top-right corner, as the other catalogue grids.
const STAR: f32 = 24.0;

impl View<'_> {
    pub(super) fn text_library(&mut self, ui: &mut egui::Ui) {
        let cx = self.cx;
        let dark = cx.preferences.dark;
        let page = self.state.text_page;
        let width = ui.available_width();
        ui.horizontal(|ui| {
            for candidate in TextPage::ALL {
                let selected = page == candidate;
                let response = ui.selectable_label(
                    selected,
                    egui::RichText::new(self.tr(candidate.label()))
                        .font(theme::label_font())
                        .size(theme::type_size::SMALL_LABEL),
                );
                self.remember(format!("text.page.{}", candidate.id()), &response, None);
                if response.clicked() && !selected {
                    self.state.text_page = candidate;
                }
            }
        });
        ui.add_space(theme::space::SM);
        self.search(ui, width);
        ui.add_space(theme::space::SM);
        self.text_chips(ui);
        ui.add_space(theme::space::SM);
        let query = self
            .state
            .searches
            .get(&AssetsTab::Text)
            .cloned()
            .unwrap_or_default();
        let page = self.state.text_page;
        let chip = self.state.text_chip;
        let rows = tiles(
            page,
            &query,
            chip,
            self.state.text_commercial,
            cx.preferences.catalog_favorites.list(page.family()),
            cx.preferences.locale,
        );
        egui::ScrollArea::vertical()
            .id_salt("text.list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if rows.is_empty() {
                    let empty = if chip == Some(TextChip::Favorites) && query.trim().is_empty() {
                        Text::AssetsFavoritesEmpty
                    } else {
                        Text::TextLibraryNoResults
                    };
                    let response = ui.label(
                        egui::RichText::new(self.tr(empty))
                            .font(theme::label_font())
                            .color(theme::secondary(dark)),
                    );
                    self.remember("text.empty", &response, None);
                    return;
                }
                self.text_grid(ui, page, &rows);
            });
    }

    /// The star chip, the five shelf chips, then the All and Commercial filter chips. Choosing
    /// the chosen category chip again clears it.
    fn text_chips(&mut self, ui: &mut egui::Ui) {
        let dark = self.cx.preferences.dark;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(theme::space::XS, theme::space::XS);
            for chip in TextChip::ALL {
                let selected = self.state.text_chip == Some(chip);
                let response = chip_button(ui, self.tr(chip.label()), selected, dark);
                self.remember(format!("text.chip.{}", chip.id()), &response, None);
                if response.clicked() {
                    self.state.text_chip = if selected { None } else { Some(chip) };
                }
            }
            for (commercial, id, label) in [
                (false, "all", Text::AssetsAll),
                (true, "commercial", Text::FontsCommercial),
            ] {
                let selected = self.state.text_commercial == commercial;
                let response = chip_button(ui, self.tr(label), selected, dark);
                self.remember(format!("text.filter.{id}"), &response, None);
                if response.clicked() {
                    self.state.text_commercial = commercial;
                }
            }
        });
    }

    /// Tiles in as many columns as the list width holds.
    fn text_grid(&mut self, ui: &mut egui::Ui, page: TextPage, rows: &[Tile]) {
        let columns = ((ui.available_width() + GAP_X) / (TILE + GAP_X))
            .floor()
            .max(1.0) as usize;
        ui.spacing_mut().item_spacing.y = GAP_Y;
        for row in rows.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(GAP_X, GAP_Y);
                for tile in row {
                    self.text_tile(ui, page, tile);
                }
            });
        }
    }

    fn text_tile(&mut self, ui: &mut egui::Ui, page: TextPage, tile: &Tile) {
        let dark = self.cx.preferences.dark;
        let (frame, response) = ui.allocate_exact_size(Vec2::splat(TILE), egui::Sense::click());
        ui.painter()
            .rect_filled(frame, theme::radius::UPLOAD, theme::field(dark));
        let [red, green, blue, alpha] = tile.look().style().color.0;
        // A style needs a selected text item: its tiles dim while none is selected.
        let idle = matches!(tile.entry, Entry::Look(_)) && self.cx.text_targets.is_empty();
        let mut ink = Color32::from_rgba_unmultiplied(red, green, blue, alpha);
        if idle {
            ink = ink.gamma_multiply(0.5);
        }
        let sample = match tile.entry {
            Entry::Look(_) => self.tr(Text::LookSample),
            Entry::Template(_) => tile.name,
        };
        ui.painter().with_clip_rect(frame).text(
            frame.center(),
            Align2::CENTER_CENTER,
            sample,
            theme::label_font(),
            ink,
        );
        let name = tile.name;
        let response = response.on_hover_text(if idle {
            self.tr(Text::TextLibrarySelectText)
        } else {
            name
        });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
        let (kind, action) = match tile.entry {
            Entry::Look(look) => ("look", Action::ApplyTextLook(look)),
            Entry::Template(template) => ("template", Action::InsertTextTemplate(template)),
        };
        self.remember(format!("text.{kind}.{}", tile.id), &response, None);
        // A template needs an insertion point; a style reports what to select when none is.
        let ready = match tile.entry {
            Entry::Look(_) => true,
            Entry::Template(_) => self.cx.insertion_ready,
        };
        if response.clicked() && ready {
            self.actions.push(action);
        }
        let star = Rect::from_min_size(frame.right_top() - Vec2::new(STAR, 0.0), Vec2::splat(STAR));
        let label = self.tr(if tile.starred {
            Text::AssetsFavoriteRemove
        } else {
            Text::AssetsFavoriteAdd
        });
        let response = ui
            .interact(
                star,
                egui::Id::unique(("text.favorite", page.id(), tile.id.as_str())),
                egui::Sense::click(),
            )
            .on_hover_text(label);
        self.remember(
            format!("text.favorite.{}.{}", page.id(), tile.id),
            &response,
            None,
        );
        ui.painter().text(
            star.center(),
            Align2::CENTER_CENTER,
            if tile.starred { "★" } else { "☆" },
            theme::label_font(),
            theme::text(dark),
        );
        if response.clicked() {
            self.actions.push(Action::ToggleFavorite {
                family: page.family(),
                id: tile.id.clone(),
                favorite: !tile.starred,
            });
        }
    }
}

/// One chip of the app's selected-chip roles: accent fill with on-accent ink when chosen, field
/// fill with text ink otherwise, 24 px high.
fn chip_button(ui: &mut egui::Ui, label: &str, selected: bool, dark: bool) -> egui::Response {
    let ink = if selected {
        theme::on_accent(dark)
    } else {
        theme::text(dark)
    };
    let fill = if selected {
        theme::accent(dark)
    } else {
        theme::field(dark)
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .font(theme::label_font())
                .size(theme::type_size::SMALL_LABEL)
                .color(ink),
        )
        .fill(fill)
        .stroke(egui::Stroke::new(theme::space::HAIRLINE, theme::border(dark)))
        .min_size(Vec2::new(0.0, theme::size::CONTROL)),
    )
}
