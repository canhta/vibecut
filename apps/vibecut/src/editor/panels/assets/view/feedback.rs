//! Renders asset feedback.

use super::super::{Action, Icon, Id, Recorded, Rect, State, Text, Vec2, egui, theme};
use crate::editor::{Context, Toast, ToastPlacement};
use crate::i18n;

/// Non-modal import overlay over the media panel. Import controls are already
/// disabled while the batch runs; cancel remains available.
pub(in crate::editor) fn import_overlay(
    state: &State,
    cx: &Context<'_>,
    ctx: &egui::Context,
    rect: Rect,
) -> Vec<Action> {
    if !state.importing() {
        return Vec::new();
    }
    let percent = (state.import_fraction() * 100.0).clamp(0.0, 100.0) as u64;
    let width = (rect.width() - 24.0).max(48.0);
    let mut actions = Vec::new();
    egui::Area::new(Id::unique("import.overlay"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min + Vec2::new(12.0, theme::size::TAB_STRIP + 12.0))
        .show(ctx, |ui| {
            ui.set_width(width);
            egui::Frame::new()
                .fill(theme::panel(cx.dark))
                .stroke(egui::Stroke::new(1.0, theme::border(cx.dark)))
                .corner_radius(theme::radius::CONTROL)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_width((width - 24.0).max(24.0));
                    ui.horizontal(|ui| {
                        let value = i18n::number(cx.locale, percent);
                        let status = if width < 240.0 {
                            format!("{value}%")
                        } else {
                            i18n::format(
                                cx.locale,
                                Text::ProcessingPercent,
                                &[("status", cx.tr(Text::Processing)), ("percent", &value)],
                            )
                        };
                        ui.label(egui::RichText::new(status).size(theme::type_size::BODY));
                        let response = crate::kit::icon_button(
                            ui,
                            "import.cancel",
                            Icon::X,
                            cx.tr(Text::Cancel),
                            false,
                        )
                        .on_hover_text(cx.tr(Text::Cancel));
                        remember(&mut actions, "import.cancel", &response, Some(Icon::X));
                        if response.clicked() {
                            actions.push(Action::Command("import.cancel".into()));
                        }
                    });
                    crate::kit::progress_bar(ui, "import.progress", state.import_fraction());
                    ui.label(
                        egui::RichText::new(i18n::format(
                            cx.locale,
                            Text::ImportProgress,
                            &[
                                (
                                    "done",
                                    &i18n::number(cx.locale, state.import_completed as u64),
                                ),
                                ("total", &i18n::number(cx.locale, state.import_total as u64)),
                            ],
                        ))
                        .size(theme::type_size::BODY)
                        .color(theme::secondary(cx.dark)),
                    );
                });
        });
    actions
}

pub(in crate::editor) fn drop_overlay(
    cx: &Context<'_>,
    ctx: &egui::Context,
    hovering: bool,
    importing: bool,
) {
    if !hovering || importing {
        return;
    }
    let screen = ctx.content_rect();
    egui::Area::new(Id::unique("import.drop"))
        .order(egui::Order::Tooltip)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.set_width(screen.width());
            ui.set_height(screen.height());
            ui.painter()
                .rect_filled(screen, theme::radius::PANEL, theme::scrim());
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(screen.shrink(48.0)).layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                ),
                |ui| {
                    ui.label(
                        egui::RichText::new(cx.tr(Text::DropMedia))
                            .size(18.0)
                            .color(theme::on_scrim()),
                    );
                },
            );
        });
}

pub(in crate::editor) fn toast(
    toast: Option<&Toast>,
    media: Rect,
    cx: &Context<'_>,
    ctx: &egui::Context,
) {
    let Some(toast) = toast else {
        return;
    };
    let rows = |ui: &mut egui::Ui| {
        let response = ui.label(
            egui::RichText::new(toast.text.as_str())
                .size(theme::type_size::CHROME)
                .strong(),
        );
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_author_id("import.toast");
        });
        if let Some(detail) = &toast.detail {
            ui.label(
                egui::RichText::new(detail.as_str())
                    .small()
                    .color(theme::secondary(cx.dark)),
            );
        }
    };
    match toast.placement {
        ToastPlacement::Corner => crate::kit::toast(ctx, "import.toast", rows),
        ToastPlacement::MediaPanel => crate::kit::toast_top(
            ctx,
            "import.toast",
            media.center_top() + Vec2::new(0.0, theme::size::TAB_STRIP + 12.0),
            rows,
        ),
    }
}

fn remember(actions: &mut Vec<Action>, id: &str, response: &egui::Response, glyph: Option<Icon>) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
    actions.push(Action::Record(Box::new(Recorded {
        id: id.to_owned(),
        response: response.clone(),
        glyph,
        tooltip: None,
    })));
}
