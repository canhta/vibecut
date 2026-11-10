//! Transient source-normalized crop interaction; project owns the contract.
mod dialog;

use crate::{Error, Result, egui};
pub use dialog::{Aspect, Inspection};
pub(crate) use dialog::{Dialog, available, open, show};

/// Smallest crop edge, as a fraction of the source, so a handle never collapses
/// the rectangle.
const MIN_SIZE: f64 = 0.05;
/// Pointer distance to an edge or corner that resizes instead of moving.
const HANDLE_DISTANCE: f32 = 10.0;
/// Drawn handle square, centred on its edge or corner.
const HANDLE_SIZE: f32 = 8.0;

/// Largest source rectangle matching a portrait 9:16 output, initially centered.
/// The dimensions are source pixels; zero sizes and non-finite input refuse.
pub fn portrait(width: f64, height: f64) -> Result<project::Crop> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(Error::Invalid("crop source dimensions"));
    }
    let aspect = width / height;
    let (w, h) = if aspect > 9.0 / 16.0 {
        (9.0 / 16.0 / aspect, 1.0)
    } else {
        (1.0, aspect / (9.0 / 16.0))
    };
    let crop = project::Crop {
        x: (1.0 - w) / 2.0,
        y: (1.0 - h) / 2.0,
        width: w,
        height: h,
    };
    if !crop.valid() {
        return Err(Error::Invalid("normalized rectangle"));
    }
    Ok(crop)
}

/// What the pointer grabbed at the start of a crop gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    Move,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Grab {
    fn cursor(self) -> egui::CursorIcon {
        match self {
            Self::Move => egui::CursorIcon::Grab,
            Self::Left | Self::Right => egui::CursorIcon::ResizeHorizontal,
            Self::Top | Self::Bottom => egui::CursorIcon::ResizeVertical,
            Self::TopLeft | Self::BottomRight => egui::CursorIcon::ResizeNwSe,
            Self::TopRight | Self::BottomLeft => egui::CursorIcon::ResizeNeSw,
        }
    }
    fn left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }
    fn right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }
    fn top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }
    fn bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

/// Drag a crop rectangle inside the normalized source bounds. The pointer moves
/// the whole rectangle from its inside, or resizes it from an edge or corner;
/// both stay inside the frame and above the minimum size. A `ratio` lock is the
/// crop's width/height in source pixels (so corners keep their shape anchored at
/// the opposite corner and edges grow the other axis around its centre); `None`
/// resizes freely. Caller retains the gesture-entry rectangle and commits a
/// single undo step on drag stop. Keyboard arrows move by 1% (Shift: 10%);
/// `texture` paints the source frame under the box when the caller has one.
/// Render consumes the returned rectangle.
pub fn control(
    ui: &mut egui::Ui,
    id: &str,
    crop: &mut project::Crop,
    aspect: f32,
    ratio: Option<f64>,
    texture: Option<&egui::TextureHandle>,
    label: &str,
) -> Result<egui::Response> {
    if !crop.valid() {
        return Err(Error::Invalid("normalized rectangle"));
    }
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(Error::Invalid("crop source aspect"));
    }
    let lock = ratio.map(|ratio| ratio / f64::from(aspect));
    if lock.is_some_and(|lock| !lock.is_finite() || lock <= 0.0) {
        return Err(Error::Invalid("crop ratio lock"));
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
    let grab_id = response.id.with("grab");
    if response.drag_started() {
        // The press origin is where the user aimed; the drag threshold has
        // already eaten a few pixels before the frame runs.
        let origin = ui
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if let Some(origin) = origin {
            let grab = grab_at(origin, crop_rect(bounds, *crop));
            ui.ctx().data_mut(|data| data.insert_temp(grab_id, grab));
        }
    }
    let grab = ui
        .ctx()
        .data(|data| data.get_temp::<Grab>(grab_id))
        .unwrap_or(Grab::Move);
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
    if delta != egui::Vec2::ZERO {
        let dx = f64::from(delta.x / bounds.width());
        let dy = f64::from(delta.y / bounds.height());
        resize(crop, grab, dx, dy, lock);
    }
    if response.drag_stopped() {
        ui.ctx().data_mut(|data| data.remove::<Grab>(grab_id));
    }
    if !response.dragged()
        && let Some(pos) = response.hover_pos()
    {
        ui.ctx()
            .set_cursor_icon(grab_at(pos, crop_rect(bounds, *crop)).cursor());
    }
    let selected = crop_rect(bounds, *crop);
    let painter = ui.painter();
    painter.rect_filled(bounds, 4.0, ui.visuals().extreme_bg_color);
    if let Some(texture) = texture {
        let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        painter.image(texture.id(), bounds, uv, egui::Color32::WHITE);
    }
    // Keep the source frame readable when no decoded frame is available yet.
    painter.rect_stroke(
        bounds,
        4.0,
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
        egui::StrokeKind::Inside,
    );
    // Dim what the crop leaves out, so the kept region reads as the frame.
    let scrim = egui::Color32::from_black_alpha(150);
    for region in [
        egui::Rect::from_min_max(bounds.min, egui::pos2(bounds.right(), selected.top())),
        egui::Rect::from_min_max(egui::pos2(bounds.left(), selected.bottom()), bounds.max),
        egui::Rect::from_min_max(
            egui::pos2(bounds.left(), selected.top()),
            egui::pos2(selected.left(), selected.bottom()),
        ),
        egui::Rect::from_min_max(
            egui::pos2(selected.right(), selected.top()),
            egui::pos2(bounds.right(), selected.bottom()),
        ),
    ] {
        if region.width() > 0.0 && region.height() > 0.0 {
            painter.rect_filled(region, 0.0, scrim);
        }
    }
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
    painter.rect_stroke(
        selected,
        0.0,
        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
        egui::StrokeKind::Inside,
    );
    for centre in [
        selected.left_top(),
        selected.center_top(),
        selected.right_top(),
        selected.right_center(),
        selected.right_bottom(),
        selected.center_bottom(),
        selected.left_bottom(),
        selected.left_center(),
    ] {
        let handle = egui::Rect::from_center_size(centre, egui::Vec2::splat(HANDLE_SIZE));
        painter.rect_filled(handle, 1.0, egui::Color32::WHITE);
        painter.rect_stroke(
            handle,
            1.0,
            egui::Stroke::new(1.0, ui.visuals().extreme_bg_color),
            egui::StrokeKind::Outside,
        );
    }
    Ok(response.on_hover_text(label))
}

/// The rectangle `crop` occupies inside the widget bounds.
fn crop_rect(bounds: egui::Rect, crop: project::Crop) -> egui::Rect {
    egui::Rect::from_min_size(
        bounds.min
            + egui::vec2(
                bounds.width() * crop.x as f32,
                bounds.height() * crop.y as f32,
            ),
        egui::vec2(
            bounds.width() * crop.width as f32,
            bounds.height() * crop.height as f32,
        ),
    )
}

/// Which edge or corner the pointer is on; anywhere else moves the rectangle.
fn grab_at(pos: egui::Pos2, rect: egui::Rect) -> Grab {
    if !rect.expand(HANDLE_DISTANCE).contains(pos) {
        return Grab::Move;
    }
    let left = (pos.x - rect.left()).abs() <= HANDLE_DISTANCE;
    let right = (pos.x - rect.right()).abs() <= HANDLE_DISTANCE;
    let top = (pos.y - rect.top()).abs() <= HANDLE_DISTANCE;
    let bottom = (pos.y - rect.bottom()).abs() <= HANDLE_DISTANCE;
    match (left, right, top, bottom) {
        (true, _, true, _) => Grab::TopLeft,
        (_, true, true, _) => Grab::TopRight,
        (true, _, _, true) => Grab::BottomLeft,
        (_, true, _, true) => Grab::BottomRight,
        (true, _, _, _) => Grab::Left,
        (_, true, _, _) => Grab::Right,
        (_, _, true, _) => Grab::Top,
        (_, _, _, true) => Grab::Bottom,
        _ => Grab::Move,
    }
}

/// Apply one gesture delta, keeping the rectangle inside the frame and above
/// the minimum size on every axis. `lock` fixes the width/height shape in
/// normalized source units.
fn resize(crop: &mut project::Crop, grab: Grab, dx: f64, dy: f64, lock: Option<f64>) {
    if grab == Grab::Move {
        crop.x = (crop.x + dx).clamp(0.0, 1.0 - crop.width);
        crop.y = (crop.y + dy).clamp(0.0, 1.0 - crop.height);
        return;
    }
    match lock {
        Some(lock) => resize_locked(crop, grab, dx, dy, lock),
        None => resize_free(crop, grab, dx, dy),
    }
}

/// Free resize: the dragged edge or corner moves on its own axis.
fn resize_free(crop: &mut project::Crop, grab: Grab, dx: f64, dy: f64) {
    if grab.left() {
        let right = crop.x + crop.width;
        crop.x = (crop.x + dx).clamp(0.0, right - MIN_SIZE);
        crop.width = right - crop.x;
    } else if grab.right() {
        crop.width = (crop.width + dx).clamp(MIN_SIZE, 1.0 - crop.x);
    }
    if grab.top() {
        let bottom = crop.y + crop.height;
        crop.y = (crop.y + dy).clamp(0.0, bottom - MIN_SIZE);
        crop.height = bottom - crop.y;
    } else if grab.bottom() {
        crop.height = (crop.height + dy).clamp(MIN_SIZE, 1.0 - crop.y);
    }
}

/// Ratio-locked resize. Corners keep the opposite corner anchored; edges move
/// the dragged edge and grow or shrink the other axis around its centre. The
/// frame wins over both the pointer and the minimum size.
fn resize_locked(crop: &mut project::Crop, grab: Grab, dx: f64, dy: f64, lock: f64) {
    let right = crop.x + crop.width;
    let bottom = crop.y + crop.height;
    let middle = (crop.x + crop.width / 2.0, crop.y + crop.height / 2.0);
    let horizontal = if grab.left() {
        crop.width - dx
    } else {
        crop.width + dx
    };
    let vertical = if grab.top() {
        (crop.height - dy) * lock
    } else {
        (crop.height + dy) * lock
    };
    let wanted = if (grab.left() || grab.right()) && (grab.top() || grab.bottom()) {
        // A corner follows whichever axis the pointer moved more.
        if dx.abs() >= dy.abs() {
            horizontal
        } else {
            vertical
        }
    } else if grab.left() || grab.right() {
        horizontal
    } else {
        vertical
    };
    let min = MIN_SIZE.max(MIN_SIZE * lock);
    let max = if grab.left() && grab.top() {
        right.min(bottom * lock)
    } else if grab.right() && grab.top() {
        (1.0 - crop.x).min(bottom * lock)
    } else if grab.left() && grab.bottom() {
        right.min((1.0 - crop.y) * lock)
    } else if grab.right() && grab.bottom() {
        (1.0 - crop.x).min((1.0 - crop.y) * lock)
    } else if grab.left() {
        right.min(lock)
    } else if grab.right() {
        (1.0 - crop.x).min(lock)
    } else if grab.top() {
        (bottom * lock).min(1.0)
    } else {
        ((1.0 - crop.y) * lock).min(1.0)
    };
    // An extreme lock can leave less room than the minimum; the frame wins.
    let width = if max < min {
        max.max(0.0)
    } else {
        wanted.clamp(min, max)
    };
    let height = width / lock;
    let centred = |at: f64, extent: f64| (at - extent / 2.0).clamp(0.0, 1.0 - extent);
    crop.x = match grab {
        Grab::TopLeft | Grab::BottomLeft | Grab::Left => right - width,
        Grab::Top | Grab::Bottom => centred(middle.0, width),
        _ => crop.x,
    };
    crop.y = match grab {
        Grab::TopLeft | Grab::TopRight | Grab::Top => bottom - height,
        Grab::Left | Grab::Right => centred(middle.1, height),
        _ => crop.y,
    };
    crop.width = width;
    crop.height = height;
}
