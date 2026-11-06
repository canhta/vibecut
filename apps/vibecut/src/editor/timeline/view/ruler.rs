//! Renders the timeline ruler.

use crate::editor::timeline::{
    Align2, BookmarkView, Editor, FontId, FrameRate, Id, MIN_ZOOM_FLOOR, RULER_LABEL_SPACING,
    RULER_TICK_SPACING, Rect, Rounding, Tick, TimelineLayout, Vec2, display_time, drop_frame, egui,
    theme,
};

impl Editor {
    pub(in crate::editor::timeline) fn paint_ruler(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        bookmarks: &[BookmarkView],
    ) {
        let painter = ui.painter().with_clip_rect(layout.ruler);
        painter.rect_filled(layout.ruler, theme::radius::PANEL, self.control_fill());
        painter.rect_filled(layout.corner, theme::radius::PANEL, self.control_fill());
        let rate = self.session.project().data().canvas.fps;
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
                let height = if labelled { 8.0 } else { 4.0 };
                painter.line_segment(
                    [
                        egui::pos2(x, layout.ruler.bottom() - height),
                        egui::pos2(x, layout.ruler.bottom()),
                    ],
                    egui::Stroke::new(1.0, self.border()),
                );
                if labelled {
                    painter.text(
                        egui::pos2(x + 4.0, layout.ruler.top() + 2.0),
                        Align2::LEFT_TOP,
                        ruler_label(seconds, rate),
                        FontId::proportional(theme::type_size::BODY),
                        self.secondary(),
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
                    self.accent(),
                );
                bookmark_hits.push((
                    bookmark.id,
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
        for (id, hit) in bookmark_hits {
            let response = ui.interact(
                hit,
                Id::unique(("timeline.bookmark", id.0)),
                egui::Sense::click_and_drag(),
            );
            self.remember(format!("timeline.bookmark.{}", id.0), &response);
            if response.drag_started() && self.drag.is_none() && self.bookmark_drag.is_none() {
                self.begin_bookmark_drag(id, ui);
            }
        }
        if self.bookmark_drag.is_none()
            && (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = self.frame_snap(self.tick_at(pointer.x, layout.ruler.left()));
            self.seek_to(at);
        }
    }
    pub(in crate::editor::timeline) fn paint_playhead(
        &self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 2.0 || x > layout.body.right() + 2.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(layout.ruler.left(), layout.ruler.top()),
            egui::pos2(layout.ruler.right(), layout.body.bottom()),
        ));
        painter.line_segment(
            [
                egui::pos2(x, layout.ruler.bottom()),
                egui::pos2(x, layout.body.bottom()),
            ],
            egui::Stroke::new(2.0, self.accent()),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - 5.0, layout.ruler.bottom() - 8.0),
                egui::pos2(x + 5.0, layout.ruler.bottom() - 8.0),
                egui::pos2(x, layout.ruler.bottom()),
            ],
            self.accent(),
            egui::Stroke::NONE,
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
    let tick = tick_dividing_label(label_seconds, raw_tick, pixels_per_second, fps);
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
        Some(tick) => time::format_timecode(tick, rate, drop_frame(rate))
            .unwrap_or_else(|_| display_time(tick)),
        None => display_time(Tick::ZERO),
    }
}

#[cfg(test)]
mod tests;
