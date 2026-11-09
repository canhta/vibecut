//! Direct transform gestures on the preview canvas.
use super::{Action, Context};
use crate::{
    egui::{self, CursorIcon, Pos2, Rect, Sense, Vec2},
    i18n::Text,
    kit,
};
use project::{Item, ItemId, ItemParams, TextAlign, TextParams, TransformField};
use render::viewport::{Handle, ItemGeometry, Viewport};
use time::Tick;

/// The four corners and two side midpoints the reference capture shows.
const SCALE_HANDLES: [Handle; 6] = [
    Handle::TopLeft,
    Handle::TopRight,
    Handle::Right,
    Handle::BottomRight,
    Handle::BottomLeft,
    Handle::Left,
];

#[derive(Default)]
pub(in crate::editor) struct State {
    drag: Option<Drag>,
    text: Option<TextEdit>,
}

impl State {
    /// Abandon an in-progress gesture; the caller discards its staged candidate.
    pub(in crate::editor) fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// Close an inline text edit; the caller commits when it returns `Some(true)`
    /// and discards when the content returned to its opening value.
    pub(in crate::editor) fn cancel_text(&mut self) -> Option<bool> {
        self.text.take().map(|edit| edit.changed())
    }
}

/// One inline text session: the opening content, the live buffer and the box it
/// was opened over. The box follows the staged text while it grows; an empty
/// buffer keeps the last box so the caret stays anchored on a blank item.
struct TextEdit {
    item: ItemId,
    start: String,
    content: String,
    rect: Rect,
    font: f32,
    align: egui::Align,
    focus: bool,
    focused_once: bool,
}

impl TextEdit {
    fn begin(item: ItemId, params: &TextParams, rect: Rect, font: f32) -> Self {
        Self {
            item,
            start: params.content.as_str().to_owned(),
            content: params.content.as_str().to_owned(),
            rect,
            font,
            align: match params.align {
                TextAlign::Left => egui::Align::Min,
                TextAlign::Center => egui::Align::Center,
                TextAlign::Right => egui::Align::Max,
            },
            focus: true,
            focused_once: false,
        }
    }

    fn changed(&self) -> bool {
        self.content != self.start
    }

    /// Track the staged text box; an empty item has no geometry, so the last
    /// non-empty box stays as the caret anchor.
    fn follow(&mut self, geometry: &ItemGeometry, viewport: &Viewport) {
        if let Some(rect) = bounds(geometry, viewport) {
            self.rect = rect;
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Move,
    Scale(Handle),
    Rotate,
}

struct Drag {
    item: ItemId,
    kind: Kind,
    geometry: ItemGeometry,
    start: Values,
    /// Gesture origin in project canvas pixels.
    canvas: [f64; 2],
    /// Rotation grip angle at the gesture origin, radians.
    angle: f64,
    last: Option<Vec<(TransformField, f64)>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Values {
    x: f64,
    y: f64,
    sx: f64,
    sy: f64,
    rotate: f64,
}

impl Values {
    fn at(item: &Item, local: Tick) -> Option<Self> {
        Some(Self {
            x: TransformField::PositionX
                .value_at(&item.animations, local)
                .ok()?,
            y: TransformField::PositionY
                .value_at(&item.animations, local)
                .ok()?,
            sx: TransformField::ScaleX
                .value_at(&item.animations, local)
                .ok()?,
            sy: TransformField::ScaleY
                .value_at(&item.animations, local)
                .ok()?,
            rotate: TransformField::Rotate
                .value_at(&item.animations, local)
                .ok()?,
        })
    }

    fn get(self, field: TransformField) -> f64 {
        match field {
            TransformField::PositionX => self.x,
            TransformField::PositionY => self.y,
            TransformField::ScaleX => self.sx,
            TransformField::ScaleY => self.sy,
            TransformField::Rotate => self.rotate,
        }
    }
}

impl Drag {
    fn begin(
        project: &project::Project,
        viewport: &Viewport,
        item: ItemId,
        kind: Kind,
        geometry: ItemGeometry,
        pointer: [f64; 2],
        at: Tick,
    ) -> Option<Self> {
        let data = find(project, item)?;
        let local = at.checked_sub(data.start)?;
        let start = Values::at(data, local)?;
        let (screen, angle) = match kind {
            Kind::Move => (pointer, 0.0),
            Kind::Scale(handle) => (geometry.handle_position(viewport, handle).ok()?, 0.0),
            Kind::Rotate => {
                let grip = geometry
                    .rotation_handle_position(viewport, f64::from(kit::size::TRANSFORM_GAP))
                    .ok()?;
                let centre = centre(&geometry, viewport)?;
                let angle = (grip[1] - centre[1]).atan2(grip[0] - centre[0]);
                (grip, angle)
            }
        };
        let canvas = viewport.screen_to_canvas(screen).ok()?;
        Some(Self {
            item,
            kind,
            geometry,
            start,
            canvas,
            angle,
            last: None,
        })
    }

    /// Absolute values from the gesture origin, ready for the preview command.
    fn values(&self, viewport: &Viewport, pointer: Pos2) -> Option<Vec<(TransformField, f64)>> {
        let point = [f64::from(pointer.x), f64::from(pointer.y)];
        let canvas = viewport.screen_to_canvas(point).ok()?;
        Some(match self.kind {
            Kind::Move => {
                let x = TransformField::PositionX
                    .clamp(self.start.x + canvas[0] - self.canvas[0])
                    .ok()?;
                let y = TransformField::PositionY
                    .clamp(self.start.y + canvas[1] - self.canvas[1])
                    .ok()?;
                vec![
                    (TransformField::PositionX, x),
                    (TransformField::PositionY, y),
                ]
            }
            Kind::Scale(handle) => {
                let [sx, sy] = self.geometry.scale_at(handle, canvas).ok()?;
                let (across, down) = axes(handle);
                let mut values = Vec::with_capacity(2);
                if across {
                    values.push((TransformField::ScaleX, sx));
                }
                if down {
                    values.push((TransformField::ScaleY, sy));
                }
                values
            }
            Kind::Rotate => {
                let centre = centre(&self.geometry, viewport)?;
                let angle = (point[1] - centre[1]).atan2(point[0] - centre[0]);
                let mut delta = (angle - self.angle).to_degrees();
                delta = (delta + 180.0).rem_euclid(360.0) - 180.0;
                let rotate = TransformField::Rotate
                    .clamp(self.start.rotate + delta)
                    .ok()?;
                vec![(TransformField::Rotate, rotate)]
            }
        })
    }

    /// A gesture that returned every submitted value to its origin publishes nothing.
    fn commit(&self) -> bool {
        self.last.as_ref().is_some_and(|values| {
            values
                .iter()
                .any(|(field, value)| (value - self.start.get(*field)).abs() > 1e-9)
        })
    }
}

/// Paints the selected item's box, scale handles and rotation grip.
struct Visual {
    corners: [Pos2; 4],
    handles: [(Handle, Pos2); 6],
    rotation: Pos2,
    bottom: Pos2,
}

impl Visual {
    fn new(viewport: &Viewport, geometry: &ItemGeometry) -> Option<Self> {
        let position = |handle| handle_position(viewport, geometry, handle);
        Some(Self {
            corners: [
                position(Handle::TopLeft)?,
                position(Handle::TopRight)?,
                position(Handle::BottomRight)?,
                position(Handle::BottomLeft)?,
            ],
            handles: [
                (Handle::TopLeft, position(Handle::TopLeft)?),
                (Handle::TopRight, position(Handle::TopRight)?),
                (Handle::Right, position(Handle::Right)?),
                (Handle::BottomRight, position(Handle::BottomRight)?),
                (Handle::BottomLeft, position(Handle::BottomLeft)?),
                (Handle::Left, position(Handle::Left)?),
            ],
            rotation: {
                let [x, y] = geometry
                    .rotation_handle_position(viewport, f64::from(kit::size::TRANSFORM_GAP))
                    .ok()?;
                egui::pos2(x as f32, y as f32)
            },
            bottom: position(Handle::Bottom)?,
        })
    }

    fn centre(&self) -> Pos2 {
        let [top_left, _, bottom_right, _] = self.corners;
        egui::pos2(
            (top_left.x + bottom_right.x) / 2.0,
            (top_left.y + bottom_right.y) / 2.0,
        )
    }

    fn hit(&self, point: Pos2) -> Option<Kind> {
        if point.distance(self.rotation) <= kit::size::TRANSFORM_HIT {
            return Some(Kind::Rotate);
        }
        let mut nearest: Option<(f32, Handle)> = None;
        for (handle, position) in self.handles {
            let distance = point.distance(position);
            if distance <= kit::size::TRANSFORM_HIT
                && nearest.is_none_or(|(nearest, _)| distance < nearest)
            {
                nearest = Some((distance, handle));
            }
        }
        nearest.map(|(_, handle)| Kind::Scale(handle))
    }

    fn paint(&self, ui: &egui::Ui, dark: bool) {
        let painter = ui.painter();
        let stroke = egui::Stroke::new(kit::size::TRANSFORM_STROKE, kit::accent(dark));
        let [top_left, top_right, bottom_right, bottom_left] = self.corners;
        for (from, to) in [
            (top_left, top_right),
            (top_right, bottom_right),
            (bottom_right, bottom_left),
            (bottom_left, top_left),
        ] {
            painter.line_segment([from, to], stroke);
        }
        painter.line_segment([self.bottom, self.rotation], stroke);
        for (_, position) in self.handles {
            paint_handle(ui, position, kit::size::TRANSFORM_HANDLE / 2.0, dark);
        }
        paint_handle(ui, self.rotation, kit::size::TRANSFORM_GRIP / 2.0, dark);
        kit::tinted(kit::Icon::ArrowClockwise, kit::icon::PLAY, kit::text(dark)).paint_at(
            ui,
            Rect::from_center_size(self.rotation, Vec2::splat(kit::icon::PLAY)),
        );
    }
}

pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    frame: Rect,
    canvas: &egui::Response,
    at: Tick,
    actions: &mut Vec<Action>,
) {
    let project = cx.staged_project();
    let dimensions = &project.data().canvas;
    let Some(viewport) = viewport(frame, [dimensions.width, dimensions.height]) else {
        state.drag = None;
        return;
    };
    let settings = match cx
        .playback
        .render_state()
        .and_then(|state| state.settings(project))
    {
        Ok(settings) => settings,
        Err(_) => return,
    };
    let Ok(renderer) = render::Renderer::new(project, settings) else {
        return;
    };
    let selected = if cx.selection.len() == 1 {
        cx.selection.iter().next().copied()
    } else {
        None
    };
    let geometry = selected.and_then(|item| {
        renderer
            .item_geometry(item, at)
            .ok()
            .flatten()
            .map(|geometry| (item, geometry))
    });
    let visual = geometry.and_then(|(item, geometry)| {
        Visual::new(&viewport, &geometry).map(|visual| (item, geometry, visual))
    });

    let pointer = ui.input(|i| i.pointer.interact_pos());
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    let down = ui.input(|i| i.pointer.primary_down());
    let double = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });

    // An open inline edit owns the pointer and keys until it commits.
    if let Some(mut edit) = state.text.take() {
        if let Some(geometry) = renderer.item_geometry(edit.item, at).ok().flatten() {
            edit.follow(&geometry, &viewport);
        }
        if text_overlay(ui, &mut edit, cx.dark, actions) {
            state.text = Some(edit);
        }
        if let Some((_, _, visual)) = &visual {
            visual.paint(ui, cx.dark);
        }
        return;
    }

    // Register the handle targets before reading this frame's presses.
    let mut responses = Vec::with_capacity(SCALE_HANDLES.len() + 1);
    if let Some((_, _, visual)) = &visual {
        for (handle, position) in visual.handles {
            responses.push((
                Kind::Scale(handle),
                scale_id(handle),
                handle_target(ui, position, scale_id(handle), cx.tr(Text::Scale)),
            ));
        }
        responses.push((
            Kind::Rotate,
            "preview.transform.rotate",
            handle_target(
                ui,
                visual.rotation,
                "preview.transform.rotate",
                cx.tr(Text::Rotate),
            ),
        ));
    }

    let opened = double
        && if let Some(pointer) = pointer {
            open_text(project, &renderer, &viewport, pointer, at, state, actions)
        } else {
            false
        };

    if !opened && let Some(mut drag) = state.drag.take() {
        if !down {
            let commit = drag.commit();
            if drag.last.is_some() {
                actions.push(Action::TransformEnd { commit });
            }
        } else if let Some(pointer) = pointer {
            if let Some(values) = drag.values(&viewport, pointer)
                && drag.last.as_ref() != Some(&values)
            {
                drag.last = Some(values.clone());
                actions.push(Action::TransformPreview {
                    item: drag.item,
                    values,
                });
            }
            state.drag = Some(drag);
        } else {
            state.drag = Some(drag);
        }
    } else if !opened
        && pressed
        && let Some(pointer) = pointer
    {
        let handle = responses
            .iter()
            .find(|(_, _, response)| response.is_pointer_button_down_on())
            .map(|(kind, _, _)| *kind);
        if let Some((item, geometry, _)) = &visual
            && let Some(kind) = handle
        {
            state.drag = Drag::begin(
                project,
                &viewport,
                *item,
                kind,
                *geometry,
                [f64::from(pointer.x), f64::from(pointer.y)],
                at,
            );
        } else if canvas.is_pointer_button_down_on() {
            press(&renderer, &viewport, pointer, at, project, actions, state);
        }
    }

    if let Some((_, _, visual)) = &visual {
        visual.paint(ui, cx.dark);
    }

    // Cursors: a grabbed handle while dragging, otherwise the hovered target.
    if state.drag.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if let (Some(pointer), Some((_, _, visual))) = (pointer, &visual) {
        if let Some(kind) = visual.hit(pointer) {
            ui.ctx().set_cursor_icon(match kind {
                Kind::Scale(handle) => resize_cursor(handle, visual),
                Kind::Rotate => CursorIcon::Grab,
                Kind::Move => CursorIcon::Move,
            });
        } else if frame.contains(pointer)
            && renderer
                .hit_test(&viewport, [f64::from(pointer.x), f64::from(pointer.y)], at)
                .ok()
                .flatten()
                .is_some()
        {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        }
    }

    for (_, id, response) in responses {
        actions.push(Action::Remember {
            id: id.into(),
            icon: None,
            tooltip: None,
            response,
        });
    }
}

/// A double click on a visible text item opens one inline edit session.
fn open_text(
    project: &project::Project,
    renderer: &render::Renderer<'_>,
    viewport: &Viewport,
    pointer: Pos2,
    at: Tick,
    state: &mut State,
    actions: &mut Vec<Action>,
) -> bool {
    let point = [f64::from(pointer.x), f64::from(pointer.y)];
    let Some(item) = renderer.hit_test(viewport, point, at).ok().flatten() else {
        return false;
    };
    let Some(params) = find(project, item).and_then(text_params) else {
        return false;
    };
    let Some(geometry) = renderer.item_geometry(item, at).ok().flatten() else {
        return false;
    };
    let Some(rect) = bounds(&geometry, viewport) else {
        return false;
    };
    let Some(font) = font_pixels(project, viewport, item, at, params) else {
        return false;
    };
    state.drag = None;
    state.text = Some(TextEdit::begin(item, params, rect, font));
    actions.push(Action::TextBegin { item });
    true
}

/// The live inline editor: transparent glyphs over the rendered text, an accent
/// caret and a translucent accent selection. Returns whether it stays open.
fn text_overlay(
    ui: &mut egui::Ui,
    edit: &mut TextEdit,
    dark: bool,
    actions: &mut Vec<Action>,
) -> bool {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(edit.rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(edit.rect));
    let accent = kit::accent(dark);
    let mut style = (**child.style()).clone();
    style.visuals.selection.bg_fill = accent.gamma_multiply(kit::size::TEXT_EDIT_SELECTION);
    style.visuals.selection.stroke = egui::Stroke::NONE;
    style.visuals.text_cursor.stroke = egui::Stroke::new(kit::size::TEXT_EDIT_CARET, accent);
    child.set_style(style);
    let previous = edit.content.clone();
    let output = egui::TextEdit::multiline(&mut edit.content)
        .id_salt("preview.text.edit")
        .font(egui::FontId::proportional(edit.font))
        .text_color(egui::Color32::TRANSPARENT)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .align(egui::Align2([edit.align, egui::Align::Min]))
        .desired_width(edit.rect.width())
        .show(&mut child);
    let response = output.response.response;
    if edit.focus {
        response.request_focus();
        edit.focus = false;
    }
    if response.has_focus() {
        edit.focused_once = true;
    }
    if edit.content != previous {
        actions.push(Action::TextPreview {
            item: edit.item,
            content: edit.content.clone(),
        });
    }
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let outside = ui.input(|i| {
        i.pointer.primary_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|pointer| !edit.rect.contains(pointer))
    });
    let blur = (edit.focused_once && response.lost_focus()) || outside;
    actions.push(Action::Remember {
        id: "preview.text.edit".into(),
        icon: None,
        tooltip: None,
        response,
    });
    if escape || blur {
        actions.push(Action::TextEnd {
            commit: edit.changed(),
        });
        return false;
    }
    true
}

fn text_params(item: &Item) -> Option<&TextParams> {
    match &item.content {
        ItemParams::Text(params) => Some(params),
        _ => None,
    }
}

/// Axis-aligned screen box around the rotated text box.
fn bounds(geometry: &ItemGeometry, viewport: &Viewport) -> Option<Rect> {
    let mut min = egui::pos2(f32::INFINITY, f32::INFINITY);
    let mut max = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for handle in [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
    ] {
        let [x, y] = geometry.handle_position(viewport, handle).ok()?;
        let point = egui::pos2(x as f32, y as f32);
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }
    Some(Rect::from_min_max(min, max))
}

/// Font size in screen pixels: the item's canvas size scaled by its transform
/// and the letterboxed viewport.
fn font_pixels(
    project: &project::Project,
    viewport: &Viewport,
    item: ItemId,
    at: Tick,
    params: &TextParams,
) -> Option<f32> {
    let data = find(project, item)?;
    let local = at.checked_sub(data.start)?;
    let scale_x = TransformField::ScaleX
        .value_at(&data.animations, local)
        .ok()?;
    let origin = viewport.canvas_to_screen([0.0, 0.0]).ok()?;
    let unit = viewport.canvas_to_screen([1.0, 0.0]).ok()?;
    let scale = (unit[0] - origin[0]).hypot(unit[1] - origin[1]);
    Some(((f64::from(params.font_size) * scale_x * scale) as f32).max(1.0))
}

fn press(
    renderer: &render::Renderer<'_>,
    viewport: &Viewport,
    pointer: Pos2,
    at: Tick,
    project: &project::Project,
    actions: &mut Vec<Action>,
    state: &mut State,
) {
    let point = [f64::from(pointer.x), f64::from(pointer.y)];
    if let Some(item) = renderer.hit_test(viewport, point, at).ok().flatten() {
        actions.push(Action::Select(item));
        if let Some(geometry) = renderer.item_geometry(item, at).ok().flatten() {
            state.drag = Drag::begin(project, viewport, item, Kind::Move, geometry, point, at);
        }
    } else {
        actions.push(Action::ClearSelection);
    }
}

fn handle_target(ui: &mut egui::Ui, position: Pos2, id: &str, label: &str) -> egui::Response {
    let rect = Rect::from_center_size(position, Vec2::splat(kit::size::TRANSFORM_HIT * 2.0));
    let response = ui.interact(rect, ui.make_persistent_id(id), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

fn paint_handle(ui: &egui::Ui, position: Pos2, radius: f32, dark: bool) {
    ui.painter()
        .circle_filled(position, radius, kit::panel(dark));
    ui.painter().circle_stroke(
        position,
        radius,
        egui::Stroke::new(kit::size::TRANSFORM_STROKE, kit::accent(dark)),
    );
}

fn viewport(frame: Rect, canvas: [u32; 2]) -> Option<Viewport> {
    Viewport::new(
        render::Rect {
            x: f64::from(frame.min.x),
            y: f64::from(frame.min.y),
            width: f64::from(frame.width()),
            height: f64::from(frame.height()),
        },
        canvas,
        1.0,
        [0.0, 0.0],
    )
    .ok()
}

fn handle_position(viewport: &Viewport, geometry: &ItemGeometry, handle: Handle) -> Option<Pos2> {
    let [x, y] = geometry.handle_position(viewport, handle).ok()?;
    Some(egui::pos2(x as f32, y as f32))
}

fn centre(geometry: &ItemGeometry, viewport: &Viewport) -> Option<[f64; 2]> {
    let top_left = geometry.handle_position(viewport, Handle::TopLeft).ok()?;
    let bottom_right = geometry
        .handle_position(viewport, Handle::BottomRight)
        .ok()?;
    Some([
        (top_left[0] + bottom_right[0]) / 2.0,
        (top_left[1] + bottom_right[1]) / 2.0,
    ])
}

fn find(project: &project::Project, item: ItemId) -> Option<&Item> {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
}

fn axes(handle: Handle) -> (bool, bool) {
    match handle {
        Handle::TopLeft | Handle::TopRight | Handle::BottomRight | Handle::BottomLeft => {
            (true, true)
        }
        Handle::Left | Handle::Right => (true, false),
        Handle::Top | Handle::Bottom => (false, true),
    }
}

fn scale_id(handle: Handle) -> &'static str {
    match handle {
        Handle::TopLeft => "preview.transform.top-left",
        Handle::TopRight => "preview.transform.top-right",
        Handle::Right => "preview.transform.right",
        Handle::BottomRight => "preview.transform.bottom-right",
        Handle::BottomLeft => "preview.transform.bottom-left",
        Handle::Left => "preview.transform.left",
        Handle::Top => "preview.transform.top",
        Handle::Bottom => "preview.transform.bottom",
    }
}

fn resize_cursor(handle: Handle, visual: &Visual) -> CursorIcon {
    let centre = visual.centre();
    let position = visual
        .handles
        .iter()
        .find(|(candidate, _)| *candidate == handle)
        .map(|(_, position)| *position)
        .unwrap_or(visual.rotation);
    let dx = position.x - centre.x;
    let dy = position.y - centre.y;
    let across = dx.abs();
    let down = dy.abs();
    if across > down * 2.0 {
        CursorIcon::ResizeHorizontal
    } else if down > across * 2.0 {
        CursorIcon::ResizeVertical
    } else if dx * dy >= 0.0 {
        CursorIcon::ResizeNwSe
    } else {
        CursorIcon::ResizeNeSw
    }
}
