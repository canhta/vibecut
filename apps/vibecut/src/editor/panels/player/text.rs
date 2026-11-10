//! Positions and paints the inline text session from the text crate's layout.
use super::Action;
use crate::{
    editor::panels::player::state::TextCaret,
    egui::{
        self, Rect,
        text::{CCursor, CCursorRange},
    },
    kit,
};
use project::{ItemId, TextAlign, TextBackground, TextParams};
use render::viewport::{ItemGeometry, Viewport};
use std::ops::Range;
use text::{Align as LayoutAlign, LayoutOptions};

/// One inline text session: the opening content, the live buffer and the box it
/// was opened over. The box follows the staged text while it grows; an empty
/// buffer keeps the last box so the caret stays anchored on a blank item.
pub(super) struct Edit {
    pub(super) item: ItemId,
    start: String,
    content: String,
    rect: Rect,
    geometry: Option<ItemGeometry>,
    face: Face,
    /// Text background and outline margin in canvas pixels.
    pad: f32,
    focus: bool,
    focused_once: bool,
    /// Character index where a pointer drag started.
    drag: Option<usize>,
    caret: Option<TextCaret>,
}

struct Face {
    family: String,
    style: String,
    size: f32,
    letter_spacing: f32,
    line_height: f32,
    align: LayoutAlign,
}

impl Face {
    fn new(params: &TextParams) -> Self {
        Self {
            family: params.font.as_str().to_owned(),
            style: text::style_name(params.weight, params.italic),
            size: params.font_size,
            letter_spacing: params.letter_spacing,
            line_height: params.line_height,
            align: match params.align {
                TextAlign::Left => LayoutAlign::Left,
                TextAlign::Center => LayoutAlign::Center,
                TextAlign::Right => LayoutAlign::Right,
            },
        }
    }
}

impl Edit {
    pub(super) fn begin(
        item: ItemId,
        params: &TextParams,
        rect: Rect,
        geometry: ItemGeometry,
    ) -> Self {
        Self {
            item,
            start: params.content.as_str().to_owned(),
            content: params.content.as_str().to_owned(),
            rect,
            geometry: Some(geometry),
            face: Face::new(params),
            pad: padding(params),
            focus: true,
            focused_once: false,
            drag: None,
            caret: None,
        }
    }

    pub(super) fn changed(&self) -> bool {
        self.content != self.start
    }

    /// Track the staged text box; an empty item has no geometry, so the last
    /// non-empty box stays as the caret anchor.
    pub(super) fn follow(&mut self, geometry: &ItemGeometry, viewport: &Viewport) {
        if let Some(rect) = bounds(geometry, viewport) {
            self.rect = rect;
            self.geometry = Some(*geometry);
        }
    }

    pub(super) fn caret(&self) -> Option<TextCaret> {
        self.caret.clone()
    }

    fn options(&self) -> Option<LayoutOptions<'_>> {
        let width = self.geometry.as_ref()?.bounds().width as f32 - 2.0 * self.pad;
        if !(width.is_finite() && width > 0.0) {
            return None;
        }
        Some(LayoutOptions {
            family: &self.face.family,
            style: &self.face.style,
            size: self.face.size,
            max_width: width,
            align: self.face.align,
            line_height: self.face.line_height,
            letter_spacing: self.face.letter_spacing,
            ..Default::default()
        })
    }

    /// Source character index at a screen point, through the item transform and the
    /// text crate's inverse layout query.
    fn pointer_index(&self, viewport: &Viewport, point: egui::Pos2) -> Option<usize> {
        let geometry = self.geometry.as_ref()?;
        let canvas = geometry
            .inverse(viewport, [f64::from(point.x), f64::from(point.y)])
            .ok()?;
        let bounds = geometry.bounds();
        let local = [
            canvas[0] as f32 - bounds.x as f32 - self.pad,
            canvas[1] as f32 - bounds.y as f32 - self.pad,
        ];
        let layout = text::layout(&self.content, &self.options()?).ok()?;
        Some(layout.index(&self.content, local))
    }

    /// Primary index and selected range for this frame's pointer input.
    fn pointer(
        &mut self,
        viewport: &Viewport,
        point: egui::Pos2,
        double: bool,
        pressed: bool,
        down: bool,
        released: bool,
    ) -> Option<(usize, Range<usize>)> {
        if double {
            let index = self.pointer_index(viewport, point)?;
            let word = text::word_range(&self.content, index);
            self.drag = None;
            return Some((word.end, word));
        }
        if pressed {
            let index = self.pointer_index(viewport, point)?;
            self.drag = down.then_some(index);
            return Some((index, index..index));
        }
        let anchor = self.drag?;
        if !down && !released {
            self.drag = None;
            return None;
        }
        let index = self.pointer_index(viewport, point)?;
        if released || !down {
            self.drag = None;
        }
        Some((index, anchor.min(index)..anchor.max(index)))
    }

    /// Caret and selection geometry for a character range in the live buffer.
    fn sample(
        &self,
        primary: usize,
        cursor: Range<usize>,
        viewport: &Viewport,
    ) -> Option<TextCaret> {
        let geometry = self.geometry.as_ref()?;
        let layout = text::layout(&self.content, &self.options()?).ok()?;
        let caret = layout.caret(&self.content, primary);
        let start = layout.caret(&self.content, cursor.start);
        let end = layout.caret(&self.content, cursor.end);
        let bounds = geometry.bounds();
        let origin = [bounds.x as f32 + self.pad, bounds.y as f32 + self.pad];
        let point = |x: f32, y: f32| -> Option<[f32; 2]> {
            let [x, y] = geometry
                .point(
                    viewport,
                    [f64::from(origin[0] + x), f64::from(origin[1] + y)],
                )
                .ok()?;
            Some([x as f32, y as f32])
        };
        let mut selection = Vec::new();
        if cursor.start != cursor.end {
            for index in start.line..=end.line {
                let Some(line) = layout.lines.get(index) else {
                    break;
                };
                let left = if index == start.line { start.x } else { line.x };
                let right = if index == end.line {
                    end.x
                } else {
                    line.x + line.width
                };
                if right <= left {
                    continue;
                }
                let top = line.baseline - line.ascent;
                let bottom = line.baseline + line.descent;
                selection.push([
                    point(left, top)?,
                    point(right, top)?,
                    point(right, bottom)?,
                    point(left, bottom)?,
                ]);
            }
        }
        Some(TextCaret {
            index: primary,
            range: cursor,
            layout: [[caret.x, caret.top], [caret.x, caret.bottom]],
            screen: [point(caret.x, caret.top)?, point(caret.x, caret.bottom)?],
            selection,
        })
    }
}

/// The live inline editor: transparent input glyphs under a caret and selection
/// painted at the text crate's layout positions. Returns whether it stays open.
pub(super) fn show(
    ui: &mut egui::Ui,
    edit: &mut Edit,
    viewport: &Viewport,
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
    style.visuals.selection.bg_fill = egui::Color32::TRANSPARENT;
    style.visuals.selection.stroke = egui::Stroke::NONE;
    style.visuals.text_cursor.stroke = egui::Stroke::NONE;
    child.set_style(style);
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    let double = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });
    let down = ui.input(|i| i.pointer.primary_down());
    let released = ui.input(|i| i.pointer.primary_released());
    let inside = pointer.is_some_and(|point| edit.rect.contains(point));
    let dragging = edit.drag.is_some();
    let pointer_cursor = if inside && (pressed || double) {
        pointer.and_then(|point| edit.pointer(viewport, point, double, pressed, down, released))
    } else if dragging {
        pointer.and_then(|point| edit.pointer(viewport, point, false, false, down, released))
    } else {
        None
    };
    let owns_pointer = (inside && (pressed || double)) || dragging;
    if owns_pointer {
        let rect = edit.rect;
        ui.input_mut(|input| {
            input.events.retain(|event| match event {
                egui::Event::PointerMoved(point) => !dragging && !rect.contains(*point),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    ..
                } => !(dragging || rect.contains(*pos)),
                _ => true,
            });
        });
    }
    let previous = edit.content.clone();
    let output = egui::TextEdit::multiline(&mut edit.content)
        .id_salt("preview.text.edit")
        .font(egui::FontId::proportional(
            edit.face.size * screen_scale(viewport),
        ))
        .text_color(egui::Color32::TRANSPARENT)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .align(egui::Align2([
            match edit.face.align {
                LayoutAlign::Left => egui::Align::Min,
                LayoutAlign::Center => egui::Align::Center,
                LayoutAlign::Right => egui::Align::Max,
            },
            egui::Align::Min,
        ]))
        .desired_width(edit.rect.width())
        .show(&mut child);
    let id = output.response.id;
    let response = output.response.response;
    if edit.focus {
        response.request_focus();
        edit.focus = false;
    }
    if inside && (pressed || double) {
        response.request_focus();
    }
    if response.has_focus() {
        edit.focused_once = true;
    }
    let keyboard = output.cursor_range.map(|range| {
        let primary = usize::from(range.primary.index);
        let range = range.as_sorted_char_range();
        (primary, usize::from(range.start)..usize::from(range.end))
    });
    let chosen = pointer_cursor.clone().or(keyboard);
    if let Some((primary, range)) = pointer_cursor {
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(if range.start == range.end {
                CCursorRange::one(CCursor::new(primary))
            } else {
                CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))
            }));
        state.store(ui.ctx(), id);
    }
    edit.caret = response
        .has_focus()
        .then(|| chosen.and_then(|(primary, range)| edit.sample(primary, range, viewport)))
        .flatten();
    if let Some(caret) = &edit.caret {
        let painter = child.painter();
        let fill = accent.gamma_multiply(kit::size::TEXT_EDIT_SELECTION);
        for quad in &caret.selection {
            painter.add(egui::Shape::convex_polygon(
                quad.iter()
                    .map(|point| egui::pos2(point[0], point[1]))
                    .collect(),
                fill,
                egui::Stroke::NONE,
            ));
        }
        painter.line_segment(
            [
                egui::pos2(caret.screen[0][0], caret.screen[0][1]),
                egui::pos2(caret.screen[1][0], caret.screen[1][1]),
            ],
            egui::Stroke::new(kit::size::TEXT_EDIT_CARET, accent),
        );
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

/// Logical screen pixels per project canvas pixel for the current viewport.
fn screen_scale(viewport: &Viewport) -> f32 {
    let origin = viewport.canvas_to_screen([0.0, 0.0]).ok();
    let unit = viewport.canvas_to_screen([1.0, 0.0]).ok();
    origin
        .zip(unit)
        .map_or(1.0, |(origin, unit)| {
            (unit[0] - origin[0]).hypot(unit[1] - origin[1]) as f32
        })
        .max(f32::EPSILON)
}

/// Text background and outline margin in canvas pixels, matching the renderer's
/// raster margin so the layout uses the same wrap width.
fn padding(params: &TextParams) -> f32 {
    params.background.as_ref().map_or(0.0, |background| {
        let extent = |value: f32| value.clamp(0.0, TextBackground::MAX_EXTENT);
        let offset = |value: f32| {
            value
                .clamp(-TextBackground::MAX_OFFSET, TextBackground::MAX_OFFSET)
                .abs()
        };
        (extent(background.padding_x) + offset(background.offset_x))
            .max(extent(background.padding_y) + offset(background.offset_y))
            .ceil()
    })
}

/// Axis-aligned screen box around the rotated text box.
pub(super) fn bounds(geometry: &ItemGeometry, viewport: &Viewport) -> Option<Rect> {
    let mut min = egui::pos2(f32::INFINITY, f32::INFINITY);
    let mut max = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for handle in [
        render::viewport::Handle::TopLeft,
        render::viewport::Handle::TopRight,
        render::viewport::Handle::BottomRight,
        render::viewport::Handle::BottomLeft,
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
