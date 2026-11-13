//! Late-reverberation suppression by spectral subtraction with a statistical decay model.
//!
//! The reverberant tail is modelled as an exponentially decaying diffuse field (Polack): the late
//! reverberation power of a bin is predicted from the smoothed power `LATE_DELAY_SECONDS` earlier,
//! `late = exp(-2 * delta * Td) * power(t - Td)` with `delta = 3 ln(10) / RT60`, and removed by
//! over-subtracted spectral gain `max(1 - beta * late / power, floor)`, `beta = 1 + amount`,
//! `floor = -18 dB * amount`. Sources: Lebart, Boucher and Denbigh, "A new method based on
//! spectral subtraction for speech dereverberation", Acta Acustica 87, 2001; Habets, "Single- and
//! multi-microphone speech dereverberation using spectral enhancement", PhD thesis, 2007. The
//! constants and gain smoothing are adapted from the structure reference's `audio-dsp` DeReverb.
//!
//! The transform layout is the noise reduction's: frames of `frame_size(rate)` samples, 75 %
//! overlap and square-root Hann windows that sum to a constant, so at `amount` 0 every gain is
//! one and the stage is an exact delayed identity. The delay is exactly one frame, whatever the
//! block sizes, so the caller compensates it by reading ahead.
#![allow(
    clippy::indexing_slicing,
    clippy::needless_range_loop,
    reason = "frame, window and per-bin arrays are private and sized together at construction"
)]

use super::fft::Fft;
use super::spectral::frame_size;
use std::f64::consts::{LN_10, PI};

/// Frames per window length: the hop is `size / OVERLAP`.
const OVERLAP: usize = 4;
/// Reflections later than this count as reverberation.
const LATE_DELAY_SECONDS: f64 = 0.05;
/// Time constant of the per-bin power smoothing.
const POWER_SMOOTHING_SECONDS: f64 = 0.02;
/// Most frames the late-delay history holds; bounds the state at any sample rate.
const MAX_DELAY_FRAMES: usize = 8;
/// Gain opening speed per frame: fast recovery on onsets.
const GAIN_ATTACK: f32 = 0.8;
/// Gain closing speed per frame: slower suppression, less musical noise.
const GAIN_RELEASE: f32 = 0.5;
/// Gain floor at 100 % amount, in dB.
const FLOOR_AT_FULL_DB: f32 = -18.0;

/// Upper bound in bytes of the state of one de-reverberator over two channels.
pub(super) fn state_bytes(rate: u32) -> u64 {
    let size = frame_size(rate) as u64;
    let bins = size / 2 + 1;
    4 * (4 * size + 2 * (2 * size + (MAX_DELAY_FRAMES as u64 + 3) * bins))
}

struct Params {
    size: usize,
    hop: usize,
    /// Frames between a bin's power and the late reverberation it predicts.
    delay: usize,
    /// Power ratio of the decay over the late delay.
    decay: f32,
    beta: f32,
    floor: f32,
    /// One-pole coefficient of the power smoothing per frame.
    alpha: f32,
}

/// Transform buffers shared by every channel.
struct Scratch {
    fft: Fft,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    smoothed: Vec<f32>,
}

/// Streaming buffers and per-bin state of one channel.
struct Channel {
    /// The last `size` samples; the newest hop sits at the end.
    input: Vec<f32>,
    /// Overlap-added output; the oldest finished hop is read from the start.
    output: Vec<f32>,
    pos: usize,
    /// Smoothed power per bin.
    power: Vec<f32>,
    /// Ring of past smoothed powers, `delay` frames deep.
    history: Vec<f32>,
    history_pos: usize,
    gains: Vec<f32>,
    frames: usize,
}

impl Channel {
    fn new(params: &Params) -> Self {
        let bins = params.size / 2 + 1;
        Self {
            input: vec![0.0; params.size],
            output: vec![0.0; params.size],
            pos: 0,
            power: vec![0.0; bins],
            history: vec![0.0; bins * params.delay],
            history_pos: 0,
            gains: vec![1.0; bins],
            frames: 0,
        }
    }

    /// Pushes one sample and pops the one that entered `size` samples earlier, processed.
    fn tick(&mut self, x: f32, params: &Params, scratch: &mut Scratch) -> f32 {
        let (size, hop) = (params.size, params.hop);
        self.input[size - hop + self.pos] = x;
        let y = self.output[self.pos];
        self.pos += 1;
        if self.pos == hop {
            self.pos = 0;
            self.output.copy_within(hop.., 0);
            self.output[size - hop..].fill(0.0);
            self.frame(params, scratch);
            self.input.copy_within(hop.., 0);
        }
        y
    }

    /// One analysis frame: update the late-reverberation gains and overlap-add the resynthesis.
    fn frame(&mut self, params: &Params, scratch: &mut Scratch) {
        let size = params.size;
        let bins = size / 2 + 1;
        for (((re, im), x), w) in scratch
            .re
            .iter_mut()
            .zip(scratch.im.iter_mut())
            .zip(&self.input)
            .zip(&scratch.window)
        {
            *re = x * w;
            *im = 0.0;
        }
        scratch
            .fft
            .transform(&mut scratch.re, &mut scratch.im, false);

        let warm = self.frames >= params.delay;
        let base = self.history_pos * bins;
        for k in 0..bins {
            let instant = scratch.re[k] * scratch.re[k] + scratch.im[k] * scratch.im[k];
            let power = params.alpha * self.power[k] + (1.0 - params.alpha) * instant;
            self.power[k] = if power < 1e-30 { 0.0 } else { power };
            // The ring slot about to be overwritten holds the power `delay` frames ago.
            let late = if warm {
                params.decay * self.history[base + k]
            } else {
                0.0
            };
            self.history[base + k] = self.power[k];
            let target = if self.power[k] > 0.0 {
                (1.0 - params.beta * late / self.power[k]).max(params.floor)
            } else {
                1.0
            };
            let gain = &mut self.gains[k];
            let speed = if target > *gain {
                GAIN_ATTACK
            } else {
                GAIN_RELEASE
            };
            *gain += (target - *gain) * speed;
        }
        self.history_pos = (self.history_pos + 1) % params.delay;
        self.frames = self.frames.saturating_add(1);

        // Three-tap smoothing across frequency against musical noise.
        for k in 0..bins {
            let below = self.gains[k.saturating_sub(1)];
            let above = self.gains[(k + 1).min(bins - 1)];
            scratch.smoothed[k] = 0.25 * below + 0.5 * self.gains[k] + 0.25 * above;
        }
        for k in 0..bins {
            scratch.re[k] *= scratch.smoothed[k];
            scratch.im[k] *= scratch.smoothed[k];
        }
        // Hermitian symmetry from bins 0..=size/2, inverse transform, synthesis window.
        scratch.im[0] = 0.0;
        scratch.im[size / 2] = 0.0;
        for k in 1..size / 2 {
            scratch.re[size - k] = scratch.re[k];
            scratch.im[size - k] = -scratch.im[k];
        }
        scratch
            .fft
            .transform(&mut scratch.re, &mut scratch.im, true);
        let scale = 2.0 / (size * OVERLAP) as f32;
        for ((out, re), w) in self.output.iter_mut().zip(&scratch.re).zip(&scratch.window) {
            *out += re * w * scale;
        }
    }
}

/// De-reverberation of one stage over any number of channels.
pub(super) struct Dereverber {
    params: Params,
    scratch: Scratch,
    channels: Vec<Channel>,
}

impl Dereverber {
    pub(super) fn new(rate: u32, amount_percent: f32, rt60_seconds: f32) -> Self {
        let size = frame_size(rate);
        let hop = size / OVERLAP;
        let rate = f64::from(rate);
        let amount = amount_percent / 100.0;
        let delay = ((LATE_DELAY_SECONDS * rate / hop as f64).round() as usize)
            .clamp(1, MAX_DELAY_FRAMES);
        let delta = 3.0 * LN_10 / f64::from(rt60_seconds);
        let late_seconds = delay as f64 * hop as f64 / rate;
        let window = (0..size)
            .map(|i| {
                let hann = 0.5 - 0.5 * (2.0 * PI * i as f64 / size as f64).cos();
                hann.sqrt() as f32
            })
            .collect();
        Self {
            params: Params {
                size,
                hop,
                delay,
                decay: (-2.0 * delta * late_seconds).exp() as f32,
                beta: 1.0 + amount,
                floor: 10f32.powf(FLOOR_AT_FULL_DB * amount / 20.0),
                alpha: (-(hop as f64) / (rate * POWER_SMOOTHING_SECONDS)).exp() as f32,
            },
            scratch: Scratch {
                fft: Fft::new(size),
                window,
                re: vec![0.0; size],
                im: vec![0.0; size],
                smoothed: vec![0.0; size / 2 + 1],
            },
            channels: Vec::new(),
        }
    }

    /// Samples between an input sample entering and its output leaving.
    pub(super) fn latency(&self) -> usize {
        self.params.size
    }

    /// Forgets all signal history.
    pub(super) fn reset(&mut self) {
        self.channels.clear();
    }

    /// Replaces each sample by the output that left with it, delayed by `latency`.
    pub(super) fn process(&mut self, planes: &mut [Vec<f32>]) {
        if self.channels.len() != planes.len() {
            self.channels = planes
                .iter()
                .map(|_| Channel::new(&self.params))
                .collect();
        }
        for (plane, channel) in planes.iter_mut().zip(&mut self.channels) {
            for sample in plane.iter_mut() {
                *sample = channel.tick(*sample, &self.params, &mut self.scratch);
            }
        }
    }
}
