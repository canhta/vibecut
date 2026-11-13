//! De-esser: a dynamic high shelf that cuts the sibilance band while it stands out.
//!
//! A second-order high-pass at the band frequency is the detector. The band's level is compared
//! with the broadband level, both smoothed power sums over all channels, so the cut is
//! stereo-linked. When the band exceeds the broadband level by more than the threshold, the
//! excess (doubled, capped at the reduction) becomes the target gain reduction, smoothed with a
//! 1 ms attack and a 60 ms release, and drives an RBJ high shelf at the band frequency. At zero
//! reduction the shelf is an identity, so passages without sibilance pass unchanged. A subtractive
//! split `x - HP(x)` is not used: the high-pass phase shift near the crossover would boost there
//! instead of cutting. The detector, attack and release are adapted from the structure
//! reference's `audio-dsp` DeEsser, the shelf is the RBJ cookbook high shelf. The stage has no
//! delay.

use super::biquad::{Biquad, Coeffs, FilterType};
use std::f64::consts::FRAC_1_SQRT_2;

/// Smoothing of the band and broadband power detectors in milliseconds.
const DETECTOR_MS: f64 = 2.0;
const ATTACK_MS: f64 = 1.0;
const RELEASE_MS: f64 = 60.0;
/// The shelf is redesigned when the gain reduction moved by more than this many dB.
const REDESIGN_DB: f64 = 0.05;
/// A band quieter than this is not sibilance, whatever its ratio to the rest.
const BAND_FLOOR_DB: f64 = -70.0;
/// Upper bound in bytes of the state of one de-esser over two channels.
pub(super) const STATE_BYTES: u64 = 4096;

/// One-pole smoothing coefficient for a time constant.
fn pole(ms: f64, rate: f64) -> f64 {
    (-1.0 / (ms * 0.001 * rate)).exp()
}

fn power_db(power: f64) -> f64 {
    10.0 * (power + 1e-20).log10()
}

fn flush(x: f64) -> f64 {
    if x.abs() < 1e-30 { 0.0 } else { x }
}

/// De-esser of one stage over any number of channels.
pub(super) struct DeEsser {
    rate: f64,
    frequency_hz: f64,
    threshold_db: f64,
    reduction_db: f64,
    detector: Coeffs,
    shelf: Coeffs,
    /// Gain reduction the shelf was designed for, in dB (at most 0).
    shelf_db: f64,
    detect_pole: f64,
    attack: f64,
    release: f64,
    band_power: f64,
    full_power: f64,
    /// Smoothed gain reduction in dB (at most 0).
    reduction: f64,
    high_pass: Vec<Biquad>,
    shelf_state: Vec<Biquad>,
}

impl DeEsser {
    pub(super) fn new(rate: u32, frequency_hz: f32, threshold_db: f32, reduction_db: f32) -> Self {
        let rate = f64::from(rate);
        let frequency_hz = f64::from(frequency_hz);
        Self {
            rate,
            frequency_hz,
            threshold_db: f64::from(threshold_db),
            reduction_db: f64::from(reduction_db),
            detector: Coeffs::design(FilterType::HighPass, rate, frequency_hz, FRAC_1_SQRT_2, 0.0),
            shelf: Self::shelf_for(rate, frequency_hz, 0.0),
            shelf_db: 0.0,
            detect_pole: pole(DETECTOR_MS, rate),
            attack: pole(ATTACK_MS, rate),
            release: pole(RELEASE_MS, rate),
            band_power: 0.0,
            full_power: 0.0,
            reduction: 0.0,
            high_pass: Vec::new(),
            shelf_state: Vec::new(),
        }
    }

    fn shelf_for(rate: f64, frequency_hz: f64, gain_db: f64) -> Coeffs {
        Coeffs::design(
            FilterType::HighShelf,
            rate,
            frequency_hz,
            FRAC_1_SQRT_2,
            gain_db,
        )
    }

    /// Forgets all signal history.
    pub(super) fn reset(&mut self) {
        self.high_pass.clear();
        self.shelf_state.clear();
        self.band_power = 0.0;
        self.full_power = 0.0;
        self.reduction = 0.0;
        self.shelf_db = 0.0;
        self.shelf = Self::shelf_for(self.rate, self.frequency_hz, 0.0);
    }

    /// Cuts the sibilance of `planes` in place; every output sample is the shelf's response to
    /// the input sample at the same position.
    pub(super) fn process(&mut self, planes: &mut [Vec<f32>]) {
        if self.high_pass.len() != planes.len() {
            self.high_pass = vec![Biquad::default(); planes.len()];
            self.shelf_state = vec![Biquad::default(); planes.len()];
        }
        let frames = planes.first().map_or(0, Vec::len);
        for index in 0..frames {
            let (mut band, mut full) = (0.0f64, 0.0f64);
            for (plane, high_pass) in planes.iter().zip(self.high_pass.iter_mut()) {
                let x = f64::from(plane.get(index).copied().unwrap_or(0.0));
                let b = high_pass.tick(&self.detector, x);
                band += b * b;
                full += x * x;
            }
            self.band_power =
                flush(self.detect_pole * self.band_power + (1.0 - self.detect_pole) * band);
            self.full_power =
                flush(self.detect_pole * self.full_power + (1.0 - self.detect_pole) * full);
            let band_db = power_db(self.band_power);
            let excess = band_db - power_db(self.full_power) - self.threshold_db;
            let target = if band_db > BAND_FLOOR_DB && excess > 0.0 {
                -(excess * 2.0).min(self.reduction_db)
            } else {
                0.0
            };
            let speed = if target < self.reduction {
                self.attack
            } else {
                self.release
            };
            self.reduction = flush(speed * self.reduction + (1.0 - speed) * target);
            if target == 0.0 && self.reduction > -1e-4 {
                self.reduction = 0.0;
            }
            if (self.reduction - self.shelf_db).abs() > REDESIGN_DB
                || (self.reduction == 0.0) != (self.shelf_db == 0.0)
            {
                self.shelf = Self::shelf_for(self.rate, self.frequency_hz, self.reduction);
                self.shelf_db = self.reduction;
            }
            for (plane, state) in planes.iter_mut().zip(self.shelf_state.iter_mut()) {
                if let Some(sample) = plane.get_mut(index) {
                    *sample = state.tick(&self.shelf, f64::from(*sample)) as f32;
                }
            }
        }
    }
}
