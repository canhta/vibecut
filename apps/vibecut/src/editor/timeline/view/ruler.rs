//! Renders the timeline ruler.

use crate::editor::timeline::{
    Action, Align2, BookmarkView, Context, FontId, FrameRate, Id, MIN_ZOOM_FLOOR,
    RULER_LABEL_SPACING, RULER_TICK_SPACING, Rect, Rounding, State, Tick, TimelineLayout, Vec2,
    display_time, egui, frame_snap, theme, view::Ink,
};

impl State {
    pub(in crate::editor::timeline) fn paint_ruler(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        bookmarks: &[BookmarkView],
    ) {
        let ink = Ink::of(cx);
        let painter = ui.painter().with_clip_rect(layout.ruler);
        let rate = cx.staged_project().data().canvas.fps;
        let config = ruler_config(self.zoom, rate);
        let start = f64::from(self.scroll);
        let end = start + f64::from(layout.ruler.width() / self.zoom);
        let first = (start / config.tick_seconds).floor() as i64;
        let last = (end / config.tick_seconds).ceil() as i64;
        let mut index = first;
        let mut guard = 0;
        while index <= last && guard < 4096 {
            let seconds = index as f64 * config.tick_seconds;
            let labelled = index % config.label_multiple == 0;
            let x = if labelled {
                snapped_seconds(seconds, rate)
            } else {
                seconds
            };
            let x = layout.ruler.left() + ((x - start) * f64::from(self.zoom)) as f32;
            if x >= layout.ruler.left() - 1.0 && x <= layout.ruler.right() + 1.0 {
                // Ticks start 3 px under the ruler's top: 5.5 px minor, 10.5 px labelled.
                let height = if labelled { 10.5 } else { 5.5 };
                let x = x.round() + 0.5;
                painter.line_segment(
                    [
                        egui::pos2(x, layout.ruler.top() + 3.0),
                        egui::pos2(x, layout.ruler.top() + 3.0 + height),
                    ],
                    egui::Stroke::new(1.0, theme::disabled(ink.dark())),
                );
                if labelled {
                    painter.text(
                        egui::pos2(x + 4.0, layout.ruler.top() + 7.5),
                        Align2::LEFT_CENTER,
                        ruler_text(seconds, rate, config.label_seconds),
                        FontId::proportional(theme::type_size::CAPTION),
                        ink.secondary(),
                    );
                }
            }
            index += 1;
            guard += 1;
        }
        let mut bookmark_hits = Vec::new();
        for bookmark in bookmarks {
            let x = self.x_at(bookmark.start, layout.ruler.left());
            if x >= layout.ruler.left() && x <= layout.ruler.right() {
                painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(x - 2.0, layout.ruler.bottom() - 5.0),
                        Vec2::new(4.0, 5.0),
                    ),
                    1.0,
                    ink.accent(),
                );
                bookmark_hits.push((
                    bookmark.id,
                    bookmark.start,
                    bookmark.duration,
                    bookmark.note.clone(),
                    Rect::from_min_max(
                        egui::pos2(x - 5.0, layout.ruler.bottom() - 11.0),
                        egui::pos2(x + 5.0, layout.ruler.bottom()),
                    ),
                ));
            }
        }
        let response = ui.interact(
            layout.ruler,
            Id::unique("timeline.ruler"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.ruler", &response);
        let mut marker_clicked = false;
        for (id, start, duration, note, hit) in bookmark_hits {
            let response = ui.interact(
                hit,
                Id::unique(("timeline.bookmark", id.0)),
                egui::Sense::click_and_drag(),
            );
            self.remember(format!("timeline.bookmark.{}", id.0), &response);
            if response.drag_started() && self.drag.is_none() && self.bookmark_drag.is_none() {
                self.begin_bookmark_drag(cx, id, ui);
            }
            if response.clicked() {
                marker_clicked = true;
                self.open_bookmark_popover(
                    id,
                    egui::pos2(hit.center().x, hit.bottom() + theme::space::MD),
                    note,
                    duration,
                );
                self.push(Action::Seek(start));
            }
        }
        if !marker_clicked
            && self.bookmark_drag.is_none()
            && (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.close_bookmark_popover();
            let at = frame_snap(cx, self.tick_at(pointer.x, layout.ruler.left()));
            self.push(Action::Seek(at));
        }
    }
    pub(in crate::editor::timeline) fn paint_playhead(
        &self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let ink = Ink::of(cx);
        let x = self.playhead_x(cx, layout.body.left());
        if x < layout.body.left() - 2.0 || x > layout.body.right() + 2.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(layout.ruler.left(), layout.ruler.top()),
            egui::pos2(layout.ruler.right(), layout.body.bottom()),
        ));
        let x = x.round() + 0.5;
        let width = theme::size::PLAYHEAD_HANDLE_WIDTH;
        let height = theme::size::PLAYHEAD_HANDLE_HEIGHT;
        let top = layout.ruler.top();
        painter.line_segment(
            [
                egui::pos2(x, top + height - 1.0),
                egui::pos2(x, layout.body.bottom()),
            ],
            egui::Stroke::new(1.0, ink.accent()),
        );
        // Shield: rounded top corners, sides taper to a rounded point at the bottom.
        let half = width / 2.0;
        let shoulder = top + height - 6.0;
        let corner = 2.0;
        let points = vec![
            egui::pos2(x - half + corner, top + 0.5),
            egui::pos2(x + half - corner, top + 0.5),
            egui::pos2(x + half, top + 0.5 + corner),
            egui::pos2(x + half, shoulder),
            egui::pos2(x + 1.5, top + height - 0.5),
            egui::pos2(x - 1.5, top + height - 0.5),
            egui::pos2(x - half, shoulder),
            egui::pos2(x - half, top + 0.5 + corner),
        ];
        painter.add(egui::Shape::convex_polygon(
            points,
            ink.panel(),
            egui::Stroke::new(1.0, ink.accent()),
        ));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RulerConfig {
    pub(super) label_seconds: f64,
    pub(super) tick_seconds: f64,
    pub(super) label_multiple: i64,
}
const LABEL_FRAME_INTERVALS: [f64; 5] = [2.0, 3.0, 5.0, 10.0, 15.0];
const TICK_FRAME_INTERVALS: [f64; 6] = [1.0, 2.0, 3.0, 5.0, 10.0, 15.0];
const SECOND_INTERVALS: [f64; 14] = [
    1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0,
];
/// Dynamic fit minimum: content occupies 25% of the visible width at minimum.
pub(in crate::editor) fn ruler_config(zoom: f32, rate: FrameRate) -> RulerConfig {
    let fps = rate.as_f64();
    let pixels_per_second = f64::from(zoom.max(MIN_ZOOM_FLOOR));
    let pixels_per_frame = pixels_per_second / fps;
    let label_seconds = optimal_ruler_interval(
        pixels_per_frame,
        pixels_per_second,
        fps,
        f64::from(RULER_LABEL_SPACING),
        &LABEL_FRAME_INTERVALS,
    );
    let raw_tick = optimal_ruler_interval(
        pixels_per_frame,
        pixels_per_second,
        fps,
        f64::from(RULER_TICK_SPACING),
        &TICK_FRAME_INTERVALS,
    );
    // Whole-second labels split into ten ticks when they fit (221643: 3 s label,
    // ten 0.3 s ticks 21.4 px apart); finer labels keep the frame-based ticks.
    let tick = if label_seconds >= 1.0
        && pixels_per_second * label_seconds / 10.0 >= f64::from(RULER_TICK_SPACING)
    {
        label_seconds / 10.0
    } else {
        tick_dividing_label(label_seconds, raw_tick, pixels_per_second, fps)
    };
    let multiple = (label_seconds / tick).round().max(1.0) as i64;
    RulerConfig {
        label_seconds,
        tick_seconds: label_seconds / multiple as f64,
        label_multiple: multiple,
    }
}
pub(in crate::editor) fn optimal_ruler_interval(
    pixels_per_frame: f64,
    pixels_per_second: f64,
    fps: f64,
    minimum_spacing: f64,
    frame_intervals: &[f64],
) -> f64 {
    for interval in frame_intervals {
        if pixels_per_frame * interval >= minimum_spacing {
            return interval / fps;
        }
    }
    for seconds in SECOND_INTERVALS {
        if pixels_per_second * seconds >= minimum_spacing {
            return seconds;
        }
    }
    let mut seconds = *SECOND_INTERVALS.last().unwrap_or(&3600.0);
    while pixels_per_second * seconds < minimum_spacing {
        seconds *= 2.0;
    }
    seconds
}
pub(in crate::editor) fn tick_dividing_label(
    label: f64,
    raw_tick: f64,
    pixels_per_second: f64,
    fps: f64,
) -> f64 {
    if divides(label, raw_tick) {
        return raw_tick;
    }
    for interval in TICK_FRAME_INTERVALS {
        let candidate = interval / fps;
        if divides(label, candidate)
            && candidate * pixels_per_second >= f64::from(RULER_TICK_SPACING)
        {
            return candidate;
        }
    }
    for candidate in SECOND_INTERVALS {
        if divides(label, candidate)
            && candidate * pixels_per_second >= f64::from(RULER_TICK_SPACING)
        {
            return candidate;
        }
    }
    label
}
pub(in crate::editor) fn divides(label: f64, candidate: f64) -> bool {
    if candidate <= 0.0 {
        return false;
    }
    let ratio = label / candidate;
    (ratio - ratio.round()).abs() < 1e-6
}
pub(in crate::editor) fn snapped_seconds(seconds: f64, rate: FrameRate) -> f64 {
    Tick::from_seconds_f64(seconds, Rounding::Nearest)
        .ok()
        .and_then(|tick| rate.snap_nearest(tick).ok())
        .map_or(seconds, |tick| tick.seconds())
}

pub(in crate::editor) fn ruler_label(seconds: f64, rate: FrameRate) -> String {
    let tick = Tick::from_seconds_f64(seconds, Rounding::Nearest)
        .ok()
        .and_then(|tick| rate.snap_nearest(tick).ok());
    match tick {
        Some(tick) => time::format_timecode(tick, rate).unwrap_or_else(|_| display_time(tick)),
        None => display_time(Tick::ZERO),
    }
}

/// Ruler caption: `MM:SS` (or `H:MM:SS`) while labels fall on whole seconds,
/// the full frame timecode once they are finer than a second.
pub(in crate::editor) fn ruler_text(seconds: f64, rate: FrameRate, label_seconds: f64) -> String {
    if label_seconds >= 1.0 - 1e-6 {
        let whole = seconds.max(0.0).round() as u64;
        let (hours, minutes, secs) = (whole / 3600, whole / 60 % 60, whole % 60);
        if hours > 0 {
            format!("{hours}:{minutes:02}:{secs:02}")
        } else {
            format!("{minutes:02}:{secs:02}")
        }
    } else {
        ruler_label(seconds, rate)
    }
}

#[cfg(test)]
mod tests;
