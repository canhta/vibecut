//! Second-order IIR sections from the RBJ "Audio EQ Cookbook", run in transposed direct form II.

use std::f64::consts::PI;

/// Filter shapes the cleanup chain designs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilterType {
    Peaking,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    Notch,
}

/// Normalised coefficients with `a0 == 1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coeffs {
    fn normalised(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Designs a section. `freq` is clamped below Nyquist, `q` is positive, and `gain_db`
    /// is used by the peaking and shelving shapes only (shelves take `q` as the shelf Q).
    pub(crate) fn design(
        kind: FilterType,
        sample_rate: f64,
        freq: f64,
        q: f64,
        gain_db: f64,
    ) -> Self {
        let f = freq.clamp(1.0, sample_rate * 0.49);
        let q = q.max(1e-3);
        let w0 = 2.0 * PI * f / sample_rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        match kind {
            FilterType::LowPass => Self::normalised(
                (1.0 - cs) / 2.0,
                1.0 - cs,
                (1.0 - cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            FilterType::HighPass => Self::normalised(
                (1.0 + cs) / 2.0,
                -(1.0 + cs),
                (1.0 + cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            FilterType::Notch => {
                Self::normalised(1.0, -2.0 * cs, 1.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
            }
            FilterType::Peaking => Self::normalised(
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            FilterType::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                Self::normalised(
                    a * ((a + 1.0) - (a - 1.0) * cs + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - k),
                    (a + 1.0) + (a - 1.0) * cs + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - k,
                )
            }
            FilterType::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                Self::normalised(
                    a * ((a + 1.0) + (a - 1.0) * cs + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - k),
                    (a + 1.0) - (a - 1.0) * cs + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - k,
                )
            }
        }
    }
}

/// One section's state. Values that decay below 1e-30 are flushed to zero so denormals never appear.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Biquad {
    s1: f64,
    s2: f64,
}

impl Biquad {
    pub(crate) fn tick(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = flush(c.b1 * x - c.a1 * y + self.s2);
        self.s2 = flush(c.b2 * x - c.a2 * y);
        y
    }
}

fn flush(x: f64) -> f64 {
    if (1e-30..=f64::MAX).contains(&x.abs()) {
        x
    } else {
        0.0
    }
}
