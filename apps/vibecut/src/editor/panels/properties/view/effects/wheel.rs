//! Maps colour-wheel pointer positions to stored lift/gamma/gain percentages.
use crate::editor::Text;

const TAU: f64 = std::f64::consts::TAU;
/// One neutral wheel keeps the grade effect's identity values.
const NEUTRAL_GAIN: f64 = 100.0;
/// Stored percentages keep the exact inverse so a drag never drifts.
const PERCENT: f64 = 100.0;

/// One of the three lift/gamma/gain wheel families.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Family {
    Lift,
    Gamma,
    Gain,
}

impl Family {
    pub(super) const ALL: [Self; 3] = [Self::Lift, Self::Gamma, Self::Gain];

    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Lift => "lift",
            Self::Gamma => "gamma",
            Self::Gain => "gain",
        }
    }

    pub(super) const fn keys(self) -> [&'static str; 3] {
        match self {
            Self::Lift => ["lift_r", "lift_g", "lift_b"],
            Self::Gamma => ["gamma_r", "gamma_g", "gamma_b"],
            Self::Gain => ["gain_r", "gain_g", "gain_b"],
        }
    }

    pub(super) const fn label(self) -> Text {
        match self {
            Self::Lift => Text::EffectLift,
            Self::Gamma => Text::EffectGamma,
            Self::Gain => Text::EffectGain,
        }
    }

    pub(super) const fn neutral(self) -> [f64; 3] {
        match self {
            Self::Lift => [0.0; 3],
            Self::Gamma | Self::Gain => [NEUTRAL_GAIN; 3],
        }
    }

    pub(super) fn is_neutral(self, values: [f64; 3]) -> bool {
        values
            .iter()
            .zip(self.neutral())
            .all(|(value, neutral)| (value - neutral).abs() < 0.05)
    }

    /// Stored percentage to the wheel's neutral-normalized coefficient.
    fn coefficient(self, stored: f64) -> f64 {
        match self {
            Self::Lift => (stored / PERCENT).clamp(-1.0, 1.0),
            Self::Gamma => (stored.clamp(25.0, 400.0) / PERCENT).log2() / 2.0,
            Self::Gain => {
                let gain = (stored / PERCENT).clamp(0.0, 4.0);
                if gain <= 1.0 {
                    gain - 1.0
                } else {
                    (gain - 1.0) / 3.0
                }
            }
        }
    }

    /// Neutral-normalized coefficient to the stored percentage.
    fn percent(self, coefficient: f64) -> f64 {
        let z = coefficient.clamp(-1.0, 1.0);
        match self {
            Self::Lift => z * PERCENT,
            Self::Gamma => 2.0_f64.powf(2.0 * z) * PERCENT,
            Self::Gain if z <= 0.0 => (1.0 + z) * PERCENT,
            Self::Gain => (1.0 + 3.0 * z) * PERCENT,
        }
    }
}

/// A wheel position: the triangular chroma puck plus its shared master level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Wheel {
    pub(super) master: f64,
    pub(super) hue: f64,
    pub(super) radius: f64,
}

impl Wheel {
    pub(super) fn from_values(family: Family, values: [f64; 3]) -> Self {
        let z = values.map(|value| family.coefficient(value));
        let master = (z[0] + z[1] + z[2]) / 3.0;
        if master.abs() >= 1.0 - 1e-9 {
            return Self {
                master: master.clamp(-1.0, 1.0),
                hue: 0.0,
                radius: 0.0,
            };
        }
        let delta = z.map(|value| value - master);
        if delta.iter().all(|value| value.abs() < 1e-9) {
            return Self {
                master,
                hue: 0.0,
                radius: 0.0,
            };
        }
        let hue =
            (3.0_f64.sqrt() * (delta[1] - delta[2])).atan2(2.0 * delta[0] - delta[1] - delta[2]);
        let limit = limit(master, basis(hue));
        let radius = if limit <= 0.0 {
            0.0
        } else {
            (delta.iter().fold(0.0_f64, |m, value| m.max(value.abs())) / limit).clamp(0.0, 1.0)
        };
        Self {
            master,
            hue,
            radius,
        }
    }

    pub(super) fn values(self, family: Family) -> [f64; 3] {
        let master = self.master.clamp(-1.0, 1.0);
        let z = if self.radius <= 0.0 || master.abs() >= 1.0 - 1e-9 {
            [master; 3]
        } else {
            let direction = basis(self.hue);
            let limit = limit(master, direction);
            direction.map(|axis| (master + self.radius * limit * axis).clamp(-1.0, 1.0))
        };
        z.map(|value| family.percent(value))
    }

    /// The same chroma with a new centre level; the puck stays where the creator put it.
    pub(super) fn with_master(self, master: f64) -> Self {
        let master = master.clamp(-1.0, 1.0);
        Self {
            master,
            hue: if master.abs() >= 1.0 { 0.0 } else { self.hue },
            radius: if master.abs() >= 1.0 {
                0.0
            } else {
                self.radius
            },
        }
    }

    /// A pointer position in wheel radii from the centre, up positive.
    pub(super) fn with_puck(self, x: f64, y: f64) -> Self {
        let radius = (x * x + y * y).sqrt();
        if radius <= 1e-9 {
            return Self {
                master: self.master,
                hue: 0.0,
                radius: 0.0,
            };
        }
        Self {
            master: self.master,
            hue: y.atan2(x),
            radius: radius.min(1.0),
        }
    }
}

/// Unit basis of the triangular chroma direction at `hue`, zero-sum and peak 1.
fn basis(hue: f64) -> [f64; 3] {
    let values = [hue.cos(), (hue - TAU / 3.0).cos(), (hue + TAU / 3.0).cos()];
    let peak = values.iter().fold(0.0_f64, |m, value| m.max(value.abs()));
    if peak <= f64::EPSILON {
        [0.0; 3]
    } else {
        values.map(|value| value / peak)
    }
}

/// Largest chroma the triangle can add at `master` without any channel leaving -1..=1.
fn limit(master: f64, direction: [f64; 3]) -> f64 {
    let mut limit = f64::INFINITY;
    for value in direction {
        if value > 0.0 {
            limit = limit.min((1.0 - master) / value);
        } else if value < 0.0 {
            limit = limit.min((-1.0 - master) / value);
        }
    }
    if limit.is_finite() && limit > 0.0 {
        limit
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Family, TAU, Wheel};

    fn assert_close(actual: f64, expected: f64, label: &str) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{label}: {actual} != {expected}"
        );
    }

    #[test]
    fn neutral_and_master_extremes_centre_the_puck() {
        for family in Family::ALL {
            let wheel = Wheel::from_values(family, family.neutral());
            assert_close(wheel.master, 0.0, "neutral master");
            assert_close(wheel.hue, 0.0, "neutral hue");
            assert_close(wheel.radius, 0.0, "neutral radius");
            assert_eq!(wheel.values(family), family.neutral());
            for master in [-1.0, 1.0] {
                let extremes = Wheel {
                    master,
                    hue: 2.1,
                    radius: 0.7,
                };
                let values = extremes.values(family);
                assert_close(values[0], values[1], "extreme channels");
                assert_close(values[1], values[2], "extreme channels");
                let back = Wheel::from_values(family, values);
                assert_close(back.radius, 0.0, "extreme radius");
            }
        }
    }

    #[test]
    fn a_wheel_round_trips_through_the_stored_percentages() {
        for family in Family::ALL {
            for master in [-0.9, -0.35, 0.0, 0.42, 0.95] {
                for hue in [-3.0, -1.2, 0.0, 0.8, 2.6] {
                    for radius in [0.0, 0.18, 0.5, 1.0] {
                        let wheel = Wheel {
                            master,
                            hue,
                            radius,
                        };
                        let values = wheel.values(family);
                        let back = Wheel::from_values(family, values);
                        assert_close(back.master, master, "master round trip");
                        assert_close(back.radius, radius, "radius round trip");
                        if radius > 0.02 {
                            let turned = (back.hue - hue).sin().abs();
                            assert!(turned < 1e-9, "hue round trip {hue} -> {}", back.hue);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_puck_keeps_hue_and_radius_when_the_master_moves() {
        for family in Family::ALL {
            let wheel = Wheel {
                master: 0.2,
                hue: TAU / 3.0,
                radius: 0.4,
            };
            let moved = wheel.with_master(-0.6);
            let values = moved.values(family);
            let back = Wheel::from_values(family, values);
            assert_close(back.master, -0.6, "master");
            assert_close(back.radius, 0.4, "radius");
            assert_close(back.hue, TAU / 3.0, "hue");
        }
    }

    #[test]
    fn red_cardinal_sets_red_above_the_other_channels() {
        let wheel = Wheel {
            master: 0.0,
            hue: 0.0,
            radius: 0.5,
        };
        let values = wheel.values(Family::Lift);
        assert!(values[0] > 0.0, "red lifts");
        assert_close(values[1], values[2], "green and blue stay equal");
        assert!(values[1] < 0.0, "green and blue fall");
        let back = Wheel::from_values(Family::Lift, values);
        assert_close(back.hue, 0.0, "red hue");
        assert_close(back.radius, 0.5, "red radius");
    }

    #[test]
    fn every_wheel_position_stays_inside_its_admitted_range() {
        for family in Family::ALL {
            for master in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                for hue in (0..24).map(|step| step as f64 / 24.0 * TAU) {
                    for radius in [0.0, 0.25, 0.5, 0.75, 1.0] {
                        let values = Wheel {
                            master,
                            hue,
                            radius,
                        }
                        .values(family);
                        let (min, max) = match family {
                            Family::Lift => (-100.0, 100.0),
                            Family::Gamma => (25.0, 400.0),
                            Family::Gain => (0.0, 400.0),
                        };
                        for value in values {
                            assert!((min..=max).contains(&value), "{family:?}: {value}");
                        }
                    }
                }
            }
        }
    }
}
