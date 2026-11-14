//! Maintains the timeline viewport.

use crate::editor::timeline::{
    Context, EDGE_AUTO_SCROLL_MAX_STEP, EDGE_AUTO_SCROLL_ZONE, HORIZONTAL_WHEEL_STEP, Id, MAX_ZOOM,
    MIN_ZOOM_FLOOR, PADDING_MAX_RATIO, PADDING_MIN_AT_ZOOM_PERCENT, PADDING_MIN_RATIO, Rect, State,
    TRACK_GAP, Tick, TimelineLayout, TrackRow, Vec2, ZOOM_ANCHOR_THRESHOLD, egui, theme, view::Ink,
};

impl State {
    pub(in crate::editor::timeline) fn scroll_bounds(
        &self,
        layout: &TimelineLayout,
        duration: Tick,
    ) -> (f32, f32) {
        let view = (layout.body.width() / self.zoom).max(0.001);
        let min = fit_zoom(layout.body.width(), duration.seconds());
        let max = max_scroll_seconds(duration.seconds(), layout.body.width(), self.zoom, min);
        (view, max)
    }
    pub(in crate::editor::timeline) fn scroll_input(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        if self.drag.is_some() {
            return;
        }
        let hovered = ui.rect_contains_pointer(layout.body)
            || ui.rect_contains_pointer(layout.ruler)
            || ui.rect_contains_pointer(layout.labels);
        if !hovered {
            return;
        }
        let (scroll, zoom, modifiers) = ui.ctx().input(|input| {
            (
                input.smooth_scroll_delta(),
                input.zoom_delta(),
                input.modifiers,
            )
        });
        let duration = cx.staged_project().duration();
        let (_, max_scroll) = self.scroll_bounds(layout, duration);
        if (zoom - 1.0).abs() > 0.01 {
            self.apply_zoom(self.zoom * zoom, duration, cx.playhead);
        } else if modifiers.shift || scroll.x.abs() > scroll.y.abs() {
            self.end_follow_on_scroll(scroll);
            let raw = if modifiers.shift && scroll.x.abs() <= f32::EPSILON {
                scroll.y
            } else if scroll.x.abs() > scroll.y.abs() {
                scroll.x
            } else {
                scroll.y
            };
            let delta = raw.clamp(-HORIZONTAL_WHEEL_STEP, HORIZONTAL_WHEEL_STEP);
            self.scroll = (self.scroll - delta / self.zoom).clamp(0.0, max_scroll);
        } else {
            self.end_follow_on_scroll(scroll);
            self.vertical_scroll = (self.vertical_scroll - scroll.y).max(0.0);
        }
    }
    /// A wheel scroll is the user's own view: playback stops moving it until the next play.
    fn end_follow_on_scroll(&mut self, scroll: egui::Vec2) {
        if scroll != egui::Vec2::ZERO {
            self.follow_playhead = false;
        }
    }
    /// Page-style follow while playback runs. Playing starts a new follow; a paused
    /// transport never moves the view.
    pub(in crate::editor::timeline) fn follow_transport(
        &mut self,
        cx: &Context<'_>,
        layout: &TimelineLayout,
    ) {
        let (view, max_scroll) = self.scroll_bounds(layout, cx.staged_project().duration());
        self.follow_playback(
            cx.playback.playing(),
            cx.playhead.seconds() as f32,
            view,
            max_scroll,
        );
    }
    /// The follow state machine over one frame's transport, in seconds.
    pub(in crate::editor::timeline) fn follow_playback(
        &mut self,
        playing: bool,
        playhead: f32,
        view: f32,
        max_scroll: f32,
    ) {
        if playing && !self.was_playing {
            self.follow_playhead = true;
        }
        self.was_playing = playing;
        if !playing || !self.follow_playhead {
            return;
        }
        if let Some(scroll) = follow_scroll(playhead, self.scroll, view, max_scroll) {
            self.scroll = scroll;
        }
    }
    pub(in crate::editor::timeline) fn scrollbars(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        duration: Tick,
        rows: &[TrackRow],
    ) {
        let ink = Ink::of(cx);
        let (view, max) = self.scroll_bounds(layout, duration);
        let content = view + max;
        let track = layout.horizontal;
        let thumb_width =
            ((view / content) * track.width()).clamp(12.0_f32.min(track.width()), track.width());
        let fraction = if max > 0.0 { self.scroll / max } else { 0.0 };
        let thumb_x = track.left() + fraction * (track.width() - thumb_width);
        let thumb = Rect::from_min_size(
            egui::pos2(thumb_x, track.top()),
            Vec2::new(thumb_width, track.height()),
        );
        ui.painter()
            .rect_filled(layout.horizontal, theme::radius::PANEL, ink.panel());
        ui.painter()
            .rect_filled(track, theme::radius::CONTROL, ink.field());
        ui.painter()
            .rect_filled(thumb, theme::radius::CONTROL, ink.border());
        let response = ui.interact(
            layout.horizontal,
            Id::unique("timeline.scroll_h"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.scroll_h", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && max > 0.0
            && track.width() > 0.0
        {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            self.scroll = fraction * max;
            self.follow_playhead = false;
        }
        let content_height: f32 = rows.iter().map(|row| row.height + TRACK_GAP).sum::<f32>();
        let max_vertical = (content_height - TRACK_GAP - layout.body.height()).max(0.0);
        let track = layout.vertical.shrink2(Vec2::new(3.0, 0.0));
        let thumb_height = if content_height > 0.0 {
            ((layout.body.height() / content_height) * track.height())
                .clamp(12.0_f32.min(track.height()), track.height())
        } else {
            track.height()
        };
        let fraction = if max_vertical > 0.0 {
            self.vertical_scroll / max_vertical
        } else {
            0.0
        };
        let thumb_y = track.top() + fraction * (track.height() - thumb_height);
        ui.painter()
            .rect_filled(layout.vertical, theme::radius::PANEL, ink.panel());
        ui.painter()
            .rect_filled(track, theme::radius::CONTROL, ink.field());
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(track.left(), thumb_y),
                Vec2::new(track.width(), thumb_height),
            ),
            6.0,
            ink.border(),
        );
        let response = ui.interact(
            layout.vertical,
            Id::unique("timeline.scroll_v"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.scroll_v", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && max_vertical > 0.0
            && track.height() > 0.0
        {
            let fraction = ((pointer.y - track.top()) / track.height()).clamp(0.0, 1.0);
            self.vertical_scroll = fraction * max_vertical;
            self.follow_playhead = false;
        }
    }
}

pub(in crate::editor::timeline) fn fit_zoom(viewport_width: f32, duration_seconds: f64) -> f32 {
    let width = f64::from(viewport_width.max(1.0));
    let seconds = if duration_seconds > 0.0 {
        duration_seconds
    } else {
        1.0
    };
    let fit = width * f64::from(1.0 - PADDING_MAX_RATIO) / seconds;
    if !fit.is_finite() {
        return MIN_ZOOM_FLOOR;
    }
    (fit as f32).clamp(MIN_ZOOM_FLOOR, MAX_ZOOM)
}
pub(in crate::editor::timeline) fn zoom_percent(zoom: f32, min: f32) -> f32 {
    let span = MAX_ZOOM - min;
    if span <= 0.0 {
        1.0
    } else {
        ((zoom - min) / span).clamp(0.0, 1.0)
    }
}
pub(in crate::editor::timeline) fn slider_to_zoom(position: f32, min: f32) -> f32 {
    let min = min.clamp(MIN_ZOOM_FLOOR, MAX_ZOOM);
    if MAX_ZOOM <= min {
        return min;
    }
    min * (MAX_ZOOM / min).powf(position.clamp(0.0, 1.0))
}
pub(in crate::editor::timeline) fn zoom_to_slider(zoom: f32, min: f32) -> f32 {
    let min = min.clamp(MIN_ZOOM_FLOOR, MAX_ZOOM);
    if MAX_ZOOM <= min {
        return 0.0;
    }
    (zoom.clamp(min, MAX_ZOOM) / min).ln() / (MAX_ZOOM / min).ln()
}
pub(in crate::editor::timeline) fn padding_ratio(zoom: f32, min: f32) -> f32 {
    let transition = (zoom_percent(zoom, min) / PADDING_MIN_AT_ZOOM_PERCENT).min(1.0);
    PADDING_MAX_RATIO - (PADDING_MAX_RATIO - PADDING_MIN_RATIO) * transition
}
pub(in crate::editor::timeline) fn padding_px(viewport_width: f32, zoom: f32, min: f32) -> f32 {
    viewport_width.max(0.0) * padding_ratio(zoom, min)
}
pub(in crate::editor::timeline) fn max_scroll_seconds(
    duration_seconds: f64,
    viewport_width: f32,
    zoom: f32,
    min: f32,
) -> f32 {
    let view = (viewport_width / zoom.max(MIN_ZOOM_FLOOR)).max(0.001);
    let padding = padding_px(viewport_width, zoom, min);
    (duration_seconds as f32 + padding / zoom - view).max(0.0)
}
/// Scroll while keeping the playhead's viewport offset; restore the pre-anchor
/// scroll when the zoom slider falls back below the threshold.
pub(in crate::editor::timeline) fn anchored_scroll(
    old_zoom: f32,
    new_zoom: f32,
    min: f32,
    scroll: f32,
    playhead: f32,
    pre_anchor: Option<f32>,
) -> (f32, Option<f32>) {
    let old_percent = zoom_to_slider(old_zoom, min);
    let new_percent = zoom_to_slider(new_zoom, min);
    let mut pre_anchor = pre_anchor;
    if old_percent < ZOOM_ANCHOR_THRESHOLD && new_percent >= ZOOM_ANCHOR_THRESHOLD {
        pre_anchor = Some(scroll * old_zoom);
    }
    let next = if new_percent >= ZOOM_ANCHOR_THRESHOLD {
        let offset = (playhead - scroll) * old_zoom;
        playhead - offset / new_zoom
    } else if old_percent >= ZOOM_ANCHOR_THRESHOLD && new_percent < ZOOM_ANCHOR_THRESHOLD {
        pre_anchor
            .take()
            .map_or(scroll * old_zoom / new_zoom, |pixels| pixels / new_zoom)
    } else {
        scroll * old_zoom / new_zoom
    };
    (next.max(0.0), pre_anchor)
}
/// Page-style follow: while the playhead is outside the visible span, the scroll that starts
/// the page at it (clamped to the scroll bounds); `None` while it stays visible. Spans and
/// the scroll are in seconds.
pub(in crate::editor::timeline) fn follow_scroll(
    playhead: f32,
    scroll: f32,
    view: f32,
    max_scroll: f32,
) -> Option<f32> {
    let outside = playhead < scroll || playhead > scroll + view;
    outside.then(|| playhead.clamp(0.0, max_scroll.max(0.0)))
}
/// Pointer-distance-proportional edge auto-scroll in logical pixels per frame.
pub(in crate::editor::timeline) fn edge_auto_scroll(
    pointer: Option<egui::Pos2>,
    body: Rect,
    scroll_seconds: f32,
    max_scroll_seconds: f32,
) -> f32 {
    let Some(pointer) = pointer else {
        return 0.0;
    };
    if !body.contains(pointer) || body.width() <= 0.0 {
        return 0.0;
    }
    let relative = pointer.x - body.left();
    if relative < EDGE_AUTO_SCROLL_ZONE && scroll_seconds > 0.0 {
        let intensity = (1.0 - relative / EDGE_AUTO_SCROLL_ZONE).clamp(0.0, 1.0);
        -EDGE_AUTO_SCROLL_MAX_STEP * intensity
    } else if relative > body.width() - EDGE_AUTO_SCROLL_ZONE && scroll_seconds < max_scroll_seconds
    {
        let distance = (body.width() - relative).max(0.0);
        let intensity = (1.0 - distance / EDGE_AUTO_SCROLL_ZONE).clamp(0.0, 1.0);
        EDGE_AUTO_SCROLL_MAX_STEP * intensity
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
