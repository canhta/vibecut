//! The working screen: placeholders for the coming shorts, a progress bar and the upload approval.
use super::{
    State,
    widgets::{self, Look, bold, corner},
};
use crate::{
    catalog::{self, Text},
    egui::{self, Align2, Rect, pos2, vec2},
    kit::{self, Icon},
};
use engine::providers::{CatalogOperation, Cost};

/// Measured at the 1718×989 window (reference 221946), from the content origin.
pub(super) const GRID: f32 = 1359.0;
pub(super) const GAP: f32 = 25.0;
pub(super) const COLUMNS: usize = 4;
pub(super) const TITLE_Y: f32 = 34.0;
const BAR_TOP: f32 = 68.0;
const BAR: [f32; 2] = [868.0, 16.0];
pub(super) const CARDS_TOP: f32 = 101.0;
pub(super) const ROW_PITCH: f32 = 687.0;
const TEXT_LINES: [(f32, f32, f32); 3] =
    [(16.0, 17.0, 1.0), (39.0, 16.0, 0.60), (65.0, 14.0, 0.50)];

pub(super) fn show(state: &mut State, ui: &mut egui::Ui, rect: Rect) {
    let dark = state.preferences.dark;
    let painter = ui.painter().clone();
    let width = GRID.min((rect.width() - 80.0).max(480.0));
    let left = rect.center().x - width / 2.0;
    let cancelling = state.cancelling();
    let waiting = state.awaiting_consent();
    let skeleton = kit::field(dark);
    let faint = kit::field(dark);

    let back = Rect::from_center_size(pos2(left + 17.0, rect.top() + TITLE_Y), vec2(32.0, 32.0));
    if widgets::icon_tap(
        ui,
        "shorts.cancel",
        back,
        Icon::ArrowLeft,
        state.tr(Text::Cancel),
        !cancelling,
        false,
    )
    .clicked()
    {
        state.cancel();
    }
    let title_x = left + 44.0;
    let count = catalog::format(
        state.preferences.locale,
        Text::ShortsCountLabel,
        &[("count", "0")],
    );
    let accent = painter.text(
        pos2(title_x, rect.top() + TITLE_Y),
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

    // The progress bar fills the slot the reference leaves as a placeholder.
    let bar = Rect::from_min_size(
        pos2(left, rect.top() + BAR_TOP),
        vec2(BAR[0].min(width), BAR[1]),
    );
    painter.rect_filled(bar, corner(4.0), skeleton);
    let fraction = state
        .generating
        .as_ref()
        .and_then(|job| job.events.state.lock().ok().and_then(|s| s.fraction))
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    if fraction > 0.0 {
        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
        painter.rect_filled(fill, corner(4.0), kit::accent(dark).gamma_multiply(0.35));
    }
    painter.text(
        pos2(bar.left() + 8.0, bar.center().y),
        Align2::LEFT_CENTER,
        state.tr(if cancelling {
            Text::ShortsCancelling
        } else {
            Text::ShortsCreating
        }),
        egui::FontId::proportional(12.0),
        kit::secondary(dark),
    );
    painter.text(
        pos2(bar.right() - 8.0, bar.center().y),
        Align2::RIGHT_CENTER,
        catalog::format(
            state.preferences.locale,
            Text::ShortsPercent,
            &[(
                "value",
                &catalog::number(state.preferences.locale, (fraction * 100.0).round() as u64),
            )],
        ),
        egui::FontId::proportional(12.0),
        kit::text(dark),
    );

    let card_width = (width - GAP * (COLUMNS as f32 - 1.0)) / COLUMNS as f32;
    let card_height = card_width * 16.0 / 9.0;
    let clip = painter.with_clip_rect(rect);
    for row in 0..2 {
        let top = rect.top() + CARDS_TOP + ROW_PITCH * row as f32;
        for column in 0..COLUMNS {
            let x = left + (card_width + GAP) * column as f32;
            clip.rect_filled(
                Rect::from_min_size(pos2(x, top), vec2(card_width, card_height)),
                corner(12.0),
                skeleton,
            );
            for (offset, line, share) in TEXT_LINES {
                clip.rect_filled(
                    Rect::from_min_size(
                        pos2(x, top + card_height + offset),
                        vec2(card_width * share, line),
                    ),
                    corner(2.0),
                    faint,
                );
            }
        }
    }
    if waiting {
        consent(state, ui, rect);
    }
}

/// The exact-upload approval: what leaves this computer, to whom, at what cost.
fn consent(state: &mut State, ui: &mut egui::Ui, rect: Rect) {
    let dark = state.preferences.dark;
    let locale = state.preferences.locale;
    ui.painter().rect_filled(rect, 0.0, kit::scrim());
    let mut answer = None;
    egui::Area::new(egui::Id::unique("shorts.consent"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(rect.center().x - 220.0, rect.top() + 220.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(kit::modal(dark))
                .corner_radius(12)
                .inner_margin(24.0)
                .show(ui, |ui| {
                    ui.set_width(392.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let Some(job) = &state.generating else { return };
                    let Ok(guard) = job.events.state.lock() else {
                        return;
                    };
                    let Some(upload) = &guard.upload else { return };
                    let request = &upload.request;
                    ui.label(
                        egui::RichText::new(catalog::format(
                            locale,
                            Text::ShortsUploadIdentity,
                            &[
                                ("provider", &request.provider),
                                ("model", &request.model),
                                ("languages", &request.languages),
                            ],
                        ))
                        .font(bold(14.0)),
                    );
                    ui.label(
                        state.tr(if request.operation == CatalogOperation::Transcribe {
                            Text::ShortsUploadAudio
                        } else {
                            Text::ShortsUploadTranscript
                        }),
                    );
                    for (label, value) in [
                        (
                            Text::ShortsSeconds,
                            catalog::decimal(locale, request.audio_seconds, 2),
                        ),
                        (
                            Text::ShortsBytes,
                            catalog::number(locale, request.upload_bytes),
                        ),
                        (Text::ShortsCost, cost_text(locale, &request.cost)),
                    ] {
                        ui.label(catalog::format(
                            locale,
                            Text::ShortsValue,
                            &[("label", state.tr(label)), ("value", &value)],
                        ));
                    }
                    for (index, (kind, url)) in request.terms.iter().enumerate() {
                        let label = match *kind {
                            "policy" | "retention_training_policy" => Text::ShortsPolicy,
                            "pricing" => Text::ShortsCost,
                            "endpoint" => continue,
                            _ => Text::Details,
                        };
                        if url.starts_with("https://") {
                            let r = ui.hyperlink_to(state.tr(label), url);
                            widgets::author(&r, &format!("shorts.consent.policy.{index}"));
                        }
                    }
                    ui.add_space(8.0);
                    let row = ui.allocate_space(vec2(392.0, 40.0)).1;
                    let approve = Rect::from_min_size(row.min, vec2(200.0, 40.0));
                    let deny = Rect::from_min_size(row.min + vec2(208.0, 0.0), vec2(184.0, 40.0));
                    if widgets::button(
                        ui,
                        "shorts.consent.approve",
                        approve,
                        Some(Icon::UploadSimple),
                        Some(state.tr(Text::ShortsConsent)),
                        state.tr(Text::ShortsConsent),
                        Look::Primary,
                        true,
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if widgets::button(
                        ui,
                        "shorts.consent.deny",
                        deny,
                        None,
                        Some(state.tr(Text::ShortsDeny)),
                        state.tr(Text::ShortsDeny),
                        Look::Field,
                        true,
                    )
                    .clicked()
                    {
                        answer = Some(false);
                    }
                });
        });
    if let Some(approve) = answer
        && let Err(e) = state.approve_upload(approve)
    {
        state.fail(e, Text::ShortsError);
    }
}

pub(super) fn cost_text(locale: catalog::Locale, cost: &Cost) -> String {
    match cost {
        Cost::Free => catalog::text(locale, Text::ShortsFree).into(),
        Cost::Unknown => catalog::text(locale, Text::ShortsUnknownCost).into(),
        Cost::UpperBound { currency, micros } => catalog::format(
            locale,
            Text::ShortsMoney,
            &[
                ("currency", currency),
                (
                    "amount",
                    &catalog::decimal(locale, *micros as f64 / 1_000_000.0, 6),
                ),
            ],
        ),
    }
}
