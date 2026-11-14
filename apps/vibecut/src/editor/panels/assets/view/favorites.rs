//! Renders the Media tab's Favorites page: the starred catalogue items and their empty state.
use super::{Action, AssetsTab, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons, theme};
use crate::i18n;

/// The catalogue families a star can sit in: the stored id, the label and the assets tab that
/// lists them (animations are starred from the properties panel, so they have no tab).
const FAMILIES: [(&str, Text, Option<AssetsTab>); 6] = [
    ("effects", Text::Effects, Some(AssetsTab::Effects)),
    (
        "transitions",
        Text::Transitions,
        Some(AssetsTab::Transitions),
    ),
    ("filters", Text::Filters, Some(AssetsTab::Filters)),
    ("animations", Text::MediaFavoritesAnimations, None),
    ("stickers", Text::Stickers, Some(AssetsTab::Stickers)),
    ("sounds", Text::Sounds, Some(AssetsTab::Sounds)),
];

impl View<'_> {
    fn favorite_count(&self, family: &str) -> usize {
        let favorites = &self.cx.preferences.catalog_favorites;
        if family == "sounds" {
            favorites.sounds().len()
        } else {
            favorites.list(family).len()
        }
    }

    pub(super) fn favorites(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let tool = Rect::from_min_size(
            egui::pos2(area.right() - 36.0, area.top()),
            Vec2::new(36.0, theme::size::CONTROL),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.favorites.filter.tool")
                .max_rect(tool),
            |ui| self.favorites_filter(ui),
        );
        let chosen = self.state.media_favorites_family;
        let rows: Vec<_> = FAMILIES
            .iter()
            .filter(|(family, ..)| chosen.is_none_or(|chosen| chosen == *family))
            .map(|(family, label, tab)| (*label, *tab, self.favorite_count(family)))
            .filter(|(_, _, count)| *count > 0)
            .collect();
        let body = Rect::from_min_max(
            egui::pos2(area.left(), tool.bottom() + theme::space::MD),
            area.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("media.favorites.body")
                .max_rect(body),
            |ui| {
                if rows.is_empty() {
                    self.favorites_empty(ui, body);
                    return;
                }
                let dark = self.cx.preferences.dark;
                egui::ScrollArea::vertical()
                    .id_salt("media.favorites.list")
                    .show(ui, |ui| {
                        for (label, tab, count) in rows {
                            let count = i18n::number(self.cx.preferences.locale, count as u64);
                            let response = ui.add_enabled(
                                tab.is_some(),
                                egui::Button::new(
                                    egui::RichText::new(self.tr(label))
                                        .size(theme::type_size::BODY)
                                        .color(theme::text(dark)),
                                )
                                .fill(theme::hover(dark))
                                .stroke(egui::Stroke::NONE)
                                .min_size(Vec2::new(ui.available_width(), theme::size::CONTROL))
                                .right_text(count),
                            );
                            self.remember(
                                format!("media.favorites.row.{}", label.key()),
                                &response,
                                None,
                            );
                            if let Some(tab) = tab
                                && response.clicked()
                            {
                                self.actions.push(Action::SelectTab(tab));
                            }
                        }
                    });
            },
        );
    }

    /// The filter at the top right: one catalogue family, or all of them.
    fn favorites_filter(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = theme::size::CONTROL;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
        let menu = egui::ComboBox::from_id_salt(Id::unique("media.favorites.filter"))
            .selected_text("")
            .width(32.0)
            .show_ui(ui, |ui| {
                let all = ui.selectable_label(
                    self.state.media_favorites_family.is_none(),
                    self.tr(Text::AssetsAll),
                );
                self.remember("media.favorites.filter.all", &all, None);
                if all.clicked() {
                    self.state.media_favorites_family = None;
                    ui.close();
                }
                for (family, label, _) in FAMILIES {
                    let response = ui.selectable_label(
                        self.state.media_favorites_family == Some(family),
                        self.tr(label),
                    );
                    self.remember(
                        format!("media.favorites.filter.{family}"),
                        &response,
                        None,
                    );
                    if response.clicked() {
                        self.state.media_favorites_family = Some(family);
                        ui.close();
                    }
                }
            });
        let response = menu
            .response
            .on_hover_text(self.tr(Text::MediaFavoritesFilter));
        icons::tinted(
            Icon::Funnel,
            theme::icon::DENSE,
            theme::text(self.cx.preferences.dark),
        )
        .paint_at(
            ui,
            Rect::from_center_size(
                egui::pos2(response.rect.left() + 10.0, response.rect.center().y),
                Vec2::splat(theme::icon::DENSE),
            ),
        );
        self.remember("media.favorites.filter", &response, Some(Icon::Funnel));
    }

    /// The empty state: the star, one line, and the link to where stars are made. The link
    /// goes to the Effects tab, the nearest catalogue (the reference's Library is online).
    fn favorites_empty(&mut self, ui: &mut egui::Ui, body: Rect) {
        let dark = self.cx.preferences.dark;
        let font = FontId::proportional(theme::type_size::BODY);
        let text = ui.painter().layout_no_wrap(
            self.tr(Text::MediaFavoritesEmpty).to_owned(),
            font.clone(),
            theme::secondary(dark),
        );
        let link = ui.painter().layout_no_wrap(
            self.tr(Text::MediaFavoritesLink).to_owned(),
            font,
            theme::accent(dark),
        );
        let width = text.size().x + theme::space::XS + link.size().x;
        let height = theme::icon::STATE + theme::space::MD + text.size().y;
        let top = body.center().y - height / 2.0;
        icons::tinted(Icon::Star, theme::icon::STATE, theme::secondary(dark)).paint_at(
            ui,
            Rect::from_center_size(
                egui::pos2(body.center().x, top + theme::icon::STATE / 2.0),
                Vec2::splat(theme::icon::STATE),
            ),
        );
        let line = egui::pos2(
            body.center().x - width / 2.0,
            top + theme::icon::STATE + theme::space::MD,
        );
        let link_rect = Rect::from_min_size(
            line + Vec2::new(text.size().x + theme::space::XS, 0.0),
            link.size(),
        );
        ui.painter().galley(line, text, theme::secondary(dark));
        ui.painter().galley(link_rect.min, link, theme::accent(dark));
        let response = ui
            .interact(
                link_rect,
                Id::unique("media.favorites.link"),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                self.tr(Text::MediaFavoritesLink),
            )
        });
        self.remember("media.favorites.link", &response, None);
        if response.clicked() {
            self.actions.push(Action::SelectTab(AssetsTab::Effects));
        }
    }
}
