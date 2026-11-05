//! Transient source-normalized crop interaction; render remains the geometry owner.
use crate::{Error, Result, egui};

/// Largest source rectangle matching a portrait 9:16 output, initially centered.
/// The dimensions are source pixels; zero sizes and non-finite input refuse.
pub fn portrait(width: f64, height: f64) -> Result<render::Rect> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(Error::Invalid("crop source dimensions"));
    }
    let aspect = width / height;
    let (w, h) = if aspect > 9.0 / 16.0 {
        (9.0 / 16.0 / aspect, 1.0)
    } else {
        (1.0, aspect / (9.0 / 16.0))
    };
    let crop = render::Rect {
        x: (1.0 - w) / 2.0,
        y: (1.0 - h) / 2.0,
        width: w,
        height: h,
    };
    crop.validate_crop()?;
    Ok(crop)
}

/// Drag the portrait crop within the normalized source bounds. Caller retains
/// the gesture-entry rectangle and commits a single undo step on drag stop.
/// Keyboard arrows move by 1% (Shift: 10%); render consumes the returned rectangle.
pub fn control(
    ui: &mut egui::Ui,
    id: &str,
    crop: &mut render::Rect,
    aspect: f32,
    label: &str,
) -> Result<egui::Response> {
    crop.validate_crop()?;
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(Error::Invalid("crop source aspect"));
    }
    let size = egui::vec2(ui.available_width(), ui.available_width() / aspect);
    if !size.x.is_finite() || !size.y.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
        return Err(Error::Invalid("crop control bounds"));
    }
    let (bounds, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_author_id(id);
        node.set_label(label);
        node.add_action(egui::accesskit::Action::Focus);
    });
    if response.clicked() {
        response.request_focus();
    }
    let mut delta = response.drag_delta();
    if response.has_focus() {
        let step = if ui.input(|i| i.modifiers.shift) {
            0.1
        } else {
            0.01
        };
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowLeft) {
                delta.x -= bounds.width() * step;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                delta.x += bounds.width() * step;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                delta.y -= bounds.height() * step;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                delta.y += bounds.height() * step;
            }
        });
    }
    crop.x = (crop.x + f64::from(delta.x / bounds.width())).clamp(0.0, 1.0 - crop.width);
    crop.y = (crop.y + f64::from(delta.y / bounds.height())).clamp(0.0, 1.0 - crop.height);
    let painter = ui.painter();
    painter.rect_filled(bounds, 4.0, ui.visuals().extreme_bg_color);
    let selected = egui::Rect::from_min_size(
        bounds.min
            + egui::vec2(
                bounds.width() * crop.x as f32,
                bounds.height() * crop.y as f32,
            ),
        egui::vec2(
            bounds.width() * crop.width as f32,
            bounds.height() * crop.height as f32,
        ),
    );
    painter.rect_filled(
        selected,
        0.0,
        ui.visuals().selection.bg_fill.gamma_multiply(0.2),
    );
    painter.rect_stroke(
        selected,
        0.0,
        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
        egui::StrokeKind::Inside,
    );
    for t in [1.0 / 3.0, 2.0 / 3.0] {
        let stroke = egui::Stroke::new(1.0, ui.visuals().weak_text_color());
        painter.line_segment(
            [
                selected.lerp_inside(egui::vec2(t, 0.0)),
                selected.lerp_inside(egui::vec2(t, 1.0)),
            ],
            stroke,
        );
        painter.line_segment(
            [
                selected.lerp_inside(egui::vec2(0.0, t)),
                selected.lerp_inside(egui::vec2(1.0, t)),
            ],
            stroke,
        );
    }
    Ok(response.on_hover_text(label))
}
