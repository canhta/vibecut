//! Draws the create banner and the tool card that start new work.

use super::{
    AGENT_CARD, BANNER_HEIGHT, CARD_RADIUS, TOOL_CARD,
    widgets::{line, semibold, set_author_id},
};
use crate::i18n;
use crate::{
    egui,
    home::state::{Action, State},
    i18n::{Locale, Text},
    kit::{self, Icon},
};

/// The full-width banner: one click creates and opens a project.
pub(super) fn banner(ui: &mut egui::Ui, width: f32, state: &mut State, locale: Locale) {
    let dark = ui.visuals().dark_mode;
    let label = i18n::text(locale, Text::NewProject);
    let enabled = !state.busy();
    let creating = state.creating;
    let id = "project.create";
    let response = ui
        .push_id(id, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, BANNER_HEIGHT), egui::Sense::click());
                let fill = if response.hovered() && enabled {
                    kit::accent(dark).lerp_to_gamma(egui::Color32::WHITE, 0.12)
                } else {
                    kit::accent(dark)
                };
                ui.painter().rect_filled(rect, CARD_RADIUS, fill);
                let ink = kit::on_accent(dark);
                if creating {
                    let text = egui::RichText::new(i18n::text(locale, Text::LoadingProject))
                        .font(semibold(kit::type_size::CHROME))
                        .color(ink);
                    let width_of = ui
                        .painter()
                        .layout_no_wrap(
                            text.text().to_owned(),
                            semibold(kit::type_size::CHROME),
                            ink,
                        )
                        .size()
                        .x;
                    let left = rect.center().x - (24.0 + 12.0 + width_of) / 2.0;
                    let spinner = egui::Rect::from_min_size(
                        egui::pos2(left, rect.center().y - 12.0),
                        egui::Vec2::splat(24.0),
                    );
                    ui.put(spinner, egui::Spinner::new().color(ink));
                    let label_rect = egui::Rect::from_min_size(
                        egui::pos2(left + 36.0, rect.center().y - 10.0),
                        egui::vec2(width_of + 4.0, 20.0),
                    );
                    ui.put(label_rect, egui::Label::new(text));
                } else {
                    let galley = line(ui, label, semibold(24.0), ink, width - 120.0);
                    let tile = 28.0;
                    let gap = 12.0;
                    let left = rect.center().x - (tile + gap + galley.size().x) / 2.0;
                    let tile_rect = egui::Rect::from_min_size(
                        egui::pos2(left, rect.center().y - tile / 2.0),
                        egui::Vec2::splat(tile),
                    );
                    ui.painter().rect_filled(tile_rect, 6.0, ink);
                    kit::tinted(Icon::Plus, kit::icon::DENSE, kit::text(dark)).paint_at(
                        ui,
                        egui::Rect::from_center_size(
                            tile_rect.center(),
                            egui::Vec2::splat(kit::icon::DENSE),
                        ),
                    );
                    let at = egui::pos2(
                        tile_rect.max.x + gap,
                        rect.center().y - galley.size().y / 2.0,
                    );
                    ui.painter().galley(at, galley, ink);
                }
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
                });
                response
            })
            .inner
        })
        .inner;
    set_author_id(&response, id);
    if response.clicked() {
        state.request_create(label.to_owned(), None);
    }
}

/// The Long-video-to-shorts card: icon tile, title and one line of purpose.
pub(super) fn shorts_card(ui: &mut egui::Ui, state: &mut State, locale: Locale) {
    let dark = ui.visuals().dark_mode;
    let id = "home.shorts";
    let title = i18n::text(locale, Text::Shorts);
    let (rect, response) = ui
        .push_id(id, |ui| {
            ui.allocate_exact_size(TOOL_CARD.into(), egui::Sense::click())
        })
        .inner;
    set_author_id(&response, id);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let fill = if response.hovered() {
        kit::hover(dark)
    } else {
        kit::panel(dark)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CARD_RADIUS, fill);
    let tile =
        egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 12.0), egui::Vec2::splat(44.0));
    painter.rect_filled(tile, 8.0, kit::field(dark));
    kit::tinted(Icon::Scissors, 24.0, kit::text(dark)).paint_at(
        ui,
        egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(24.0)),
    );
    let inner = rect.width() - 24.0;
    let name = line(ui, title, semibold(13.0), kit::text(dark), inner);
    ui.painter().galley(
        egui::pos2(rect.min.x + 12.0, rect.min.y + 70.0),
        name,
        kit::text(dark),
    );
    let hint = line(
        ui,
        i18n::text(locale, Text::HomeShortsHint),
        egui::FontId::proportional(kit::type_size::BODY),
        kit::secondary(dark),
        inner,
    );
    ui.painter().galley(
        egui::pos2(rect.min.x + 12.0, rect.min.y + 94.0),
        hint,
        kit::secondary(dark),
    );
    if response.clicked() {
        state.action = Some(Action::Shorts);
    }
}

/// The agent prompt card, 380×136: a title row, a headline and a 356×40 input (12 px in, 12 px
/// above the bottom) with a 24 px send button 10 px from its right edge (the reference's card
/// at 221432). Sending creates a project, opens its editor and starts the agent's first turn
/// with the typed prompt.
pub(super) fn agent_card(ui: &mut egui::Ui, state: &mut State, locale: Locale) {
    let dark = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(AGENT_CARD.into(), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, CARD_RADIUS, kit::panel(dark));
    let glyph =
        egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 11.0), egui::Vec2::splat(18.0));
    kit::tinted(Icon::Sparkle, 18.0, kit::accent(dark)).paint_at(ui, glyph);
    let title = line(
        ui,
        i18n::text(locale, Text::AgentTitle),
        semibold(kit::type_size::BODY),
        kit::text(dark),
        AGENT_CARD[0] - 60.0,
    );
    ui.painter().galley(
        egui::pos2(rect.min.x + 38.0, rect.min.y + 20.0 - title.size().y / 2.0),
        title,
        kit::text(dark),
    );
    let headline = line(
        ui,
        i18n::text(locale, Text::HomeAgentHeadline),
        semibold(kit::type_size::CHROME),
        kit::text(dark),
        AGENT_CARD[0] - 24.0,
    );
    ui.painter().galley(
        egui::pos2(
            rect.min.x + 12.0,
            rect.min.y + 52.0 - headline.size().y / 2.0,
        ),
        headline,
        kit::text(dark),
    );
    let field = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + 12.0, rect.max.y - 12.0 - 40.0),
        egui::vec2(AGENT_CARD[0] - 24.0, 40.0),
    );
    ui.painter().rect_filled(field, 8.0, kit::field(dark));
    let enabled = !state.busy();
    let send_rect = egui::Rect::from_center_size(
        egui::pos2(field.right() - 10.0 - 12.0, field.center().y),
        egui::Vec2::splat(24.0),
    );
    let text_rect = egui::Rect::from_min_max(
        field.min + egui::vec2(10.0, 0.0),
        egui::pos2(send_rect.left() - 8.0, field.bottom()),
    );
    let mut text_ui = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
    {
        // The card paints the field's box, so the edit draws none.
        let visuals = text_ui.visuals_mut();
        visuals.extreme_bg_color = egui::Color32::TRANSPARENT;
        visuals.selection.stroke = egui::Stroke::NONE;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    }
    let edit = text_ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(&mut state.agent_prompt)
            .hint_text(i18n::text(locale, Text::HomeAgentPlaceholder))
            .char_limit(8 * 1024)
            .desired_width(text_rect.width()),
    );
    set_author_id(&edit, "home.agent.input");
    let send_id = "home.agent.send";
    let sendable = enabled && !state.agent_prompt.trim().is_empty();
    let send = ui
        .push_id(send_id, |ui| {
            ui.add_enabled_ui(sendable, |ui| {
                let response = ui.allocate_rect(send_rect, egui::Sense::click());
                let fill = if response.hovered() {
                    kit::accent(dark).lerp_to_gamma(egui::Color32::WHITE, 0.12)
                } else {
                    kit::accent(dark)
                };
                ui.painter().circle_filled(send_rect.center(), 12.0, fill);
                kit::tinted(Icon::CaretRight, kit::icon::DENSE, kit::on_accent(dark)).paint_at(
                    ui,
                    egui::Rect::from_center_size(
                        send_rect.center(),
                        egui::Vec2::splat(kit::icon::DENSE),
                    ),
                );
                let label = i18n::text(locale, Text::HomeAgentSend);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
                });
                response.on_hover_text(label)
            })
            .inner
        })
        .inner;
    set_author_id(&send, send_id);
    let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if (send.clicked() || entered) && sendable {
        let prompt = state.agent_prompt.trim().to_owned();
        state.agent_prompt.clear();
        state.request_create(
            i18n::text(locale, Text::NewProject).to_owned(),
            Some(prompt),
        );
    }
}
