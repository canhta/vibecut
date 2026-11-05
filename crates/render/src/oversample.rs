//! Polyphase FIR interpolator used for true-peak estimation (BS.1770-4 Annex 2 style).
//!
//! The prototype is a Kaiser-windowed sinc low-pass (cut-off at the original Nyquist) split into
//! `factor` phases of [`TAPS_PER_PHASE`] taps; each phase is normalised to unity DC gain.

use std::f64::consts::PI;

/// Taps per polyphase branch.
pub const TAPS_PER_PHASE: usize = 16;

/// Oversampling factor giving an effective rate of at least 176.4 kHz (4× at 44.1/48 kHz,
/// 2× at 88.2/96 kHz, 1× above), as recommended by BS.1770-4 for true-peak metering.
pub fn true_peak_factor(sample_rate: f64) -> usize {
    if sample_rate < 88_200.0 {
        4
    } else if sample_rate < 176_400.0 {
        2
    } else {
        1
    }
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..50 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Polyphase coefficient bank shared by all channels.
#[derive(Clone, Debug)]
pub struct PolyphaseBank {
    factor: usize,
    /// `coeffs[p * TAPS_PER_PHASE + j]` multiplies `x[n - j]` for output phase `p`.
    coeffs: Vec<f32>,
}

impl PolyphaseBank {
    pub fn new(factor: usize) -> Self {
        let factor = factor.max(1);
        let len = factor * TAPS_PER_PHASE;
        let centre = (len - 1) as f64 / 2.0;
        let beta = 8.0;
        let i0b = bessel_i0(beta);
        let proto: Vec<f64> = (0..len)
            .map(|m| {
                let t = (m as f64 - centre) / factor as f64;
                let sinc = if t.abs() < 1e-12 {
                    1.0
                } else {
                    (PI * t).sin() / (PI * t)
                };
                let r = (m as f64 - centre) / (centre + 1.0);
                let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b;
                sinc * w
            })
            .collect();
        let mut coeffs = vec![0.0f32; len];
        for p in 0..factor {
            let sum: f64 = (0..TAPS_PER_PHASE).map(|j| proto[p + factor * j]).sum();
            for j in 0..TAPS_PER_PHASE {
                coeffs[p * TAPS_PER_PHASE + j] = (proto[p + factor * j] / sum) as f32;
            }
        }
        PolyphaseBank { factor, coeffs }
    }
    /// Approximate delay (in input samples) between an input sample and the interpolated
    /// values around it.
    pub fn delay(&self) -> usize {
        TAPS_PER_PHASE / 2
    }
}

/// Per-channel interpolator history.
#[derive(Clone, Debug)]
pub struct Interpolator {
    /// Double-written ring so a contiguous window of TAPS_PER_PHASE is always available.
    hist: [f32; 2 * TAPS_PER_PHASE],
    pos: usize,
}

impl Default for Interpolator {
    fn default() -> Self {
        Interpolator {
            hist: [0.0; 2 * TAPS_PER_PHASE],
            pos: 0,
        }
    }
}

impl Interpolator {
    /// Push one input sample; returns the maximum absolute value of the `factor` interpolated
    /// output samples produced for it.
    #[inline]
    pub fn push_peak(&mut self, bank: &PolyphaseBank, x: f32) -> f32 {
        if bank.factor == 1 {
            return x.abs();
        }
        // Newest sample at window[TAPS-1-0]... store so that window[j] = x[n - j].
        self.pos = if self.pos == 0 {
            TAPS_PER_PHASE - 1
        } else {
            self.pos - 1
        };
        self.hist[self.pos] = x;
        self.hist[self.pos + TAPS_PER_PHASE] = x;
        let win = &self.hist[self.pos..self.pos + TAPS_PER_PHASE];
        let mut peak = 0.0f32;
        for c in bank.coeffs.as_chunks::<TAPS_PER_PHASE>().0 {
            peak = peak.max(dot(c, win).abs());
        }
        peak
    }
}

/// Dot product of one phase with the window. Eight partial sums (instead of one sequential sum)
/// let the compiler vectorise it; the result differs from the sequential sum only by rounding.
#[inline(always)]
fn dot(c: &[f32; TAPS_PER_PHASE], win: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    for (cc, ww) in c.as_chunks::<8>().0.iter().zip(win.as_chunks::<8>().0) {
        for ((a, &ci), &wi) in acc.iter_mut().zip(cc).zip(ww) {
            *a += ci * wi;
        }
    }
    (acc[0] + acc[4]) + (acc[1] + acc[5]) + (acc[2] + acc[6]) + (acc[3] + acc[7])
}
