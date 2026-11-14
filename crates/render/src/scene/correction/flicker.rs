//! Turns the per-frame brightness of one source into the gain that removes its flicker.
use std::sync::Arc;
use time::{TICKS_PER_SECOND, Tick};

/// Frames one track may describe (2.4 hours at 30 fps); bounds the encoded track to 3.4 MiB.
pub const MAX_FRAMES: usize = 1 << 18;
/// Versioned storage and application recipe; the engine puts it in every cache key.
pub const RECIPE: &str = "flicker-v1:linear-luma709-mean:grid80x45-cut0.25:hann-1s:gain-0.5-2";
/// Cells of the cut-detection grid, across and down.
pub const GRID: [usize; 2] = [80, 45];
/// A mean-normalised grid difference above this is a shot change. Measured by the footage
/// clean-up study: real and synthetic cuts 0.49-0.68, every other frame at most 0.10.
pub const CUT_THRESHOLD: f64 = 0.25;
/// Smallest and largest gain a frame may get. A choice, not measured: the study's fixtures stay
/// within 0.92-1.08.
pub const GAIN_RANGE: [f32; 2] = [0.5, 2.0];
/// Means below this (linear luma, 0..=1) count as black and take no gain.
pub const DARK: f32 = 1e-4;
const MAGIC: &[u8; 4] = b"VCFL";
const VERSION: u32 = 1;
const HEADER: usize = 12;
/// Bytes per frame in the encoding: an i64 time, an f32 mean and one cut byte.
const FRAME_BYTES: usize = 13;

/// The tracks of the flicker-corrected items of a project, by item.
pub type Tracks = std::collections::BTreeMap<project::ItemId, Arc<FlickerTrack>>;

/// Why a track or its encoding was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid flicker track: {0}")]
pub struct FlickerError(pub &'static str);

/// The mean linear luma of every frame of one source and where its shots change. The gain of a
/// frame is derived from these, so the track is derived data the engine computes once per source
/// and caches; trims and splits only read it.
#[derive(Clone, Debug, PartialEq)]
pub struct FlickerTrack {
    times: Vec<Tick>,
    means: Vec<f32>,
    /// `true` for a frame that starts a new shot; the first frame's flag is always `false`.
    cuts: Vec<bool>,
    gains: Vec<f32>,
}

impl FlickerTrack {
    /// A track of strictly increasing frame times, finite non-negative means in `0..=1` and the
    /// shot-start flags. The window follows the source's frame rate.
    pub fn new(
        times: Vec<Tick>,
        means: Vec<f32>,
        mut cuts: Vec<bool>,
    ) -> std::result::Result<Self, FlickerError> {
        let count = times.len();
        if count == 0 || count > MAX_FRAMES {
            return Err(FlickerError("frame count"));
        }
        if means.len() != count || cuts.len() != count {
            return Err(FlickerError("length"));
        }
        if times.windows(2).any(|pair| {
            let [before, after] = pair else { return true };
            before >= after
        }) {
            return Err(FlickerError("frame order"));
        }
        if means
            .iter()
            .any(|mean| !mean.is_finite() || !(0.0..=1.0).contains(mean))
        {
            return Err(FlickerError("mean"));
        }
        if let Some(first) = cuts.first_mut() {
            *first = false;
        }
        let half = half_window(&times);
        let gains = gains(&means, &cuts, half);
        Ok(Self {
            times,
            means,
            cuts,
            gains,
        })
    }
    /// Frame times in ascending source order.
    pub fn times(&self) -> &[Tick] {
        &self.times
    }
    pub fn len(&self) -> usize {
        self.times.len()
    }
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
    /// Mean linear luma of frame `index`.
    pub fn mean(&self, index: usize) -> Option<f32> {
        self.means.get(index).copied()
    }
    /// Whether frame `index` starts a new shot.
    pub fn is_cut(&self, index: usize) -> bool {
        self.cuts.get(index).copied().unwrap_or(false)
    }
    /// The gain that brings frame `index` to the rolling reference of its shot; 1 when absent.
    pub fn gain(&self, index: usize) -> f32 {
        self.gains.get(index).copied().unwrap_or(1.0)
    }
    /// Index of the frame covering `at`: the last frame at or before it, the first when none is.
    pub fn covering(&self, at: Tick) -> usize {
        self.times
            .partition_point(|time| *time <= at)
            .saturating_sub(1)
    }
    /// The gain of the frame covering source time `at`.
    pub fn gain_at(&self, at: Tick) -> f32 {
        self.gain(self.covering(at))
    }
    /// Deterministic little-endian bytes: the same track always encodes to the same bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + FRAME_BYTES * self.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.len() as u32).to_le_bytes());
        for time in &self.times {
            out.extend_from_slice(&time.0.to_le_bytes());
        }
        for mean in &self.means {
            out.extend_from_slice(&mean.to_le_bytes());
        }
        out.extend(self.cuts.iter().map(|cut| u8::from(*cut)));
        out
    }
    /// Read a track back, refusing any length, order or value `new` would refuse.
    pub fn decode(bytes: &[u8]) -> std::result::Result<Self, FlickerError> {
        let length = FlickerError("length");
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        if bytes.get(..4) != Some(MAGIC.as_slice()) || word(4) != Some(VERSION) {
            return Err(FlickerError("format"));
        }
        let count = word(8).ok_or(length)? as usize;
        if count == 0 || count > MAX_FRAMES || bytes.len() != HEADER + FRAME_BYTES * count {
            return Err(length);
        }
        let times_at = HEADER;
        let means_at = times_at + 8 * count;
        let cuts_at = means_at + 4 * count;
        let mut times = Vec::with_capacity(count);
        let mut means = Vec::with_capacity(count);
        for index in 0..count {
            let time = bytes
                .get(times_at + 8 * index..times_at + 8 * index + 8)
                .and_then(|raw| raw.try_into().ok())
                .ok_or(length)?;
            times.push(Tick(i64::from_le_bytes(time)));
            let mean = bytes
                .get(means_at + 4 * index..means_at + 4 * index + 4)
                .and_then(|raw| raw.try_into().ok())
                .ok_or(length)?;
            means.push(f32::from_le_bytes(mean));
        }
        let mut cuts = Vec::with_capacity(count);
        for byte in bytes.get(cuts_at..).ok_or(length)? {
            cuts.push(match *byte {
                0 => false,
                1 => true,
                _ => return Err(FlickerError("cut flag")),
            });
        }
        if cuts.first() == Some(&true) {
            return Err(FlickerError("cut flag"));
        }
        Self::new(times, means, cuts)
    }
}

/// Half the odd window the track's frame rate asks for: one second of frames, `round(fps)` made
/// odd. The study's 1 s Hann window removes flicker down to 1.7 Hz without pumping.
fn half_window(times: &[Tick]) -> usize {
    let frames = times.len().saturating_sub(1);
    let span = match (times.first(), times.last()) {
        (Some(first), Some(last)) if frames > 0 => (last.0 - first.0).max(1),
        _ => 0,
    };
    let fps = if span == 0 {
        30.0
    } else {
        frames as f64 * TICKS_PER_SECOND as f64 / span as f64
    };
    let window = (fps.round().clamp(3.0, 241.0) as usize) | 1;
    window / 2
}

/// The gain of every frame: the Hann-weighted mean of the means in the window of `half` frames
/// either side, kept inside the frame's own shot, over the frame's own mean. Weights are summed in
/// a fixed order in `f64`, so a result never depends on the host.
fn gains(means: &[f32], cuts: &[bool], half: usize) -> Vec<f32> {
    let count = means.len();
    // First frame of the shot of each frame, and the first frame of the next shot.
    let mut start = Vec::with_capacity(count);
    let mut shot = 0usize;
    for (index, cut) in cuts.iter().enumerate() {
        if *cut {
            shot = index;
        }
        start.push(shot);
    }
    let mut end = vec![count; count];
    let mut next = count;
    for (index, cut) in cuts.iter().enumerate().rev() {
        if let Some(slot) = end.get_mut(index) {
            *slot = next;
        }
        if *cut {
            next = index;
        }
    }
    let kernel: Vec<f64> = (0..=half)
        .map(|k| 0.5 * (1.0 + (std::f64::consts::PI * k as f64 / (half as f64 + 1.0)).cos()))
        .collect();
    (0..count)
        .map(|index| {
            let low = start
                .get(index)
                .copied()
                .unwrap_or(0)
                .max(index.saturating_sub(half));
            let high = end
                .get(index)
                .copied()
                .unwrap_or(count)
                .min(index + half + 1);
            let mut sum = 0.0f64;
            let mut weights = 0.0f64;
            for other in low..high {
                let weight = kernel.get(other.abs_diff(index)).copied().unwrap_or(0.0);
                sum += weight * f64::from(means.get(other).copied().unwrap_or(0.0));
                weights += weight;
            }
            let own = means.get(index).copied().unwrap_or(0.0);
            if own < DARK || weights <= 0.0 {
                return 1.0;
            }
            ((sum / weights) / f64::from(own)) as f32
        })
        .map(|gain| gain.clamp(GAIN_RANGE[0], GAIN_RANGE[1]))
        .collect()
}
