//! Runs one item's cleanup chain over its source audio, keeping filter state across contiguous blocks.
mod biquad;

use biquad::{Biquad, Coeffs, FilterType};
use project::{AudioCleanup, CleanupKind, EqBandKind};
use std::f64::consts::{FRAC_1_SQRT_2, SQRT_2};

/// Hum harmonics at or above this fraction of the sample rate are left out.
const HUM_NYQUIST_FRACTION: f64 = 0.45;
const BASS_SHELF_HZ: f64 = 200.0;
const TREBLE_SHELF_HZ: f64 = 4000.0;
/// Octave-wide constant-Q bands: `sqrt(2^1) / (2^1 - 1)`.
const GRAPHIC_EQ_Q: f64 = SQRT_2;

/// One effect of the chain: its sections in series and its dry/wet mix.
struct Stage {
    sections: Vec<Coeffs>,
    /// 0 keeps the input, 1 keeps the processed signal.
    wet: f64,
    /// `state[channel][section]`.
    state: Vec<Vec<Biquad>>,
}

/// The cleanup chain of one item. Each block is a causal function of the previous samples,
/// so any partition of the same item into blocks gives identical audio. A block that does
/// not continue the previous one restarts the state.
pub(crate) struct CleanupChain {
    stages: Vec<Stage>,
    /// Absolute output sample expected next.
    next: Option<i64>,
}

impl CleanupChain {
    /// Builds the chain for the enabled effects with a nonzero mix. An all-bypassed chain
    /// has no stages and leaves the source untouched.
    pub(crate) fn new(cleanup: &AudioCleanup, rate: u32) -> Self {
        let sample_rate = f64::from(rate);
        let stages = cleanup
            .effects
            .iter()
            .filter(|effect| effect.enabled && effect.wet_percent > 0.0)
            .map(|effect| Stage {
                sections: sections(&effect.kind, sample_rate),
                wet: f64::from(effect.wet_percent) / 100.0,
                state: Vec::new(),
            })
            .collect();
        Self { stages, next: None }
    }

    /// Filters `planes` in place. `start` is the absolute output sample of the first frame.
    pub(crate) fn process_at(&mut self, start: i64, planes: &mut [Vec<f32>]) {
        let frames = planes.first().map_or(0, Vec::len);
        if self.next != Some(start) {
            self.reset(planes.len());
        }
        for stage in &mut self.stages {
            if stage.state.len() != planes.len() {
                stage.state = vec![vec![Biquad::default(); stage.sections.len()]; planes.len()];
            }
            for (channel, plane) in planes.iter_mut().enumerate() {
                let Some(states) = stage.state.get_mut(channel) else {
                    continue;
                };
                for sample in plane.iter_mut() {
                    let dry = f64::from(*sample);
                    let mut x = dry;
                    for (section, bq) in stage.sections.iter().zip(states.iter_mut()) {
                        x = bq.tick(section, x);
                    }
                    *sample = (dry + stage.wet * (x - dry)) as f32;
                }
            }
        }
        self.next = Some(start + frames as i64);
    }

    fn reset(&mut self, channels: usize) {
        for stage in &mut self.stages {
            stage.state = vec![vec![Biquad::default(); stage.sections.len()]; channels];
        }
    }
}

fn filter_type(kind: EqBandKind) -> FilterType {
    match kind {
        EqBandKind::Peaking => FilterType::Peaking,
        EqBandKind::LowShelf => FilterType::LowShelf,
        EqBandKind::HighShelf => FilterType::HighShelf,
        EqBandKind::Notch => FilterType::Notch,
    }
}

fn sections(kind: &CleanupKind, sample_rate: f64) -> Vec<Coeffs> {
    match kind {
        CleanupKind::Hum {
            mains,
            harmonics,
            q,
        } => {
            let fundamental = f64::from(mains.hertz());
            (1..=usize::from(*harmonics))
                .map(|k| fundamental * k as f64)
                .filter(|freq| *freq < sample_rate * HUM_NYQUIST_FRACTION)
                .map(|freq| {
                    Coeffs::design(FilterType::Notch, sample_rate, freq, f64::from(*q), 0.0)
                })
                .collect()
        }
        CleanupKind::Highpass { cutoff_hz } => vec![Coeffs::design(
            FilterType::HighPass,
            sample_rate,
            f64::from(*cutoff_hz),
            FRAC_1_SQRT_2,
            0.0,
        )],
        CleanupKind::Lowpass { cutoff_hz } => vec![Coeffs::design(
            FilterType::LowPass,
            sample_rate,
            f64::from(*cutoff_hz),
            FRAC_1_SQRT_2,
            0.0,
        )],
        CleanupKind::Bass { boost_db } => {
            shelf(FilterType::LowShelf, BASS_SHELF_HZ, *boost_db, sample_rate)
        }
        CleanupKind::Treble { boost_db } => shelf(
            FilterType::HighShelf,
            TREBLE_SHELF_HZ,
            *boost_db,
            sample_rate,
        ),
        CleanupKind::GraphicEq { gains_db } => gains_db
            .iter()
            .zip(project::GRAPHIC_EQ_FREQUENCIES_HZ)
            .filter(|(gain, _)| **gain != 0.0)
            .map(|(gain, freq)| {
                Coeffs::design(
                    FilterType::Peaking,
                    sample_rate,
                    f64::from(freq),
                    GRAPHIC_EQ_Q,
                    f64::from(*gain),
                )
            })
            .collect(),
        CleanupKind::ParametricEq { bands } => bands
            .iter()
            .filter(|band| band.enabled && (band.gain_db != 0.0 || band.kind == EqBandKind::Notch))
            .map(|band| {
                Coeffs::design(
                    filter_type(band.kind),
                    sample_rate,
                    f64::from(band.frequency_hz),
                    f64::from(band.q),
                    f64::from(band.gain_db),
                )
            })
            .collect(),
    }
}

fn shelf(kind: FilterType, freq: f64, boost_db: f32, sample_rate: f64) -> Vec<Coeffs> {
    if boost_db == 0.0 {
        return Vec::new();
    }
    vec![Coeffs::design(
        kind,
        sample_rate,
        freq,
        FRAC_1_SQRT_2,
        f64::from(boost_db),
    )]
}
