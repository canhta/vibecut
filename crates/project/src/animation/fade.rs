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
///
/// A split keeps the fade's whole shape and records how much of it the other piece took, so
/// the pieces together replay the original envelope. Only the part `offset..duration` is
/// audible on this clip, and that window (not `duration`) is what the clip length clamps.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Fade {
    /// Full fade length in ticks, measured from the edge it was set on.
    pub duration: Tick,
    /// Ticks of the fade cut away beyond this clip's edge by a split; `0..=duration`.
    #[serde(default)]
    pub offset: Tick,
    pub curve: FadeCurve,
}
impl Fade {
    /// Ticks of the fade left on this clip, before clamping to the clip length.
    pub fn window(&self) -> Tick {
        self.duration.saturating_sub(self.offset).max(Tick::ZERO)
    }

    fn kept(self, fade: Self) -> Self {
        if fade.window() > Tick::ZERO {
            fade
        } else {
            Self::default()
        }
    }
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
/// Longest fade one edge can hold, whatever the clip length (the reference editor caps both
/// fades at 10.0 s). `Fades::set` stores `min(request, MAX_FADE, room)`, where room is the clip
/// minus the other edge's window.
pub const MAX_FADE: Tick = Tick(10 * time::TICKS_PER_SECOND);

impl Fades {
    pub(crate) fn validate(&self) -> Result<()> {
        for fade in [&self.fade_in, &self.fade_out] {
            if fade.duration < Tick::ZERO {
                return Err(Error::Invalid("fade duration"));
            }
            if fade.offset < Tick::ZERO || fade.offset > fade.duration {
                return Err(Error::Invalid("fade offset"));
            }
        }
        Ok(())
    }

    /// Fades of the two pieces of the clip `start..end` cut at `at`. Each piece keeps the
    /// whole shape of any fade reaching it and records the part the other piece holds, so
    /// the pieces replay the original envelope at every sample.
    pub fn split(&self, start: Tick, at: Tick, end: Tick) -> Result<(Self, Self)> {
        let grow =
            |offset: Tick, by: Tick| offset.checked_add(by).ok_or(Error::Invalid("fade offset"));
        let before = at.checked_sub(start).ok_or(Error::Invalid("fade offset"))?;
        let after = end.checked_sub(at).ok_or(Error::Invalid("fade offset"))?;
        let (fade_in, fade_out) = (self.fade_in, self.fade_out);
        let left_out = Fade {
            offset: grow(fade_out.offset, after)?,
            ..fade_out
        };
        let right_in = Fade {
            offset: grow(fade_in.offset, before)?,
            ..fade_in
        };
        Ok((
            Self {
                fade_in,
                fade_out: left_out.kept(left_out),
            },
            Self {
                fade_in: right_in.kept(right_in),
                fade_out,
            },
        ))
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
            FadeEdge::In => self.fade_out.window(),
            FadeEdge::Out => self.fade_in.window(),
        };
        let room = clip.saturating_sub(other.min(clip)).max(Tick::ZERO);
        let fade = Fade {
            duration: duration.min(MAX_FADE).min(room),
            offset: Tick::ZERO,
            curve,
        };
        match edge {
            FadeEdge::In => self.fade_in = fade,
            FadeEdge::Out => self.fade_out = fade,
        }
        Ok(fade.duration)
    }

    /// Resolve fades for the clip `start..end` at `rate` Hz. Ticks convert to samples once, by
    /// absolute position with nearest rounding, so a piece cut from a longer clip lands on
    /// exactly the samples the whole clip had. Each fade's window is clamped to the clip;
    /// overlapping windows simply multiply.
    pub fn spans(&self, start: Tick, end: Tick, rate: i64) -> Result<FadeSpans> {
        let at = |tick: Tick| tick.to_units(rate, Rounding::Nearest);
        let (i0, i1) = (at(start)?, at(end)?);
        let len = (i1 - i0).max(0);
        let edge = |fade: &Fade, origin: Tick, limit: Tick, shift: i64| -> Result<Resolved> {
            // Round both absolute endpoints, just as the clip and split positions are rounded.
            // Rounding a duration alone can retain a sample beyond its nominal window, which
            // would disappear when a split clears the fade at that window's boundary.
            let size = at(limit)? - at(origin)?;
            Ok(Resolved {
                size,
                shift,
                window: (size - shift).clamp(0, len),
                curve: fade.curve,
            })
        };
        let behind = |fade: &Fade| {
            fade.offset
                .max(Tick::ZERO)
                .min(fade.duration.max(Tick::ZERO))
        };
        let in_origin = start
            .checked_sub(behind(&self.fade_in))
            .ok_or(Error::Invalid("fade offset"))?;
        let out_origin = end
            .checked_add(behind(&self.fade_out))
            .ok_or(Error::Invalid("fade offset"))?;
        let in_limit = in_origin
            .checked_add(self.fade_in.duration)
            .ok_or(Error::Invalid("fade endpoint"))?;
        let out_start = out_origin
            .checked_sub(self.fade_out.duration)
            .ok_or(Error::Invalid("fade endpoint"))?;
        Ok(FadeSpans {
            len,
            fade_in: edge(&self.fade_in, in_origin, in_limit, i0 - at(in_origin)?)?,
            fade_out: edge(&self.fade_out, out_start, out_origin, at(out_origin)? - i1)?,
        })
    }
}

/// One fade edge resolved to samples.
#[derive(Clone, Copy, Debug)]
struct Resolved {
    /// Whole fade length.
    size: i64,
    /// Samples of the whole fade lying beyond this clip's edge.
    shift: i64,
    /// Samples audible on this clip.
    window: i64,
    curve: FadeCurve,
}
impl Resolved {
    /// Gain `distance` samples inside the clip edge, 1 past the window.
    fn gain(&self, distance: i64) -> f32 {
        if distance < self.window {
            self.curve
                .gain((distance + self.shift) as f64 / self.size as f64)
        } else {
            1.0
        }
    }
}

/// Fades resolved to sample counts for one clip.
#[derive(Clone, Copy, Debug)]
pub struct FadeSpans {
    len: i64,
    fade_in: Resolved,
    fade_out: Resolved,
}
impl FadeSpans {
    /// Audible fade lengths in samples, `(in, out)`.
    pub fn lengths(&self) -> (i64, i64) {
        (self.fade_in.window, self.fade_out.window)
    }

    /// Gain of clip sample `sample` (0-based). A fade that starts on this clip is exactly
    /// silent at its edge; the body is exactly 1.
    pub fn gain(&self, sample: i64) -> f32 {
        self.fade_in.gain(sample) * self.fade_out.gain(self.len - 1 - sample)
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

    /// Duration for `n` samples at `rate` (a whole number of ticks per sample).
    fn samples(n: i64, rate: i64) -> Tick {
        Tick(time::TICKS_PER_SECOND / rate * n)
    }

    #[test]
    fn fades_never_exceed_clip_bounds() {
        let rate = 48_000;
        for len in [0i64, 1, 2, 3, 100, 48_000] {
            for a in [0i64, 1, 50, 100, 1_000_000] {
                for b in [0i64, 1, 50, 100, 1_000_000] {
                    let fades = Fades {
                        fade_in: Fade {
                            duration: samples(a, rate),
                            curve: FadeCurve::Linear,
                            ..Fade::default()
                        },
                        fade_out: Fade {
                            duration: samples(b, rate),
                            curve: FadeCurve::EqualPower,
                            ..Fade::default()
                        },
                    };
                    let spans = fades.spans(Tick::ZERO, samples(len, rate), rate).unwrap();
                    let (i, o) = spans.lengths();
                    assert!(
                        (0..=len).contains(&i) && (0..=len).contains(&o),
                        "{len} {a} {b}"
                    );
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
        let fades = Fades {
            fade_in: Fade {
                duration: samples(10, rate),
                curve: FadeCurve::EqualPower,
                ..Fade::default()
            },
            fade_out: Fade {
                duration: samples(20, rate),
                curve: FadeCurve::Exponential,
                ..Fade::default()
            },
        };
        let spans = fades.spans(Tick::ZERO, samples(100, rate), rate).unwrap();
        assert_eq!(spans.lengths(), (10, 20));
        assert_eq!(spans.gain(0), 0.0);
        assert_eq!(spans.gain(99), 0.0);
        assert_eq!(spans.gain(10), 1.0);
        assert_eq!(spans.gain(79), 1.0);
        assert!(spans.gain(80) < 1.0);
        let none = Fades::default()
            .spans(Tick::ZERO, samples(100, rate), rate)
            .unwrap();
        assert!((0..100).all(|s| none.gain(s) == 1.0));
        // Stored overlapping windows multiply; command-created fades are clamped separately.
        let overlapping = Fades {
            fade_in: Fade {
                duration: samples(80, rate),
                curve: FadeCurve::Linear,
                ..Fade::default()
            },
            fade_out: Fade {
                duration: samples(80, rate),
                curve: FadeCurve::Linear,
                ..Fade::default()
            },
        }
        .spans(Tick::ZERO, samples(100, rate), rate)
        .unwrap();
        assert_eq!(overlapping.lengths(), (80, 80));
        assert!((overlapping.gain(50) - 0.3828125).abs() < 1e-7);
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
        fades
            .spans(Tick::ZERO, Tick(time::TICKS_PER_SECOND), 48_000)
            .unwrap();
    }

    #[test]
    fn set_clamps_against_the_audible_window_of_a_cut_fade() {
        let mut fades = Fades {
            fade_in: Fade {
                duration: Tick(300),
                offset: Tick(200),
                ..Fade::default()
            },
            ..Fades::default()
        };
        // Only 100 ticks of the fade-in remain, so the fade-out may take the other 400.
        let stored = fades
            .set(FadeEdge::Out, Tick(1000), FadeCurve::Linear, Tick(500))
            .unwrap();
        assert_eq!(stored, Tick(400));
    }

    #[test]
    fn offset_is_validated() {
        for (duration, offset) in [(10, -1), (10, 11)] {
            let fades = Fades {
                fade_in: Fade {
                    duration: Tick(duration),
                    offset: Tick(offset),
                    ..Fade::default()
                },
                ..Fades::default()
            };
            assert!(fades.validate().is_err(), "{duration} {offset}");
        }
    }

    /// Absolute-sample gains of the clip `start..end`.
    fn envelope(fades: &Fades, start: Tick, end: Tick, rate: i64) -> Vec<(i64, f32)> {
        let spans = fades.spans(start, end, rate).unwrap();
        let first = start.to_units(rate, Rounding::Nearest).unwrap();
        let last = end.to_units(rate, Rounding::Nearest).unwrap();
        (first..last).map(|n| (n, spans.gain(n - first))).collect()
    }

    #[test]
    fn split_pieces_replay_the_original_envelope_at_every_sample() {
        // Durations, clip edges and cuts deliberately off the sample grid.
        let ticks = |ms: i64| Tick(time::TICKS_PER_SECOND / 1000 * ms + 12_345);
        for rate in [44_100, 48_000] {
            for (a, b) in CURVES.into_iter().zip(CURVES.into_iter().cycle().skip(1)) {
                let start = Tick(777_777);
                let end = Tick(777_777 + ticks(50).0);
                let fades = Fades {
                    fade_in: Fade {
                        duration: ticks(15),
                        curve: a,
                        ..Fade::default()
                    },
                    fade_out: Fade {
                        duration: ticks(20),
                        curve: b,
                        ..Fade::default()
                    },
                };
                let whole = envelope(&fades, start, end, rate);
                for cut_ms in [1, 7, 15, 25, 29, 30, 31, 45, 49] {
                    let at = Tick(start.0 + ticks(cut_ms).0 + 1);
                    let (left, right) = fades.split(start, at, end).unwrap();
                    let mut both = envelope(&left, start, at, rate);
                    both.extend(envelope(&right, at, end, rate));
                    assert_eq!(whole, both, "rate {rate} cut {cut_ms}");
                    // Cutting a piece again is still the same envelope.
                    let mid = Tick(at.0 + (end.0 - at.0) / 3);
                    let (r1, r2) = right.split(at, mid, end).unwrap();
                    let mut three = envelope(&left, start, at, rate);
                    three.extend(envelope(&r1, at, mid, rate));
                    three.extend(envelope(&r2, mid, end, rate));
                    assert_eq!(whole, three, "rate {rate} cut {cut_ms} twice");
                }
            }
        }

        // A cut at (or one tick either side of) an off-grid fade endpoint must not
        // clear a still-active rounded sample. Exercise all curve pairs.
        for rate in [44_100, 48_000] {
            let sample = time::TICKS_PER_SECOND / rate;
            let start = Tick(sample * 8 / 10);
            let end = Tick(start.0 + sample * 1000);
            for in_curve in CURVES {
                for out_curve in CURVES {
                    let fades = Fades {
                        fade_in: Fade {
                            duration: Tick(sample * 300 + sample * 6 / 10),
                            curve: in_curve,
                            ..Fade::default()
                        },
                        fade_out: Fade {
                            duration: Tick(sample * 400 + sample * 6 / 10),
                            curve: out_curve,
                            ..Fade::default()
                        },
                    };
                    let whole = envelope(&fades, start, end, rate);
                    for boundary in [
                        start.0 + fades.fade_in.duration.0,
                        end.0 - fades.fade_out.duration.0,
                    ] {
                        for delta in [-1, 0, 1] {
                            let at = Tick(boundary + delta);
                            let (left, right) = fades.split(start, at, end).unwrap();
                            let mut both = envelope(&left, start, at, rate);
                            both.extend(envelope(&right, at, end, rate));
                            assert_eq!(whole.len(), both.len());
                            for (original, split) in whole.iter().zip(&both) {
                                assert_eq!(
                                    original, split,
                                    "rate {rate}, curves {in_curve:?}/{out_curve:?}, cut {at:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_cut_on_a_fade_boundary_clears_the_empty_side() {
        let fades = Fades {
            fade_in: Fade {
                duration: Tick(300),
                ..Fade::default()
            },
            fade_out: Fade {
                duration: Tick(400),
                ..Fade::default()
            },
        };
        let (left, right) = fades.split(Tick::ZERO, Tick(300), Tick(1000)).unwrap();
        assert_eq!(left.fade_in, fades.fade_in);
        assert_eq!(left.fade_out, Fade::default());
        assert_eq!(right.fade_in, Fade::default());
        assert_eq!(right.fade_out, fades.fade_out);
        let (left, right) = fades.split(Tick::ZERO, Tick(600), Tick(1000)).unwrap();
        assert_eq!(left.fade_out, Fade::default());
        assert_eq!(right.fade_out, fades.fade_out);
        // One tick inside the fade-out the left piece keeps a one-tick tail.
        let (left, _) = fades.split(Tick::ZERO, Tick(601), Tick(1000)).unwrap();
        assert_eq!(left.fade_out.window(), Tick(1));
    }
}
