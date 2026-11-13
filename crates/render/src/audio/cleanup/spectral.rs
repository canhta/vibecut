//! Broadband noise reduction by short-time spectral gating with a continuously tracked floor.
//!
//! Algorithm sources: the short-time Fourier analysis, windowed overlap-add resynthesis follows
//! Griffin and Lim, "Signal estimation from modified short-time Fourier transform", IEEE Trans.
//! ASSP 32(2), 1984. Attenuating bins that do not rise above an estimate of the noise spectrum is
//! the spectral subtraction family of Boll, "Suppression of acoustic noise in speech using
//! spectral subtraction", IEEE Trans. ASSP 27(2), 1979. The noise spectrum is the minimum of the
//! smoothed periodogram over a sliding search window, bias compensated, after Martin, "Noise power
//! spectral density estimation based on optimal smoothing and minimum statistics", IEEE Trans.
//! Speech and Audio Processing 9(5), 2001. The frame, gate and smoothing constants are adapted
//! from the structure reference's `audio-dsp` DeNoise.
//!
//! Frames are `frame_size(rate)` samples long with 75 % overlap and square-root Hann analysis and
//! synthesis windows, which sum to a constant. The delay is exactly one frame: sample `n` leaves
//! after sample `n + latency` entered, whatever the block sizes, so the caller can compensate it
//! by reading ahead.
#![allow(
    clippy::indexing_slicing,
    clippy::needless_range_loop,
    reason = "frame, window and per-bin arrays are private and sized together at construction"
)]

use super::fft::Fft;
use std::f64::consts::PI;

/// Frames per window length: the hop is `size / OVERLAP`.
const OVERLAP: usize = 4;
/// Sub-windows of the minimum-statistics search.
const MS_SUBWINDOWS: usize = 8;
/// Length of the minimum-statistics search window in seconds.
const MS_WINDOW_SECONDS: f32 = 1.5;
/// Bias compensation of the minimum of smoothed periodograms (about +4 dB).
const MS_BIAS: f32 = 2.5;
/// Recursive smoothing of the per-bin power.
const PSD_SMOOTH: f32 = 0.8;
/// Gate opening speed per frame; the closing speed follows the smoothing parameter.
const GAIN_ATTACK: f32 = 0.7;

/// The FFT size that keeps a frame near 43 ms.
pub(super) fn frame_size(rate: u32) -> usize {
    match rate {
        ..=50_000 => 2048,
        ..=100_000 => 4096,
        _ => 8192,
    }
}

/// Upper bound in bytes of the state of one reducer over two channels.
pub(super) fn state_bytes(rate: u32) -> u64 {
    let size = frame_size(rate) as u64;
    let bins = size / 2 + 1;
    4 * (9 * size + 2 * (2 * size + (MS_SUBWINDOWS as u64 + 3) * bins))
}

struct Params {
    size: usize,
    hop: usize,
    /// Frames per minimum-statistics sub-window.
    sub_frames: usize,
    /// Gain of a gated bin.
    floor_gain: f32,
    /// Power ratio over the floor that opens a bin.
    open_ratio: f32,
    /// Gate closing speed per frame.
    release: f32,
}

/// Transform buffers shared by every channel.
struct Scratch {
    fft: Fft,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    smoothed: Vec<f32>,
}

/// Per-bin floor tracking and gate state of one channel.
struct Tracker {
    psd: Vec<f32>,
    current_min: Vec<f32>,
    ring: Vec<f32>,
    ring_pos: usize,
    frames_in_sub: usize,
    gains: Vec<f32>,
    /// Frames analysed so far.
    seen: usize,
}

impl Tracker {
    fn new(bins: usize) -> Self {
        Self {
            psd: vec![0.0; bins],
            current_min: vec![f32::INFINITY; bins],
            ring: vec![f32::INFINITY; bins * MS_SUBWINDOWS],
            ring_pos: 0,
            frames_in_sub: 0,
            gains: vec![1.0; bins],
            seen: 0,
        }
    }
}

/// Streaming buffers of one channel.
struct Channel {
    /// The last `size` samples; the newest hop sits at the end.
    input: Vec<f32>,
    /// Overlap-added output; the oldest finished hop is read from the start.
    output: Vec<f32>,
    pos: usize,
    tracker: Tracker,
}

impl Channel {
    fn new(size: usize) -> Self {
        Self {
            input: vec![0.0; size],
            output: vec![0.0; size],
            pos: 0,
            tracker: Tracker::new(size / 2 + 1),
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
            frame(
                params,
                scratch,
                &mut self.tracker,
                &self.input,
                &mut self.output,
            );
            self.input.copy_within(hop.., 0);
        }
        y
    }
}

/// One analysis frame: track the floor, gate each bin and overlap-add the resynthesis.
fn frame(
    params: &Params,
    scratch: &mut Scratch,
    tracker: &mut Tracker,
    input: &[f32],
    output: &mut [f32],
) {
    let size = params.size;
    let bins = size / 2 + 1;
    for (((re, im), x), w) in scratch
        .re
        .iter_mut()
        .zip(scratch.im.iter_mut())
        .zip(input)
        .zip(&scratch.window)
    {
        *re = x * w;
        *im = 0.0;
    }
    scratch
        .fft
        .transform(&mut scratch.re, &mut scratch.im, false);

    // Frames that still hold the zeroed start of the buffer say nothing about the floor, and the
    // gate stays open until one sub-window of real frames has been seen.
    let tracking = tracker.seen >= OVERLAP;
    let first = tracker.seen == OVERLAP;
    let settled = tracker.seen >= OVERLAP + params.sub_frames;
    tracker.seen = tracker.seen.saturating_add(1);
    if tracking {
        for k in 0..bins {
            let power = scratch.re[k] * scratch.re[k] + scratch.im[k] * scratch.im[k];
            tracker.psd[k] = if first {
                power
            } else {
                PSD_SMOOTH * tracker.psd[k] + (1.0 - PSD_SMOOTH) * power
            };
            tracker.current_min[k] = tracker.current_min[k].min(tracker.psd[k]);
        }
        tracker.frames_in_sub += 1;
        let rotate = tracker.frames_in_sub == params.sub_frames;
        for k in 0..bins {
            let mut floor = tracker.current_min[k];
            for sub in 0..MS_SUBWINDOWS {
                floor = floor.min(tracker.ring[sub * bins + k]);
            }
            let open = !settled || tracker.psd[k] > floor * MS_BIAS * params.open_ratio;
            let target = if open { 1.0 } else { params.floor_gain };
            let gain = &mut tracker.gains[k];
            let speed = if target > *gain {
                GAIN_ATTACK
            } else {
                params.release
            };
            *gain += (target - *gain) * speed;
            if rotate {
                tracker.ring[tracker.ring_pos * bins + k] = tracker.current_min[k];
                tracker.current_min[k] = f32::INFINITY;
            }
        }
        if rotate {
            tracker.ring_pos = (tracker.ring_pos + 1) % MS_SUBWINDOWS;
            tracker.frames_in_sub = 0;
        }
    }

    // Three-tap smoothing across frequency against musical noise.
    for k in 0..bins {
        let below = tracker.gains[k.saturating_sub(1)];
        let above = tracker.gains[(k + 1).min(bins - 1)];
        scratch.smoothed[k] = 0.25 * below + 0.5 * tracker.gains[k] + 0.25 * above;
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
    for ((out, re), w) in output.iter_mut().zip(&scratch.re).zip(&scratch.window) {
        *out += re * w * scale;
    }
}

/// Noise reduction of one stage over any number of channels.
pub(super) struct NoiseReducer {
    params: Params,
    scratch: Scratch,
    channels: Vec<Channel>,
}

impl NoiseReducer {
    pub(super) fn new(
        rate: u32,
        reduction_db: f32,
        threshold_db: f32,
        smoothing_percent: f32,
    ) -> Self {
        let size = frame_size(rate);
        let hop = size / OVERLAP;
        let sub_frames = (MS_WINDOW_SECONDS * rate as f32 / hop as f32 / MS_SUBWINDOWS as f32)
            .round()
            .max(1.0) as usize;
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
                sub_frames,
                floor_gain: 10f32.powf(-reduction_db / 20.0),
                open_ratio: 10f32.powf(threshold_db / 10.0),
                // 0 % closes quickly, 100 % slowly.
                release: 0.6 - 0.57 * smoothing_percent / 100.0,
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
                .map(|_| Channel::new(self.params.size))
                .collect();
        }
        for (plane, channel) in planes.iter_mut().zip(&mut self.channels) {
            for sample in plane.iter_mut() {
                *sample = channel.tick(*sample, &self.params, &mut self.scratch);
            }
        }
    }
}
