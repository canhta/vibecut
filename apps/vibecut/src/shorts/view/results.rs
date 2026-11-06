//! The results grid: every suggested short as a card, on the working page's own geometry.
use super::{
    State,
    progress::{CARDS_TOP, COLUMNS, GAP, GRID, ROW_PITCH, TITLE_Y},
    review::{card_ui, export_button, export_width},
    widgets::{self, bold},
};
use crate::{
    catalog::{self, Text},
    egui::{self, Align2, Rect, Sense, pos2, vec2},
    kit::{self, Icon},
};

pub(super) fn show(state: &mut State, ui: &mut egui::Ui, rect: Rect) {
    let dark = state.preferences.dark;
    let width = GRID.min((rect.width() - 80.0).max(480.0));
    let left = rect.center().x - width / 2.0;
    let painter = ui.painter().clone();

    let back = Rect::from_center_size(pos2(left + 17.0, rect.top() + TITLE_Y), vec2(32.0, 32.0));
    if widgets::icon_tap(
        ui,
        "shorts.setup",
        back,
        Icon::ArrowLeft,
        state.tr(Text::ShortsBack),
        !state.busy(),
        false,
    )
    .clicked()
        && let Err(e) = state.pick_video(state.request.path.clone())
    {
        state.fail(e, Text::ShortsError);
    }
    let count = catalog::format(
        state.preferences.locale,
        Text::ShortsCountLabel,
        &[(
            "count",
            &catalog::number(state.preferences.locale, state.reviews.len() as u64),
        )],
    );
    let accent = painter.text(
        pos2(left + 44.0, rect.top() + TITLE_Y),
        Align2::LEFT_CENTER,
        count,
        bold(18.0),
        kit::accent(dark),
    );
    painter.text(
        pos2(accent.right() + 6.0, rect.top() + TITLE_Y),
        Align2::LEFT_CENTER,
        state.tr(Text::ShortsCreated),
        bold(18.0),
        kit::text(dark),
    );
    let export_w = export_width(ui, state);
    let export = Rect::from_min_size(
        pos2(left + width - export_w, rect.top() + TITLE_Y - 20.0),
        vec2(export_w, 40.0),
    );
    export_button(ui, state, export);
    if state.reviews.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            state.tr(Text::ShortsNoResults),
            egui::FontId::proportional(14.0),
            kit::secondary(dark),
        );
        return;
    }

    let card_width = (width - GAP * (COLUMNS as f32 - 1.0)) / COLUMNS as f32;
    let card_height = card_width * 16.0 / 9.0;
    let region = Rect::from_min_max(pos2(rect.left(), rect.top() + 68.0), rect.max);
    let rows = state.reviews.len().div_ceil(COLUMNS) as f32;
    let mut opened = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("shorts.grid")
            .max_rect(region),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("shorts.results")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (area, _) = ui.allocate_exact_size(
                        vec2(region.width(), CARDS_TOP - 68.0 + rows * ROW_PITCH),
                        Sense::hover(),
                    );
                    let Some(source) = &state.source else { return };
                    for (index, (candidate, review)) in
                        source.candidates.iter().zip(&state.reviews).enumerate()
                    {
                        let origin = pos2(
                            left + (card_width + GAP) * (index % COLUMNS) as f32,
                            area.top() + (CARDS_TOP - 68.0) + (index / COLUMNS) as f32 * ROW_PITCH,
                        );
                        let card = Rect::from_min_size(origin, vec2(card_width, card_height));
                        let title = candidate.proposal.title.as_str();
                        let response = ui
                            .interact(
                                card.expand2(vec2(0.0, 80.0)),
                                egui::Id::unique(("shorts.candidate", index)),
                                Sense::click(),
                            )
                            .on_hover_text(candidate.proposal.reason.as_str());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, title)
                        });
                        widgets::author(&response, &format!("shorts.candidate.{index}"));
                        if response.clicked() {
                            opened = Some(index);
                        }
                        let poster = super::poster(&mut state.thumbs, ui.ctx(), index);
                        card_ui(ui, card, title, 16.0, review, false, poster);
                    }
                });
        },
    );
    if let Some(index) = opened {
        match state.select(index) {
            Ok(()) => state.editing = true,
            Err(e) => state.fail(e, Text::ShortsError),
        }
    }
}
