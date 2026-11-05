//! Clip fade-in/out: persisted shape plus the one per-sample gain evaluation that
//! preview and export both call.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use time::{Rounding, Tick};

/// Gain shape of one fade edge, read as a fade-in (the fade-out mirrors it in time).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    /// `sin(x·π/2)`: constant power against a mirrored fade (-3 dB at the midpoint).
    #[default]
    EqualPower,
    /// `x`: constant amplitude against a mirrored fade (-6 dB at the midpoint).
    Linear,
    /// `(e^{4x}-1)/(e^4-1)`: slow start, fast finish (about -18 dB at the midpoint).
    Exponential,
}
impl FadeCurve {
    /// Gain for fade progress `x` in `0..=1`; exactly 0 at 0 and exactly 1 at 1,
    /// whatever the float trigonometry returns. Outside the range clamps.
    pub fn gain(self, x: f64) -> f32 {
        if x.is_nan() || x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        (match self {
            Self::EqualPower => (x * std::f64::consts::FRAC_PI_2).sin(),
            Self::Linear => x,
            Self::Exponential => (4.0 * x).exp_m1() / 4f64.exp_m1(),
        }) as f32
    }
}

/// One fade edge. A zero `duration` means no fade.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Fade {
    /// Output ticks from the clip edge; at most the clip duration after clamping.
    pub duration: Tick,
    pub curve: FadeCurve,
}

/// Which clip edge a fade belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FadeEdge {
    In,
    Out,
}

/// Fade-in and fade-out of one audible clip, multiplied with its volume curve.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Fades {
    pub fade_in: Fade,
    pub fade_out: Fade,
}
impl Fades {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.fade_in.duration < Tick::ZERO || self.fade_out.duration < Tick::ZERO {
            return Err(Error::Invalid("fade duration"));
        }
        Ok(())
    }

    /// Set one edge, clamping to the clip duration minus the other edge so the two never
    /// overlap. Returns the stored duration. A negative request is an error.
    pub fn set(
        &mut self,
        edge: FadeEdge,
        duration: Tick,
        curve: FadeCurve,
        clip: Tick,
    ) -> Result<Tick> {
        if duration < Tick::ZERO {
            return Err(Error::Invalid("fade duration"));
        }
        let other = match edge {
            FadeEdge::In => self.fade_out.duration,
            FadeEdge::Out => self.fade_in.duration,
        };
        let room = clip.saturating_sub(other.min(clip)).max(Tick::ZERO);
        let fade = Fade {
            duration: duration.min(room),
            curve,
        };
        match edge {
            FadeEdge::In => self.fade_in = fade,
            FadeEdge::Out => self.fade_out = fade,
        }
        Ok(fade.duration)
    }

    /// Resolve fades for a clip of `len` output samples at `rate` Hz, converting ticks once
    /// with nearest rounding. The fade-in wins any overlap left by a trim, so
    /// `fade_in + fade_out <= len` always holds.
    pub fn spans(&self, len: i64, rate: i64) -> Result<FadeSpans> {
        let samples = |fade: &Fade| -> Result<i64> {
            Ok(fade
                .duration
                .max(Tick::ZERO)
                .to_units(rate, Rounding::Nearest)?
                .clamp(0, len.max(0)))
        };
        let fade_in = samples(&self.fade_in)?;
        let fade_out = samples(&self.fade_out)?.min(len.max(0) - fade_in);
        Ok(FadeSpans {
            len,
            fade_in,
            fade_out,
            in_curve: self.fade_in.curve,
            out_curve: self.fade_out.curve,
        })
    }
}

/// Fades resolved to sample counts for one clip.
#[derive(Clone, Copy, Debug)]
pub struct FadeSpans {
    len: i64,
    fade_in: i64,
    fade_out: i64,
    in_curve: FadeCurve,
    out_curve: FadeCurve,
}
impl FadeSpans {
    /// Fade lengths in samples, `(in, out)`.
    pub fn lengths(&self) -> (i64, i64) {
        (self.fade_in, self.fade_out)
    }

    /// Gain of clip sample `sample` (0-based). The first and last clip samples are exactly
    /// silent under a fade; the body is exactly 1.
    pub fn gain(&self, sample: i64) -> f32 {
        let mut gain = 1.0;
        if sample < self.fade_in {
            gain = self.in_curve.gain(sample as f64 / self.fade_in as f64);
        }
        let from_end = self.len - 1 - sample;
        if from_end < self.fade_out {
            gain *= self.out_curve.gain(from_end as f64 / self.fade_out as f64);
        }
        gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [FadeCurve; 3] = [
        FadeCurve::EqualPower,
        FadeCurve::Linear,
        FadeCurve::Exponential,
    ];

    #[test]
    fn endpoints_are_exact_and_midpoint_is_equal_power() {
        for curve in CURVES {
            assert_eq!(curve.gain(0.0), 0.0);
            assert_eq!(curve.gain(1.0), 1.0);
            assert_eq!(curve.gain(-1.0), 0.0);
            assert_eq!(curve.gain(2.0), 1.0);
            assert_eq!(curve.gain(f64::NAN), 0.0);
        }
        assert!((FadeCurve::EqualPower.gain(0.5) - 0.5f32.sqrt()).abs() < 1e-6);
        assert_eq!(FadeCurve::Linear.gain(0.5), 0.5);
        assert!(FadeCurve::Exponential.gain(0.5) < 0.2);
        // Equal power: a fade-in and its mirror carry constant power.
        for i in 0..=100 {
            let x = f64::from(i) / 100.0;
            let a = f64::from(FadeCurve::EqualPower.gain(x));
            let b = f64::from(FadeCurve::EqualPower.gain(1.0 - x));
            assert!((a * a + b * b - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn curves_are_monotonic_and_bounded() {
        for curve in CURVES {
            let mut last = 0.0;
            for i in 0..=1000 {
                let g = curve.gain(f64::from(i) / 1000.0);
                assert!((0.0..=1.0).contains(&g) && g >= last);
                last = g;
            }
        }
    }

    #[test]
    fn fades_never_exceed_clip_bounds() {
        let rate = 48_000;
        let second = Tick(time::TICKS_PER_SECOND);
        for len in [0i64, 1, 2, 3, 100, 48_000] {
            for a in [0i64, 1, 50, 100, 1_000_000] {
                for b in [0i64, 1, 50, 100, 1_000_000] {
                    let fades = Fades {
                        fade_in: Fade {
                            duration: Tick(second.0 / rate * a),
                            curve: FadeCurve::Linear,
                        },
                        fade_out: Fade {
                            duration: Tick(second.0 / rate * b),
                            curve: FadeCurve::EqualPower,
                        },
                    };
                    let spans = fades.spans(len, rate).unwrap();
                    let (i, o) = spans.lengths();
                    assert!(i >= 0 && o >= 0 && i + o <= len, "{len} {a} {b}");
                    for s in 0..len {
                        assert!((0.0..=1.0).contains(&spans.gain(s)));
                    }
                }
            }
        }
    }

    #[test]
    fn clip_edges_are_silent_and_body_is_unity() {
        let rate = 1000;
        let ms = |n: i64| Tick(time::TICKS_PER_SECOND / 1000 * n);
        let fades = Fades {
            fade_in: Fade {
                duration: ms(10),
                curve: FadeCurve::EqualPower,
            },
            fade_out: Fade {
                duration: ms(20),
                curve: FadeCurve::Exponential,
            },
        };
        let spans = fades.spans(100, rate).unwrap();
        assert_eq!(spans.lengths(), (10, 20));
        assert_eq!(spans.gain(0), 0.0);
        assert_eq!(spans.gain(99), 0.0);
        assert_eq!(spans.gain(10), 1.0);
        assert_eq!(spans.gain(79), 1.0);
        assert!(spans.gain(80) < 1.0);
        let none = Fades::default().spans(100, rate).unwrap();
        assert!((0..100).all(|s| none.gain(s) == 1.0));
    }

    #[test]
    fn set_clamps_to_clip_and_other_edge() {
        let ms = |n: i64| Tick(time::TICKS_PER_SECOND / 1000 * n);
        let mut fades = Fades::default();
        assert_eq!(
            fades
                .set(FadeEdge::In, ms(700), FadeCurve::Linear, ms(1000))
                .unwrap(),
            ms(700)
        );
        assert_eq!(
            fades
                .set(FadeEdge::Out, ms(700), FadeCurve::Linear, ms(1000))
                .unwrap(),
            ms(300)
        );
        assert_eq!(
            fades
                .set(FadeEdge::In, ms(5000), FadeCurve::Linear, ms(1000))
                .unwrap(),
            ms(700)
        );
        assert!(
            fades
                .set(FadeEdge::In, Tick(-1), FadeCurve::Linear, ms(1000))
                .is_err()
        );
        // Tick::MAX request never overflows.
        fades
            .set(FadeEdge::Out, Tick(i64::MAX), FadeCurve::Linear, ms(1000))
            .unwrap();
        fades.spans(48_000, 48_000).unwrap();
    }
}
