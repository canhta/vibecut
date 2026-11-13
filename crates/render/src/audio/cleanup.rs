//! Runs one item's cleanup chain over its source audio, keeping state across contiguous blocks.
mod biquad;
mod declick;
mod fft;
mod spectral;

use crate::{ExecutionHooks, MemoryKind, Reservation, Result};
use biquad::{Biquad, Coeffs, FilterType};
use declick::ClickRemover;
use project::{AudioCleanup, CleanupKind, EqBandKind};
use spectral::NoiseReducer;
use std::f64::consts::{FRAC_1_SQRT_2, SQRT_2};

/// Hum harmonics at or above this fraction of the sample rate are left out.
const HUM_NYQUIST_FRACTION: f64 = 0.45;
const BASS_SHELF_HZ: f64 = 200.0;
const TREBLE_SHELF_HZ: f64 = 4000.0;
/// Octave-wide constant-Q bands: `sqrt(2^1) / (2^1 - 1)`.
const GRAPHIC_EQ_Q: f64 = SQRT_2;

/// What one stage computes.
enum Engine {
    /// Zero-latency second-order sections in series; `state[channel][section]`.
    Filters {
        sections: Vec<Coeffs>,
        state: Vec<Vec<Biquad>>,
    },
    Noise(NoiseReducer),
    Clicks(ClickRemover),
}

impl Engine {
    /// Samples between an input sample entering the stage and its output leaving.
    fn latency(&self) -> usize {
        match self {
            Self::Filters { .. } => 0,
            Self::Noise(reducer) => reducer.latency(),
            Self::Clicks(_) => declick::LATENCY,
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Filters { state, .. } => state.clear(),
            Self::Noise(reducer) => reducer.reset(),
            Self::Clicks(remover) => remover.reset(),
        }
    }
}

/// One effect of the chain: its engine and its dry/wet mix.
struct Stage {
    engine: Engine,
    /// 0 keeps the input, 1 keeps the processed signal.
    wet: f64,
    /// The input held back by the engine's latency, per channel, so the mix adds the processed
    /// signal to the dry signal of the same instant.
    dry: Vec<Vec<f32>>,
    /// Write position of every channel's `dry` ring.
    dry_pos: usize,
}

impl Stage {
    fn new(engine: Engine, wet: f64) -> Self {
        Self {
            engine,
            wet,
            dry: Vec::new(),
            dry_pos: 0,
        }
    }

    fn reset(&mut self) {
        self.engine.reset();
        self.dry.clear();
        self.dry_pos = 0;
    }

    /// Processes `planes` in place. Each output sample is the engine's output for the input that
    /// entered `engine.latency()` samples earlier, mixed with that same delayed input.
    fn process(&mut self, planes: &mut [Vec<f32>]) {
        let latency = self.engine.latency();
        let wet = self.wet;
        if let Engine::Filters { sections, state } = &mut self.engine {
            if state.len() != planes.len() {
                *state = vec![vec![Biquad::default(); sections.len()]; planes.len()];
            }
            for (plane, states) in planes.iter_mut().zip(state.iter_mut()) {
                for sample in plane.iter_mut() {
                    let dry = f64::from(*sample);
                    let mut x = dry;
                    for (section, bq) in sections.iter().zip(states.iter_mut()) {
                        x = bq.tick(section, x);
                    }
                    *sample = (dry + wet * (x - dry)) as f32;
                }
            }
            return;
        }
        // Hold the dry input back by the engine's latency in a ring per channel.
        let ring_len = latency.max(1);
        if self.dry.len() != planes.len() {
            self.dry = vec![vec![0.0; ring_len]; planes.len()];
            self.dry_pos = 0;
        }
        let frames = planes.first().map_or(0, Vec::len);
        let start = self.dry_pos;
        let mut delayed = planes.to_vec();
        for (plane, ring) in delayed.iter_mut().zip(self.dry.iter_mut()) {
            let mut pos = start;
            for sample in plane.iter_mut() {
                if let Some(slot) = ring.get_mut(pos) {
                    std::mem::swap(slot, sample);
                }
                pos += 1;
                if pos == ring_len {
                    pos = 0;
                }
            }
        }
        self.dry_pos = (start + frames) % ring_len;
        match &mut self.engine {
            Engine::Noise(reducer) => reducer.process(planes),
            Engine::Clicks(remover) => remover.process(planes),
            Engine::Filters { .. } => {}
        }
        for (plane, held) in planes.iter_mut().zip(&delayed) {
            for (sample, dry) in plane.iter_mut().zip(held) {
                let dry = f64::from(*dry);
                *sample = (dry + wet * (f64::from(*sample) - dry)) as f32;
            }
        }
    }
}

/// The cleanup chain of one item. Each stage is a causal function of the samples before it, so
/// any partition of the same item into blocks gives identical audio. A stage that delays its
/// signal adds to the chain's `latency`: output sample `n` is the chain's response to input up to
/// sample `n + latency`, so the caller reads that far ahead. A block that does not continue the
/// previous one restarts every stage.
pub(crate) struct CleanupChain {
    stages: Vec<Stage>,
    latency: usize,
    channels: usize,
    /// Absolute source sample the chain takes next.
    fed: i64,
    /// Absolute sample of the next output, None before the first `restart`.
    out_next: Option<i64>,
    /// Leading outputs that only hold the delay and are dropped.
    skip: usize,
    _lease: Option<Box<dyn Reservation>>,
}

impl CleanupChain {
    /// Builds the chain for the enabled effects with a nonzero mix, holding the lease of its
    /// delay-line and spectral state. An all-bypassed chain has no stages and leaves the source
    /// untouched.
    ///
    /// # Errors
    ///
    /// Fails when the memory lease is refused.
    pub(crate) fn new(
        cleanup: &AudioCleanup,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        let sample_rate = f64::from(rate);
        let mut bytes = 0u64;
        let stages: Vec<Stage> = cleanup
            .effects
            .iter()
            .filter(|effect| effect.enabled && effect.wet_percent > 0.0)
            .map(|effect| {
                let engine = match &effect.kind {
                    CleanupKind::NoiseReduction {
                        reduction_db,
                        threshold_db,
                        smoothing_percent,
                    } => {
                        bytes += spectral::state_bytes(rate);
                        Engine::Noise(NoiseReducer::new(
                            rate,
                            *reduction_db,
                            *threshold_db,
                            *smoothing_percent,
                        ))
                    }
                    CleanupKind::Declick {
                        threshold,
                        complexity,
                    } => {
                        bytes += declick::STATE_BYTES;
                        Engine::Clicks(ClickRemover::new(*threshold, *complexity))
                    }
                    kind => Engine::Filters {
                        sections: sections(kind, sample_rate),
                        state: Vec::new(),
                    },
                };
                bytes += 8 * engine.latency() as u64;
                Stage::new(engine, f64::from(effect.wet_percent) / 100.0)
            })
            .collect();
        let lease = if bytes > 0 {
            Some(hooks.reserve(MemoryKind::Compositor, bytes)?)
        } else {
            None
        };
        let latency = stages.iter().map(|stage| stage.engine.latency()).sum();
        Ok(Self {
            stages,
            latency,
            channels: 0,
            fed: 0,
            out_next: None,
            skip: 0,
            _lease: lease,
        })
    }

    /// Samples the chain delays its signal by; zero when every stage is a filter.
    pub(crate) fn latency(&self) -> usize {
        self.latency
    }

    /// True when the next output sample is the absolute sample `start`.
    pub(crate) fn continues(&self, start: i64) -> bool {
        self.out_next == Some(start)
    }

    /// Absolute source sample the chain takes next.
    pub(crate) fn fed(&self) -> i64 {
        self.fed
    }

    /// Channels of the signal the chain last took, zero before the first.
    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    /// Drops all signal history. The next input sample is the absolute sample `start`; the first
    /// output, also for `start`, is ready once `latency` further samples have been taken.
    pub(crate) fn restart(&mut self, start: i64) {
        for stage in &mut self.stages {
            stage.reset();
        }
        self.fed = start;
        self.out_next = Some(start);
        self.skip = self.latency;
    }

    /// Takes `planes` as the next input samples and leaves in them the outputs that are ready:
    /// as many as were taken once the delay is paid, fewer in the first calls after a `restart`.
    pub(crate) fn push(&mut self, planes: &mut [Vec<f32>]) {
        let frames = planes.first().map_or(0, Vec::len);
        self.channels = planes.len();
        for stage in &mut self.stages {
            stage.process(planes);
        }
        self.fed += frames as i64;
        let dropped = self.skip.min(frames);
        self.skip -= dropped;
        for plane in planes.iter_mut() {
            plane.drain(..dropped);
        }
        if let Some(next) = &mut self.out_next {
            *next += (frames - dropped) as i64;
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
        // Spectral and sample-domain effects have their own engines, never sections.
        CleanupKind::NoiseReduction { .. } | CleanupKind::Declick { .. } => Vec::new(),
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

/// Samples the cleanup chain of `cleanup` delays its signal by at `rate`: the sum of the
/// latencies of its enabled effects with a nonzero mix.
pub(crate) fn latency_samples(cleanup: &AudioCleanup, rate: u32) -> usize {
    cleanup
        .effects
        .iter()
        .filter(|effect| effect.enabled && effect.wet_percent > 0.0)
        .map(|effect| match effect.kind {
            CleanupKind::NoiseReduction { .. } => spectral::frame_size(rate),
            CleanupKind::Declick { .. } => declick::LATENCY,
            _ => 0,
        })
        .sum()
}
