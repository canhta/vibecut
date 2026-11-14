//! Verifies timeline scale behaviour.
use super::*;
use crate::editor::timeline::{DEFAULT_ZOOM, ZOOM_STEP};

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.01_f32.max(right.abs() * 1e-5)
}

#[test]
fn fit_zoom_uses_a_quarter_of_narrow_and_wide_viewports() {
    for width in [320.0_f32, 720.0, 1600.0] {
        let duration = 42.0;
        let min = fit_zoom(width, duration);
        assert!(close((duration * f64::from(min)) as f32, width * 0.25));
    }
    assert_eq!(fit_zoom(16_000.0, 1.0), MAX_ZOOM);
    assert_eq!(fit_zoom(1600.0, 0.001), MAX_ZOOM);
    assert_eq!(fit_zoom(1600.0, 0.0), DEFAULT_ZOOM * 10.0);
    let duration = Tick(i64::MAX).seconds();
    assert!(fit_zoom(320.0, duration).is_finite());
    assert!(max_scroll_seconds(duration, 320.0, MAX_ZOOM, MIN_ZOOM_FLOOR).is_finite());
    // A long timeline can fit below 0.1x; a short one above it.
    assert!(fit_zoom(320.0, 3600.0) < DEFAULT_ZOOM * 0.1);
    assert!(fit_zoom(1600.0, 4.0) > DEFAULT_ZOOM * 0.1);
}

#[test]
fn zoom_padding_moves_from_three_quarters_to_fifteen_percent() {
    let min = fit_zoom(1000.0, 10.0);
    assert!(close(padding_ratio(min, min), PADDING_MAX_RATIO));
    let at_transition = min + (MAX_ZOOM - min) * PADDING_MIN_AT_ZOOM_PERCENT;
    assert!(close(padding_ratio(at_transition, min), PADDING_MIN_RATIO));
    assert!(close(padding_ratio(MAX_ZOOM, min), PADDING_MIN_RATIO));
    assert!(close(padding_px(1000.0, min, min), 750.0));
}

#[test]
fn slider_is_exponential_and_round_trips_and_zoom_steps_multiply() {
    let min = fit_zoom(1000.0, 10.0);
    assert!(close(slider_to_zoom(0.0, min), min));
    assert!(close(slider_to_zoom(1.0, min), MAX_ZOOM));
    assert!(close(slider_to_zoom(2.0, min), MAX_ZOOM));
    assert!(close(slider_to_zoom(-1.0, min), min));
    let midpoint = slider_to_zoom(0.5, min);
    assert!(close(midpoint, (min * MAX_ZOOM).sqrt()));
    for position in [0.0_f32, 0.1, 0.35, 0.8, 1.0] {
        let zoom = slider_to_zoom(position, min);
        assert!(close(zoom_to_slider(zoom, min), position));
    }
    let start = DEFAULT_ZOOM * 3.0;
    let stepped = start * ZOOM_STEP;
    assert!(close(stepped / ZOOM_STEP, start));
    assert_eq!((MAX_ZOOM * ZOOM_STEP).min(MAX_ZOOM), MAX_ZOOM);
}

#[test]
fn playhead_anchor_holds_offset_then_restores_pre_anchor_scroll() {
    let min = fit_zoom(1000.0, 40.0);
    let before = slider_to_zoom(0.1, min);
    let after = slider_to_zoom(0.35, min);
    let playhead = 20.0;
    let scroll = 2.0;
    let (held, saved) = anchored_scroll(before, after, min, scroll, playhead, None);
    let expected = playhead - (playhead - scroll) * before / after;
    assert!(close(held, expected));
    assert_eq!(saved, Some(scroll * before));
    let (restored, saved) = anchored_scroll(after, before, min, held, playhead, saved);
    assert!(close(restored, scroll));
    assert_eq!(saved, None);
    // Below the threshold the same pixel offset is kept.
    let (scaled, _) = anchored_scroll(min, before, min, 3.0, playhead, None);
    assert!(close(scaled, 3.0 * min / before));
    for target in [MAX_ZOOM, before, min] {
        let (scroll, _) = anchored_scroll(after, target, min, held, 0.0, saved);
        assert!(scroll.is_finite() && scroll >= 0.0);
    }
}

#[test]
fn scroll_clamps_to_content_and_padding_bounds() {
    let min = fit_zoom(800.0, 12.0);
    assert!(close(max_scroll_seconds(12.0, 800.0, min, min), 0.0));
    let max = max_scroll_seconds(12.0, 800.0, DEFAULT_ZOOM, min);
    assert!(max > 0.0);
    assert!(close(1000.0_f32.clamp(0.0, max), max));
    assert!(close((-5.0_f32).clamp(0.0, max), 0.0));
    assert!(close(max_scroll_seconds(0.0, 800.0, min, min), 0.0));
}

#[test]
fn edge_auto_scroll_is_proportional_capped_and_stops_at_bounds() {
    let body = Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(400.0, 200.0));
    let at = |x: f32| Some(egui::pos2(x, 100.0));
    assert!(close(
        edge_auto_scroll(at(10.0), body, 100.0, 1000.0),
        -EDGE_AUTO_SCROLL_MAX_STEP
    ));
    assert!(close(edge_auto_scroll(at(60.0), body, 100.0, 1000.0), -7.5));
    assert!(close(
        edge_auto_scroll(at(410.0), body, 100.0, 1000.0),
        15.0
    ));
    assert!(close(edge_auto_scroll(at(360.0), body, 100.0, 1000.0), 7.5));
    assert_eq!(edge_auto_scroll(at(9.0), body, 100.0, 1000.0), 0.0);
    assert_eq!(edge_auto_scroll(at(411.0), body, 100.0, 1000.0), 0.0);
    assert_eq!(edge_auto_scroll(at(10.0), body, 0.0, 1000.0), 0.0);
    assert_eq!(edge_auto_scroll(at(410.0), body, 1000.0, 1000.0), 0.0);
    assert_eq!(edge_auto_scroll(None, body, 100.0, 1000.0), 0.0);
}

#[test]
fn playback_pages_the_view_to_the_playhead_until_the_user_scrolls() {
    let mut state = State::new(DEFAULT_ZOOM, 0.0);
    let (view, max) = (10.0, 100.0);
    // Paused: the view stays where it is, even with the playhead off screen.
    state.follow_playback(false, 25.0, view, max);
    assert_eq!(state.scroll, 0.0);
    // Playing with the playhead inside the view: no move.
    state.follow_playback(true, 5.0, view, max);
    assert_eq!(state.scroll, 0.0);
    // Past the right edge the page turns and the playhead starts it.
    state.follow_playback(true, 10.5, view, max);
    assert_eq!(state.scroll, 10.5);
    // The user scrolls away; playback leaves the view alone.
    state.scroll = 40.0;
    state.end_follow_on_scroll(egui::Vec2::new(0.0, 3.0));
    state.follow_playback(true, 12.0, view, max);
    assert_eq!(state.scroll, 40.0);
    // A pause and the next play follow again, from the playhead.
    state.follow_playback(false, 12.0, view, max);
    state.follow_playback(true, 12.0, view, max);
    assert_eq!(state.scroll, 12.0);
}

#[test]
fn follow_scroll_pages_only_when_the_playhead_leaves_the_view() {
    assert_eq!(follow_scroll(5.0, 0.0, 10.0, 100.0), None);
    assert_eq!(follow_scroll(10.0, 0.0, 10.0, 100.0), None);
    assert_eq!(follow_scroll(10.5, 0.0, 10.0, 100.0), Some(10.5));
    // Behind the view the page starts at the playhead too.
    assert_eq!(follow_scroll(3.0, 8.0, 10.0, 100.0), Some(3.0));
    // Clamped to the scroll bounds at the end of the timeline.
    assert_eq!(follow_scroll(98.0, 0.0, 10.0, 90.0), Some(90.0));
    assert_eq!(follow_scroll(1.0, 0.0, 10.0, 0.0), None);
}
