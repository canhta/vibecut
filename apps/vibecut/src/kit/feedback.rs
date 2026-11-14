//! Renders the surfaces that tell a creator what a panel is doing.
//!
//! Callers own the strings (an icon plus at most four words), the glyph and
//! the automation IDs.

use super::widgets::author;
use super::{Icon, icon, radius, size, space, tinted, type_size};
use eframe::egui::{self, Align2, FontId, Rect, Response, Sense, Ui, Vec2};

/// How a panel state reads: quiet for empty, loading and offline, error for a
/// failure the creator must notice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tone {
    Quiet,
    Error,
}

/// An empty, loading, offline or error face: a 24 px glyph over a 12 px line,
/// centred in `rect`. It paints only and never moves the layout cursor.
pub(crate) fn panel_state(
    ui: &mut Ui,
    id: &str,
    rect: Rect,
    glyph: Icon,
    text: &str,
    tone: Tone,
) -> Response {
    let dark = ui.visuals().dark_mode;
    let color = match tone {
        Tone::Quiet => super::secondary(dark),
        Tone::Error => super::error_text(dark),
    };
    let galley = ui.painter().layout(
        text.to_owned(),
        FontId::proportional(type_size::BODY),
        color,
        (rect.width() - space::MD * 2.0).max(space::XL),
    );
    let height = icon::STATE + space::MD + galley.size().y;
    let top = rect.center().y - height / 2.0;
    tinted(glyph, icon::STATE, color).paint_at(
        ui,
        Rect::from_center_size(
            egui::pos2(rect.center().x, top + icon::STATE / 2.0),
            Vec2::splat(icon::STATE),
        ),
    );
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            top + icon::STATE + space::MD,
        ),
        galley,
        color,
    );
    let response = ui.interact(rect, ui.make_persistent_id(id), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    author(&response, id);
    response
}

/// A 2 px accent bar over the field track across the available width.
pub(crate) fn progress_bar(ui: &mut Ui, id: &str, fraction: f32) -> Response {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), size::PROGRESS_BAR),
        Sense::hover(),
    );
    let dark = ui.visuals().dark_mode;
    ui.painter()
        .rect_filled(rect, radius::PANEL, super::field(dark));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height())),
        radius::PANEL,
        super::accent(dark),
    );
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::new(egui::Role::ProgressIndicator);
        info.value = Some(f64::from(fraction * 100.0).floor());
        info
    });
    author(&response, id);
    response
}

/// A 320 px bottom-right toast on the modal surface. The caller draws the rows.
pub(crate) fn toast<R>(
    ctx: &egui::Context,
    key: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let area = egui::Area::new(egui::Id::unique(key))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::splat(-space::XL));
    surface(ctx, area, add_contents)
}

/// The same 320 px toast, hung from `top_centre` for a surface that owns the strip
/// at its top (the media panel). The caller draws the rows.
pub(crate) fn toast_top<R>(
    ctx: &egui::Context,
    key: &str,
    top_centre: egui::Pos2,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let area = egui::Area::new(egui::Id::unique(key))
        .fixed_pos(top_centre)
        .pivot(Align2::CENTER_TOP);
    surface(ctx, area, add_contents)
}

fn surface<R>(ctx: &egui::Context, area: egui::Area, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let dark = ctx.global_style().visuals.dark_mode;
    area.order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(super::modal(dark))
                .stroke(egui::Stroke::new(space::HAIRLINE, super::border(dark)))
                .corner_radius(radius::DIALOG)
                .inner_margin(space::LG)
                .show(ui, |ui| {
                    ui.set_width(size::TOAST_WIDTH - space::LG * 2.0);
                    add_contents(ui)
                })
                .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn harness<'a>(app: impl FnMut(&mut Ui, &mut ()) + 'a) -> Harness<'a, ()> {
        Harness::builder()
            .with_context_setup(crate::kit::install_fonts)
            .build_ui_state(app, ())
    }

    #[test]
    fn panel_state_is_named_by_its_words_in_every_tone() {
        for tone in [Tone::Quiet, Tone::Error] {
            let mut harness = harness(move |ui, _| {
                let rect = ui.max_rect();
                panel_state(ui, "state", rect, Icon::FolderOpen, "Nothing here", tone);
            });
            harness.run();
            harness.get_by_label("Nothing here");
        }
    }

    #[test]
    fn progress_bar_keeps_its_height_for_any_fraction() {
        for fraction in [-1.0, 0.0, 0.5, 2.0, f32::NAN] {
            let mut harness = harness(move |ui, _| {
                progress_bar(ui, "bar", fraction);
            });
            harness.run();
            let rect = harness.get_by_role(egui::Role::ProgressIndicator).rect();
            assert!(
                (rect.height() - size::PROGRESS_BAR).abs() < f32::EPSILON,
                "{fraction}"
            );
        }
    }

    #[test]
    fn toast_sits_bottom_right_inside_its_width() {
        let mut harness = harness(|ui, _| {
            toast(ui.ctx(), "toast", |ui| ui.label("Saved"));
        });
        harness.run();
        let screen = harness.ctx.content_rect();
        let rect = harness.get_by_label("Saved").rect();
        assert!(rect.width() <= size::TOAST_WIDTH, "{rect:?}");
        assert!(rect.right() <= screen.right() - space::XL + 0.5, "{rect:?}");
        assert!(
            rect.bottom() <= screen.bottom() - space::XL + 0.5,
            "{rect:?}"
        );
    }
}
