//! Renders the Media tab's Yours pages in the reference's order.
//!
//! Favorites, My presets and Brand assets share one empty face: a 24 px state glyph, 8 px gap
//! and one 12 px line centred in the page, with an optional accent link after the line (our
//! state tokens; the reference's empty faces are not measured to the pixel).
mod brand;
mod favorites;

use super::{Action, AssetsTab, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons, theme};

impl View<'_> {
    /// My presets: the reference's empty face. Clip presets are not built, so the page has no
    /// list yet and the reference's `Learn how` link has nowhere to go.
    pub(in crate::editor::panels::assets::view) fn my_presets(&mut self, ui: &mut egui::Ui) {
        let body = ui.available_rect_before_wrap();
        self.empty_page(
            ui,
            body,
            Icon::SlidersHorizontal,
            Text::MyPresetsEmpty,
            None,
        );
    }

    /// The glyph, the line and, when given, the link, centred in `body`. Returns whether the
    /// link was clicked.
    fn empty_page(
        &mut self,
        ui: &mut egui::Ui,
        body: Rect,
        glyph: Icon,
        line: Text,
        link: Option<(Text, &'static str)>,
    ) -> bool {
        let dark = self.cx.preferences.dark;
        let font = FontId::proportional(theme::type_size::BODY);
        let text = ui.painter().layout_no_wrap(
            self.tr(line).to_owned(),
            font.clone(),
            theme::secondary(dark),
        );
        let link_galley = link.map(|(label, _)| {
            ui.painter()
                .layout_no_wrap(self.tr(label).to_owned(), font, theme::accent(dark))
        });
        let width = text.size().x
            + link_galley
                .as_ref()
                .map_or(0.0, |galley| theme::space::XS + galley.size().x);
        let height = theme::icon::STATE + theme::space::MD + text.size().y;
        let top = body.center().y - height / 2.0;
        icons::tinted(glyph, theme::icon::STATE, theme::secondary(dark)).paint_at(
            ui,
            Rect::from_center_size(
                egui::pos2(body.center().x, top + theme::icon::STATE / 2.0),
                Vec2::splat(theme::icon::STATE),
            ),
        );
        let at = egui::pos2(
            body.center().x - width / 2.0,
            top + theme::icon::STATE + theme::space::MD,
        );
        let text_width = text.size().x;
        ui.painter().galley(at, text, theme::secondary(dark));
        let (Some((label, id)), Some(galley)) = (link, link_galley) else {
            return false;
        };
        let link_rect = Rect::from_min_size(
            at + Vec2::new(text_width + theme::space::XS, 0.0),
            galley.size(),
        );
        ui.painter()
            .galley(link_rect.min, galley, theme::accent(dark));
        let response = ui
            .interact(link_rect, Id::unique(id), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, self.tr(label)));
        self.remember(id, &response, None);
        response.clicked()
    }
}
