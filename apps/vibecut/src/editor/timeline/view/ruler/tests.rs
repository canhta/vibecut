//! Verifies timeline scale behaviour.
use super::*;
use crate::editor::DEFAULT_ZOOM;
use time::TICKS_PER_SECOND;

#[test]
fn ruler_ticks_keep_labels_separated_at_every_zoom_and_rate() {
    for rate in [
        FrameRate::FPS_23_976,
        FrameRate::FPS_29_97,
        FrameRate::FPS_30,
        FrameRate::FPS_60,
        FrameRate::FPS_59_94,
        FrameRate::new(1, 10).unwrap(),
        FrameRate::new(TICKS_PER_SECOND, 1).unwrap(),
    ] {
        for factor in [0.1_f32, 1.0, 10.0, 100.0] {
            let zoom = DEFAULT_ZOOM * factor;
            let config = ruler_config(zoom, rate);
            assert!(config.tick_seconds > 0.0);
            assert!(config.label_multiple >= 1);
            assert!(config.label_seconds * rate.as_f64() >= 1.0);
            assert!(
                (config.label_seconds - config.tick_seconds * config.label_multiple as f64).abs()
                    < 1e-6,
                "labels must land on ticks"
            );
            let label_px = config.label_seconds as f32 * zoom;
            let tick_px = config.tick_seconds as f32 * zoom;
            assert!(label_px >= RULER_LABEL_SPACING - 0.05);
            assert!(tick_px >= RULER_TICK_SPACING - 0.05);
            for offset in [0.0, 30_000_000.0] {
                let first = (offset / config.label_seconds).floor() * config.label_seconds;
                let gap = (snapped_seconds(first + config.label_seconds, rate)
                    - snapped_seconds(first, rate))
                    * f64::from(zoom);
                assert!(
                    gap >= 70.0,
                    "visible timecode labels must remain separated at long durations: {gap}"
                );
            }
        }
    }
    let df = ruler_config(DEFAULT_ZOOM, FrameRate::FPS_29_97);
    assert!(df.label_multiple >= 1);
    assert!(ruler_label(1.0, FrameRate::FPS_29_97).contains(';'));
    assert!(ruler_label(1.0, FrameRate::FPS_30).contains(':'));
    assert!(ruler_label(60.0, FrameRate::FPS_59_94).contains(';'));
    assert!(!ruler_label(60.0, FrameRate::FPS_23_976).contains(';'));
}
