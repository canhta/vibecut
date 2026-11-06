//! Painted controls for the shorts window: tap targets, buttons, dashed frame and tick.
use crate::{
    egui::{self, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2},
    kit::{self, Icon},
};

pub(in crate::shorts) fn author(response: &Response, id: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
}

pub(in crate::shorts) fn named(response: &Response, id: &str, label: &str) {
    author(response, id);
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_label(label));
}

/// Semibold face at `size`.
pub(super) fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Inter-SemiBold".into()))
}

pub(super) fn corner(radius: f32) -> egui::CornerRadius {
    egui::CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8)
}

/// An interactive rectangle with a stable automation id, accessible label and tooltip.
pub(super) fn tap(ui: &Ui, id: &str, rect: Rect, label: &str, enabled: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, egui::Id::unique(("shorts.tap", id)), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    author(&response, id);
    response.on_hover_text(label)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    /// Accent fill with `on_accent` ink.
    Primary,
    /// Field fill.
    Field,
    /// No fill until hovered or selected.
    Quiet,
}

/// A rounded button of exact size: optional icon, optional label, centred together.
#[allow(clippy::too_many_arguments)]
pub(super) fn button(
    ui: &mut Ui,
    id: &str,
    rect: Rect,
    glyph: Option<Icon>,
    label: Option<&str>,
    tip: &str,
    look: Look,
    enabled: bool,
) -> Response {
    let response = tap(ui, id, rect, tip, enabled);
    let dark = ui.visuals().dark_mode;
    let (fill, ink) = match look {
        Look::Primary => (Some(kit::accent(dark)), kit::on_accent(dark)),
        Look::Field => (Some(kit::field(dark)), kit::text(dark)),
        Look::Quiet if response.hovered() => (Some(kit::hover(dark)), kit::text(dark)),
        Look::Quiet => (None, kit::text(dark)),
    };
    let ink = if enabled { ink } else { kit::disabled(dark) };
    let fill = fill.map(|c| {
        if enabled && response.hovered() && look != Look::Quiet {
            c.gamma_multiply(0.92)
        } else {
            c
        }
    });
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, corner(8.0), fill);
    }
    let text = label.map(|label| {
        ui.painter()
            .layout_no_wrap(label.to_owned(), bold(14.0), ink)
    });
    let icon_width = if glyph.is_some() { 16.0 } else { 0.0 };
    let gap = if glyph.is_some() && text.is_some() {
        6.0
    } else {
        0.0
    };
    let text_width = text.as_ref().map_or(0.0, |t| t.size().x);
    let mut x = rect.center().x - (icon_width + gap + text_width) / 2.0;
    if let Some(glyph) = glyph {
        kit::tinted(glyph, 16.0, ink).paint_at(
            ui,
            Rect::from_min_size(Pos2::new(x, rect.center().y - 8.0), Vec2::splat(icon_width)),
        );
        x += icon_width + gap;
    }
    if let Some(text) = text {
        let at = Pos2::new(x, rect.center().y - text.size().y / 2.0);
        ui.painter().galley(at, text, ink);
    }
    response
}

/// An icon-only control, filled while `selected`.
pub(super) fn icon_tap(
    ui: &mut Ui,
    id: &str,
    rect: Rect,
    glyph: Icon,
    tip: &str,
    enabled: bool,
    selected: bool,
) -> Response {
    let look = if selected { Look::Field } else { Look::Quiet };
    button(ui, id, rect, Some(glyph), None, tip, look, enabled)
}

/// Points along a rounded rectangle, for a dashed outline.
fn outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (rect.right_top() + Vec2::new(-r, r), -90.0_f32),
        (rect.right_bottom() + Vec2::new(-r, -r), 0.0),
        (rect.left_bottom() + Vec2::new(r, -r), 90.0),
        (rect.left_top() + Vec2::new(r, r), 180.0),
    ];
    let mut points = Vec::new();
    for (center, start) in corners {
        for step in 0..=6 {
            let angle = (start + 15.0 * step as f32).to_radians();
            points.push(center + Vec2::new(angle.cos(), angle.sin()) * r);
        }
    }
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}

pub(super) fn dashed_frame(ui: &Ui, rect: Rect, radius: f32, stroke: Stroke) {
    for shape in egui::Shape::dashed_line(&outline(rect, radius), stroke, 4.0, 4.0) {
        ui.painter().add(shape);
    }
}

pub(super) fn tick(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    painter.add(egui::Shape::line(
        vec![
            center + Vec2::new(-s * 0.9, 0.0),
            center + Vec2::new(-s * 0.3, s * 0.6),
            center + Vec2::new(s * 0.9, -s * 0.6),
        ],
        Stroke::new(1.6, color),
    ));
}

/// A rounded rectangle turned by `angle` radians, as a filled polygon.
pub(super) fn tilted_card(
    painter: &egui::Painter,
    center: Pos2,
    size: Vec2,
    angle: f32,
    fill: Color32,
    stroke: Stroke,
) {
    let (sin, cos) = angle.sin_cos();
    let local = outline(Rect::from_center_size(Pos2::ZERO, size), 8.0);
    let points = local
        .into_iter()
        .map(|p| center + Vec2::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos))
        .collect();
    painter.add(egui::Shape::convex_polygon(points, fill, stroke));
}
