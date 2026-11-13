//! Click removal in the sample domain: clicks are the outliers of a linear-prediction residual
//! and are replaced by autoregressive interpolation.
//!
//! Algorithm sources: the autoregressive model, its prediction-error detector and the
//! interpolation of the corrupted samples from the clean samples on both sides are the method of
//! Vaseghi and Rayner, "Detection and suppression of impulsive noise in speech communication
//! systems", IEE Proceedings I 137(1), 1990, and Godsill and Rayner, "Digital Audio Restoration:
//! A Statistical Model Based Approach", Springer, 1998. The block layout and the constants are
//! adapted from the structure reference's `audio-dsp` click remover.
//!
//! Each block of `BLOCK` samples is analysed in a span of `CONTEXT` earlier samples, the block
//! and `LOOKAHEAD` later samples. An order-`p` model is fitted to the Hann-windowed span by
//! autocorrelation and Levinson-Durbin recursion. Samples whose prediction error exceeds
//! `k` times the robust residual deviation (median absolute residual over 0.6745) mark a click;
//! marks at most four samples apart are merged. Each region is rebuilt from forward prediction
//! out of the clean past and backward prediction out of the clean future, cross-faded. Regions
//! longer than the complexity allows are transients and stay untouched. The delay is exactly
//! `LATENCY` samples.
#![allow(
    clippy::indexing_slicing,
    clippy::needless_range_loop,
    reason = "span, residual and prediction arrays are private and sized by the block constants"
)]

const BLOCK: usize = 512;
const CONTEXT: usize = 512;
const LOOKAHEAD: usize = 128;
const SPAN: usize = CONTEXT + BLOCK + LOOKAHEAD;
const MAX_ORDER: usize = 32;
/// Longest repairable click in samples.
const MAX_CLICK: usize = 80;
/// Samples between an input sample entering and its output leaving.
pub(super) const LATENCY: usize = BLOCK + LOOKAHEAD;
/// Upper bound in bytes of the state of one remover over two channels.
pub(super) const STATE_BYTES: u64 = 4 * (2 * (SPAN + BLOCK) + 4 * SPAN) as u64 + 4096;

/// Streaming buffers of one channel.
struct Channel {
    /// The span; the newest block sits at the end.
    buf: Vec<f32>,
    /// The finished block read out while the next one fills.
    out: Vec<f32>,
    pos: usize,
}

impl Channel {
    fn new() -> Self {
        Self {
            buf: vec![0.0; SPAN],
            out: vec![0.0; BLOCK],
            pos: 0,
        }
    }
}

/// Analysis arrays shared by every channel.
struct Scratch {
    window: Vec<f32>,
    windowed: Vec<f64>,
    residual: Vec<f32>,
    sorted: Vec<f32>,
}

/// Click removal of one stage over any number of channels.
pub(super) struct ClickRemover {
    /// Residual outliers beyond this many deviations are clicks.
    factor: f32,
    order: usize,
    max_len: usize,
    scratch: Scratch,
    channels: Vec<Channel>,
}

/// Linear-prediction coefficients `a[1..=p]` of `x` (prediction `sum a[k] * x[n - k]`) by the
/// Levinson-Durbin recursion. False for silent or degenerate input.
fn lpc(x: &[f64], p: usize, a: &mut [f64; MAX_ORDER + 1]) -> bool {
    let mut r = [0.0f64; MAX_ORDER + 1];
    for (lag, value) in r.iter_mut().enumerate().take(p + 1) {
        *value = x[lag..].iter().zip(x).map(|(u, v)| u * v).sum();
    }
    if r[0] <= 1e-12 {
        return false;
    }
    // White-noise correction keeps the recursion well conditioned.
    r[0] *= 1.0 + 1e-6;
    let mut error = r[0];
    let mut previous = [0.0f64; MAX_ORDER + 1];
    *a = [0.0; MAX_ORDER + 1];
    for i in 1..=p {
        let mut acc = r[i];
        for j in 1..i {
            acc -= a[j] * r[i - j];
        }
        let k = acc / error;
        previous[..i].copy_from_slice(&a[..i]);
        a[i] = k;
        for j in 1..i {
            a[j] = previous[j] - k * previous[i - j];
        }
        error *= 1.0 - k * k;
        if error <= 1e-15 {
            return false;
        }
    }
    true
}

impl ClickRemover {
    pub(super) fn new(threshold: f32, complexity: f32) -> Self {
        let window = (0..SPAN)
            .map(|i| {
                let phase = (i as f64 + 0.5) / SPAN as f64;
                (0.5 - 0.5 * (2.0 * std::f64::consts::PI * phase).cos()) as f32
            })
            .collect();
        let level = (complexity - 1.0) / 99.0;
        Self {
            factor: 4.0 + 0.2 * threshold,
            order: ((8.0 + 24.0 * level).round() as usize).min(MAX_ORDER),
            max_len: ((16.0 + 48.0 * level).round() as usize).min(MAX_CLICK),
            scratch: Scratch {
                window,
                windowed: vec![0.0; SPAN],
                residual: vec![0.0; SPAN],
                sorted: vec![0.0; SPAN],
            },
            channels: Vec::new(),
        }
    }

    /// Forgets all signal history.
    pub(super) fn reset(&mut self) {
        self.channels.clear();
    }

    /// Replaces each sample by the repaired sample that entered `LATENCY` samples earlier.
    pub(super) fn process(&mut self, planes: &mut [Vec<f32>]) {
        let Self {
            factor,
            order,
            max_len,
            scratch,
            channels,
        } = self;
        if channels.len() != planes.len() {
            *channels = planes.iter().map(|_| Channel::new()).collect();
        }
        for (plane, channel) in planes.iter_mut().zip(channels.iter_mut()) {
            for sample in plane.iter_mut() {
                let x = *sample;
                *sample = channel.out[channel.pos];
                channel.buf[SPAN - BLOCK + channel.pos] = x;
                channel.pos += 1;
                if channel.pos == BLOCK {
                    channel.pos = 0;
                    Self::repair(*order, *factor, *max_len, scratch, channel);
                    channel
                        .out
                        .copy_from_slice(&channel.buf[CONTEXT..CONTEXT + BLOCK]);
                    channel.buf.copy_within(BLOCK.., 0);
                }
            }
        }
    }

    /// Repairs the clicks inside the block of `channel`'s span.
    fn repair(p: usize, factor: f32, max_len: usize, scratch: &mut Scratch, channel: &mut Channel) {
        let buf = &mut channel.buf;
        for i in 0..SPAN {
            scratch.windowed[i] = f64::from(buf[i] * scratch.window[i]);
        }
        let mut a = [0.0f64; MAX_ORDER + 1];
        if !lpc(&scratch.windowed, p, &mut a) {
            return;
        }
        for n in 0..SPAN {
            scratch.residual[n] = if n < p {
                0.0
            } else {
                let mut prediction = 0.0f64;
                for j in 1..=p {
                    prediction += a[j] * f64::from(buf[n - j]);
                }
                (f64::from(buf[n]) - prediction) as f32
            };
        }
        for n in 0..SPAN {
            scratch.sorted[n] = scratch.residual[n].abs();
        }
        let valid = &mut scratch.sorted[p..];
        let middle = valid.len() / 2;
        let _ = valid.select_nth_unstable_by(middle, |x, y| x.total_cmp(y));
        let sigma = valid[middle] / 0.6745;
        if sigma <= 1e-9 {
            return;
        }
        let limit = factor * sigma;
        let mut n = CONTEXT;
        while n < CONTEXT + BLOCK {
            if scratch.residual[n].abs() <= limit {
                n += 1;
                continue;
            }
            // Group outliers separated by at most four samples.
            let first = n;
            let mut last = n;
            let mut m = n + 1;
            while m < SPAN - p && m <= last + 4 {
                if scratch.residual[m].abs() > limit {
                    last = m;
                }
                m += 1;
            }
            let start = first.saturating_sub(2).max(p);
            let end = (last + 3).min(SPAN - p - 1);
            n = last + 1;
            if end <= start || end - start > max_len {
                continue;
            }
            let len = end - start;
            let mut forward = [0.0f32; MAX_CLICK];
            let mut backward = [0.0f32; MAX_CLICK];
            // Forward prediction out of the clean past.
            for t in 0..len {
                let i = start + t;
                let mut prediction = 0.0f64;
                for j in 1..=p {
                    let v = if i - j >= start {
                        forward[i - j - start]
                    } else {
                        buf[i - j]
                    };
                    prediction += a[j] * f64::from(v);
                }
                forward[t] = prediction as f32;
            }
            // Backward prediction out of the clean future: the same stationary model reversed.
            for t in (0..len).rev() {
                let i = start + t;
                let mut prediction = 0.0f64;
                for j in 1..=p {
                    let v = if i + j < end {
                        backward[i + j - start]
                    } else {
                        buf[i + j]
                    };
                    prediction += a[j] * f64::from(v);
                }
                backward[t] = prediction as f32;
            }
            for t in 0..len {
                let w = (t + 1) as f32 / (len + 1) as f32;
                buf[start + t] = (1.0 - w) * forward[t] + w * backward[t];
            }
        }
    }
}
