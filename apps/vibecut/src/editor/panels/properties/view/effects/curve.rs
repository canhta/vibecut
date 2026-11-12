//! Validates pointer proposals as monotone tone curves.
use crate::editor::Text;
use crate::{Error, Result};
use project::{CURVE_GRID, CURVE_MAX_POINTS, ToneCurve};

/// The smallest authored input step the persisted curve admits.
const STEP: f64 = 1.0 / CURVE_GRID;

/// One of the four static curves on the colour-grade effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Channel {
    Red,
    Green,
    Blue,
    Luma,
}

impl Channel {
    pub(super) const ALL: [Self; 4] = [Self::Red, Self::Green, Self::Blue, Self::Luma];

    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Luma => "luma",
        }
    }

    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Red => "curve_red",
            Self::Green => "curve_green",
            Self::Blue => "curve_blue",
            Self::Luma => "curve_luma",
        }
    }

    pub(super) const fn label(self) -> Text {
        match self {
            Self::Red => Text::EffectRed,
            Self::Green => Text::EffectGreen,
            Self::Blue => Text::EffectBlue,
            Self::Luma => Text::EffectLuma,
        }
    }
}

/// One pointer proposal in normalized plot coordinates, up positive.
#[derive(Clone, Copy, Debug)]
pub(super) enum Edit {
    Add { x: f64, y: f64 },
    Move { index: usize, x: f64, y: f64 },
    Remove { index: usize },
}

/// Apply one proposal, refusing a shape the persisted contract would reject.
pub(super) fn apply(curve: ToneCurve, edit: Edit) -> Result<ToneCurve> {
    let invalid = || Error::Invalid("tone curve edit");
    let mut points = curve.points().to_vec();
    match edit {
        Edit::Add { x, y } => {
            if points.len() >= CURVE_MAX_POINTS {
                return Err(invalid());
            }
            let x = snap(x)?;
            let y = unit(y)?;
            let at = points.partition_point(|point| point[0] < x);
            let Some(previous) = at.checked_sub(1).and_then(|index| points.get(index)) else {
                return Err(invalid());
            };
            let Some(next) = points.get(at) else {
                return Err(invalid());
            };
            if x - previous[0] < STEP || next[0] - x < STEP {
                return Err(invalid());
            }
            if y < previous[1] || y > next[1] {
                return Err(invalid());
            }
            points.insert(at, [x, y]);
        }
        Edit::Move { index, x, y } => {
            if index == 0 || index + 1 >= points.len() {
                return Err(invalid());
            }
            let Some(previous) = points.get(index - 1).copied() else {
                return Err(invalid());
            };
            let Some(next) = points.get(index + 1).copied() else {
                return Err(invalid());
            };
            let low = previous[0] + STEP;
            let high = next[0] - STEP;
            if high < low {
                return Err(invalid());
            }
            let x = snap(x)?.clamp(low, high);
            let y = unit(y)?;
            if y < previous[1] || y > next[1] {
                return Err(invalid());
            }
            if let Some(point) = points.get_mut(index) {
                *point = [x, y];
            }
        }
        Edit::Remove { index } => {
            if index == 0 || index + 1 >= points.len() {
                return Err(invalid());
            }
            points.remove(index);
        }
    }
    ToneCurve::new(&points)
}

/// Nearest exact input grid tick; a half-tick tie moves up, off-grid input refuses.
fn snap(x: f64) -> Result<f64> {
    if !x.is_finite() {
        return Err(Error::Invalid("tone curve point"));
    }
    let ticks = (x.clamp(0.0, 1.0) * CURVE_GRID + 0.5).floor();
    Ok(ticks / CURVE_GRID)
}

/// A finite output inside the 0..1 channel domain.
fn unit(y: f64) -> Result<f64> {
    if !y.is_finite() {
        return Err(Error::Invalid("tone curve point"));
    }
    Ok(y.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::{Channel, Edit, apply};
    use project::{CURVE_GRID, CURVE_MAX_POINTS, ToneCurve};

    fn identity() -> ToneCurve {
        ToneCurve::IDENTITY
    }

    fn points(curve: ToneCurve) -> Vec<[f64; 2]> {
        curve.points().to_vec()
    }

    fn interior_point(curve: ToneCurve) -> [f64; 2] {
        *curve
            .points()
            .iter()
            .find(|point| point[0] > 0.0 && point[0] < 1.0)
            .expect("an interior point")
    }

    #[test]
    fn a_curve_click_adds_one_grid_point_inside_its_neighbours() {
        let edited = apply(identity(), Edit::Add { x: 0.3, y: 0.6 }).expect("an interior point");
        let expected = (0.3 * CURVE_GRID + 0.5).floor() / CURVE_GRID;
        assert_eq!(points(edited).len(), 3);
        assert_eq!(interior_point(edited), [expected, 0.6]);
        assert!((edited.map(0.3) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn endpoints_are_fixed_and_an_off_curve_fraction_refuses() {
        for edit in [
            Edit::Move {
                index: 0,
                x: 0.2,
                y: 0.4,
            },
            Edit::Move {
                index: 1,
                x: 0.8,
                y: 0.6,
            },
            Edit::Add { x: 0.0, y: 0.2 },
            Edit::Add { x: 1.0, y: 0.8 },
            Edit::Remove { index: 0 },
            Edit::Remove { index: 1 },
            Edit::Add {
                x: f64::NAN,
                y: 0.5,
            },
            Edit::Move {
                index: 0,
                x: 0.5,
                y: f64::INFINITY,
            },
        ] {
            assert!(apply(identity(), edit).is_err(), "{edit:?}");
        }
    }

    #[test]
    fn a_dragged_point_clamps_its_input_and_refuses_a_monotonicity_break() {
        let start = apply(identity(), Edit::Add { x: 0.5, y: 0.5 }).expect("a middle point");
        let moved = apply(
            start,
            Edit::Move {
                index: 1,
                x: -2.0,
                y: 0.6,
            },
        )
        .expect("a clamped point");
        let moved_point = interior_point(moved);
        assert_eq!(moved_point[0], 1.0 / CURVE_GRID);
        assert_eq!(moved_point[1], 0.6);
        assert!(
            apply(
                start,
                Edit::Move {
                    index: 1,
                    x: 0.5,
                    y: 2.0,
                },
            )
            .is_err(),
            "a point above its next neighbour would break monotonicity"
        );
        assert!(
            apply(start, Edit::Add { x: 0.75, y: 2.0 },).is_err(),
            "a point above the segment would break monotonicity"
        );
        for pair in moved.points().windows(2) {
            assert!(pair[0][0] < pair[1][0], "strictly increasing input");
            assert!(pair[0][1] <= pair[1][1], "nondecreasing output");
        }
        for step in 0..=4096 {
            let low = moved.map(step as f32 / 4096.0);
            let high = moved.map((step + 1) as f32 / 4096.0);
            assert!(low <= high + 1e-6, "monotone at {step}");
        }
    }

    #[test]
    fn a_curve_keeps_the_contract_point_ceiling_and_removal_needs_an_interior_point() {
        let mut curve = identity();
        for step in 1..CURVE_MAX_POINTS - 1 {
            curve = apply(
                curve,
                Edit::Add {
                    x: step as f64 / CURVE_MAX_POINTS as f64,
                    y: step as f64 / CURVE_MAX_POINTS as f64,
                },
            )
            .expect("a curve point");
        }
        assert_eq!(points(curve).len(), CURVE_MAX_POINTS);
        assert!(
            apply(curve, Edit::Add { x: 0.5, y: 0.5 },).is_err(),
            "the ceiling refuses a further point"
        );
        let removed = apply(curve, Edit::Remove { index: 1 }).expect("an interior point");
        assert_eq!(points(removed).len(), CURVE_MAX_POINTS - 1);
        assert!(apply(identity(), Edit::Add { x: 0.5, y: 0.5 },).is_ok());
    }

    #[test]
    fn channel_ids_are_the_persisted_keys() {
        let keys: Vec<_> = Channel::ALL.iter().map(|channel| channel.key()).collect();
        assert_eq!(
            keys,
            ["curve_red", "curve_green", "curve_blue", "curve_luma"]
        );
    }
}
