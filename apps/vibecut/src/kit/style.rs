//! Installs the shared app appearance without window state.

use super as theme;
use eframe::egui::{self, FontId, Vec2};
use std::sync::Arc;

pub(crate) fn install_theme(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let text = theme::text(dark);
    let border = theme::border(dark);
    let field = theme::field(dark);
    let hover = theme::hover(dark);
    let separator = theme::separator(dark);
    let stroke = |color| egui::Stroke::new(1.0, color);
    visuals.panel_fill = theme::panel(dark);
    visuals.window_fill = theme::modal(dark);
    visuals.window_stroke = stroke(border);
    visuals.extreme_bg_color = field;
    visuals.faint_bg_color = separator;
    visuals.code_bg_color = field;
    visuals.warn_fg_color = theme::warning(dark);
    visuals.error_fg_color = theme::error_text(dark);
    visuals.hyperlink_color = theme::accent(dark);
    visuals.override_text_color = Some(text);
    visuals.selection.bg_fill = theme::accent(dark);
    visuals.selection.stroke = stroke(theme::on_accent(dark));
    visuals.text_cursor.stroke = stroke(theme::accent(dark));
    visuals.widgets.noninteractive.bg_fill = theme::panel(dark);
    visuals.widgets.noninteractive.weak_bg_fill = theme::panel(dark);
    visuals.widgets.noninteractive.bg_stroke = stroke(separator);
    visuals.widgets.noninteractive.fg_stroke = stroke(theme::secondary(dark));
    visuals.widgets.inactive.bg_fill = field;
    visuals.widgets.inactive.weak_bg_fill = field;
    visuals.widgets.inactive.bg_stroke = stroke(border);
    visuals.widgets.inactive.fg_stroke = stroke(text);
    visuals.widgets.hovered.bg_fill = hover;
    visuals.widgets.hovered.weak_bg_fill = hover;
    visuals.widgets.hovered.bg_stroke = stroke(border);
    visuals.widgets.hovered.fg_stroke = stroke(text);
    visuals.widgets.active.bg_fill = hover;
    visuals.widgets.active.weak_bg_fill = hover;
    visuals.widgets.active.bg_stroke = stroke(theme::accent(dark));
    visuals.widgets.active.fg_stroke = stroke(text);
    visuals.widgets.open.bg_fill = hover;
    visuals.widgets.open.weak_bg_fill = hover;
    visuals.widgets.open.bg_stroke = stroke(border);
    visuals.widgets.open.fg_stroke = stroke(text);
    ctx.set_visuals(visuals);
    // The style is shared by every window on the context: each window resets the
    // editor density below to the toolkit defaults before it draws.
    ctx.all_styles_mut(|style| {
        let defaults = egui::Style::default();
        style.text_styles = defaults.text_styles;
        style.spacing.interact_size = defaults.spacing.interact_size;
        style.spacing.item_spacing = defaults.spacing.item_spacing;
        style.visuals.widgets.inactive.corner_radius =
            defaults.visuals.widgets.inactive.corner_radius;
        style.visuals.widgets.hovered.corner_radius =
            defaults.visuals.widgets.hovered.corner_radius;
        style.visuals.widgets.active.corner_radius = defaults.visuals.widgets.active.corner_radius;
    });
}

/// Measured editor type, control height, spacing and corners; editor-only.
pub(crate) fn install_editor_density(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        for text_style in [
            egui::TextStyle::Body,
            egui::TextStyle::Button,
            egui::TextStyle::Small,
        ] {
            style
                .text_styles
                .insert(text_style, FontId::proportional(theme::type_size::BODY));
        }
        style.spacing.interact_size.y = theme::size::FIELD;
        style.spacing.item_spacing = Vec2::new(theme::space::MD, theme::space::XS);
        let corner = egui::CornerRadius::same(theme::radius::CONTROL);
        style.visuals.widgets.inactive.corner_radius = corner;
        style.visuals.widgets.hovered.corner_radius = corner;
        style.visuals.widgets.active.corner_radius = corner;
    });
}
pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert(
        "Inter".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    fonts.families.insert(
        egui::FontFamily::Name("Inter-SemiBold".into()),
        vec!["Inter-SemiBold".into(), "CJK".into()],
    );
    fonts.font_data.insert(
        "CJK".into(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/VibeCutSansCJK-Regular.otf"
        ))),
    );
    fonts.families.insert(
        egui::FontFamily::Proportional,
        vec!["Inter".into(), "CJK".into()],
    );
    fonts
        .families
        .insert(egui::FontFamily::Monospace, vec!["Inter".into()]);
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(ctx: &egui::Context) -> f32 {
        ctx.global_style().text_styles[&egui::TextStyle::Body].size
    }

    #[test]
    fn editor_density_stays_out_of_other_windows() {
        let ctx = egui::Context::default();
        let default = body(&ctx);
        install_theme(&ctx, true);
        assert_eq!(body(&ctx), default, "Home and Shorts keep toolkit defaults");
        install_editor_density(&ctx);
        assert_eq!(body(&ctx), theme::type_size::BODY);
        assert_eq!(
            ctx.global_style().spacing.interact_size.y,
            theme::size::FIELD
        );
        install_theme(&ctx, true);
        assert_eq!(body(&ctx), default, "the next window resets the density");
        assert_eq!(
            ctx.global_style().spacing.interact_size.y,
            egui::Style::default().spacing.interact_size.y
        );
    }
}
