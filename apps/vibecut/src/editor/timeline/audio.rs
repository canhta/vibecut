//! Maps clip volume between decibels and its vertical line in the timeline.
//!
//! The line is linear-gain squared: dragging near 0 dB stays precise while
//! silence compresses into the bottom of the clip. The drag grid is deliberately
//! coarser (0.1 dB) than the stored parameter grid (0.01 dB).

/// Quietest stored clip gain.
pub(in crate::editor::timeline) const DB_MIN: f32 = -60.0;
/// Loudest stored clip gain.
pub(in crate::editor::timeline) const DB_MAX: f32 = 20.0;
/// Waveform amplitude display floor, below which nothing is drawn.
pub(in crate::editor::timeline) const DISPLAY_FLOOR_DB: f32 = -40.0;
/// One drag step in dB.
pub(in crate::editor::timeline) const DRAG_STEP: f32 = 0.1;
/// One stored parameter step in dB.
#[cfg(test)]
pub(in crate::editor::timeline) const PARAM_STEP: f32 = 0.01;

fn min_gain() -> f64 {
    f64::from(10.0_f32.powf(DB_MIN / 20.0))
}
fn max_gain() -> f64 {
    f64::from(10.0_f32.powf(DB_MAX / 20.0))
}
fn normalized_gain(db: f32) -> f64 {
    let linear = 10.0_f64.powf(clamp(db) as f64 / 20.0);
    (linear - min_gain()) / (max_gain() - min_gain())
}
/// Fraction from the top of the line's travel: 0 at +20 dB, 1 at -60 dB.
pub(in crate::editor::timeline) fn line_fraction(db: f32) -> f32 {
    let normalized = normalized_gain(db).clamp(0.0, 1.0);
    (1.0 - normalized.sqrt()) as f32
}
/// Inverse of [`line_fraction`]; a position outside the band clamps.
pub(in crate::editor::timeline) fn db_at_fraction(fraction: f32) -> f32 {
    let progress = (1.0 - f64::from(fraction.clamp(0.0, 1.0))).clamp(0.0, 1.0);
    let linear = min_gain() + progress.powi(2) * (max_gain() - min_gain());
    clamp((20.0 * linear.log10()) as f32)
}
/// The stored 0.01 dB grid the engine validates.
#[cfg(test)]
pub(in crate::editor::timeline) fn quantize_param(db: f32) -> f32 {
    quantize(db, PARAM_STEP)
}
/// The coarser 0.1 dB grid a pointer drag commits.
pub(in crate::editor::timeline) fn quantize_drag(db: f32) -> f32 {
    quantize(db, DRAG_STEP)
}
fn quantize(db: f32, step: f32) -> f32 {
    let value = (clamp(db) / step).round() * step;
    clamp(value)
}
/// Visible waveform height for an output amplitude; below the floor it is flat.
pub(in crate::editor::timeline) fn bar_fraction(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * amplitude.log10();
    if db <= DISPLAY_FLOOR_DB {
        return 0.0;
    }
    (((f64::from(db - DISPLAY_FLOOR_DB) / f64::from(-DISPLAY_FLOOR_DB)).powf(1.5)) as f32)
        .clamp(0.0, 1.0)
}
fn clamp(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(DB_MIN, DB_MAX)
    } else {
        DB_MIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_mapping_hits_both_endpoints_and_round_trips() {
        for (db, fraction) in [(DB_MAX, 0.0), (DB_MIN, 1.0)] {
            assert!((line_fraction(db) - fraction).abs() < 1e-4, "{db} dB");
        }
        // 0 dB is not the midpoint: the line is squared in linear-gain space.
        assert!((line_fraction(0.0) - 0.6839).abs() < 1e-3);
        for db in [-60.0, -40.0, -12.3, -0.2, 0.0, 6.0, 20.0] {
            let round = db_at_fraction(line_fraction(db));
            assert!((round - db).abs() < 0.02, "{db} -> {round}");
        }
    }

    #[test]
    fn line_mapping_clamps_hostile_positions_and_values() {
        assert_eq!(line_fraction(-120.0), line_fraction(DB_MIN));
        assert_eq!(line_fraction(120.0), line_fraction(DB_MAX));
        assert_eq!(db_at_fraction(-1.0), DB_MAX);
        assert_eq!(db_at_fraction(2.0), DB_MIN);
        assert_eq!(line_fraction(f32::NAN), 1.0);
        assert_eq!(db_at_fraction(f32::NAN), DB_MIN);
    }

    #[test]
    fn drag_and_parameter_steps_stay_on_their_own_grids() {
        assert_eq!(quantize_drag(-6.04), -6.0);
        assert_eq!(quantize_drag(-6.05), -6.1);
        assert_eq!(quantize_drag(19.99), DB_MAX);
        assert_eq!(quantize_drag(-60.04), DB_MIN);
        assert!((quantize_param(-6.04) + 6.04).abs() < 1e-4);
        assert!((quantize_param(-6.046) + 6.05).abs() < 1e-4);
        for value in [0.0, -0.1, -12.3, 20.0] {
            let hundredths = quantize_param(value) * 100.0;
            assert!((hundredths - hundredths.round()).abs() < 1e-3);
        }
    }

    #[test]
    fn waveform_display_floor_is_minus_forty_db() {
        assert_eq!(bar_fraction(0.0), 0.0);
        assert_eq!(bar_fraction(f32::NAN), 0.0);
        assert_eq!(bar_fraction(0.01), 0.0);
        assert!(bar_fraction(0.02) > 0.0);
        assert_eq!(bar_fraction(10.0), 1.0);
        assert!((bar_fraction(1.0) - 1.0).abs() < 1e-5);
    }
}
