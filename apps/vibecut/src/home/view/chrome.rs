//! Draws the Home rail and the top band that sit around the page.

use super::{
    RAIL_INSET, RAIL_ROW, RAIL_RULE_GAP, ROW_RADIUS, TOP_BAND,
    widgets::{line, semibold, set_author_id},
};
use crate::{
    catalog::{self, Locale, Text},
    egui,
    home::state::{Action, State},
    kit::{self, Icon},
};

/// A rail destination: icon at 8 px, label 34 px from the row's left edge.
fn nav_row(
    ui: &mut egui::Ui,
    id: &str,
    rect: egui::Rect,
    glyph: Icon,
    label: &str,
    selected: bool,
) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let response = ui.interact(rect, ui.scope_id().with(id), egui::Sense::click());
    set_author_id(&response, id);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label)
    });
    if selected || response.hovered() {
        let fill = if selected {
            kit::panel(dark)
        } else {
            kit::hover(dark)
        };
        ui.painter().rect_filled(rect, ROW_RADIUS, fill);
    }
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + 8.0, rect.center().y - kit::icon::DENSE / 2.0),
        egui::Vec2::splat(kit::icon::DENSE),
    );
    kit::tinted(glyph, kit::icon::DENSE, kit::text(dark)).paint_at(ui, icon_rect);
    let galley = line(
        ui,
        label,
        semibold(12.0),
        kit::text(dark),
        rect.width() - 34.0 - 4.0,
    );
    let at = egui::pos2(rect.min.x + 34.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, kit::text(dark));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The left rail: Home, a rule, then the tools that open their own window.
pub(super) fn rail(ui: &mut egui::Ui, rail: egui::Rect, state: &mut State, locale: Locale) {
    let dark = ui.visuals().dark_mode;
    let tr = |key: Text| catalog::text(locale, key);
    let row =
        |y: f32| egui::Rect::from_min_size(egui::pos2(rail.min.x + RAIL_INSET, y), RAIL_ROW.into());
    let home = row(rail.min.y + TOP_BAND);
    nav_row(ui, "home.nav.home", home, Icon::House, tr(Text::Home), true);
    let rule_y = home.max.y + RAIL_RULE_GAP;
    ui.painter().hline(
        home.x_range(),
        rule_y,
        egui::Stroke::new(kit::space::HAIRLINE, kit::separator(dark)),
    );
    let shorts = row(rule_y + RAIL_RULE_GAP);
    if nav_row(
        ui,
        "home.nav.shorts",
        shorts,
        Icon::Scissors,
        tr(Text::Shorts),
        false,
    )
    .clicked()
    {
        state.action = Some(Action::Shorts);
    }
}

/// The top band: the open-project icon at the right and, beside it, the
/// short-video job status when one is running.
pub(super) fn top_bar(
    ui: &mut egui::Ui,
    band: egui::Rect,
    state: &mut State,
    locale: Locale,
    status: Option<Text>,
) {
    let dark = ui.visuals().dark_mode;
    let button = egui::Rect::from_min_size(
        egui::pos2(
            band.max.x - 18.0 - kit::size::CONTROL,
            band.center().y - kit::size::CONTROL / 2.0,
        ),
        egui::Vec2::splat(kit::size::CONTROL),
    );
    let enabled = !state.busy();
    let opened = ui
        .scope_builder(egui::UiBuilder::new().max_rect(button), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                kit::icon_button(
                    ui,
                    "project.open",
                    Icon::FolderOpen,
                    catalog::text(locale, Text::OpenProject),
                    false,
                )
            })
            .inner
        })
        .inner;
    if opened.clicked() {
        state.action = Some(Action::OpenFile);
    }
    if let Some(status) = status {
        ui.painter().text(
            egui::pos2(button.min.x - 12.0, button.center().y),
            egui::Align2::RIGHT_CENTER,
            catalog::text(locale, status),
            egui::FontId::proportional(kit::type_size::BODY),
            kit::secondary(dark),
        );
    }
}
