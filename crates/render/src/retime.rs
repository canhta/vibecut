//! Streaming audio retime for items that play at a constant speed other than 1x.
//!
//! The mix graph asks for item-local output samples in order; a [`Retimer`]
//! pulls the matching source samples (relative to the item's source start, on
//! the project sample grid) and either resamples them (pitch follows speed) or
//! time-stretches them (pitch preserved). Output sample `n` is a pure function of
//! `n`, the source and the speed, so the result is identical however the stream
//! is cut into blocks; a stream that does not continue replays from item start.
//!
//! Pitch following reads source position `n * speed` with linear interpolation,
//! area-averaged over one output step when speed exceeds 1 so fast playback does
//! not alias. Pitch preserving uses WSOLA (waveform-similarity overlap-add):
//! 40 ms Hann-squared grains at a fixed 20 ms output hop whose source offsets
//! are searched within +/-8 ms for the best continuation of the previous grain,
//! normalized by the summed window weight so item edges keep unity gain and
//! source samples outside the item's trimmed window never leak in.

use crate::{Error, ExecutionHooks, MemoryKind, Reservation, Result, check};

pub(crate) type Planes = Vec<Vec<f32>>;

const CHUNK: usize = 16384;
const MAX_BLOCK: usize = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pitch {
    /// Pitch follows speed (resampling).
    Follow,
    /// Pitch is preserved (time stretch).
    Keep,
}

pub(crate) struct Retimer {
    rate: u32,
    speed: (i64, i64),
    pitch: Pitch,
    length: i64,
    next: i64,
    source: Source,
    stretch: Stretch,
    _lease: Box<dyn Reservation>,
}

impl Retimer {
    /// `length` is the item's source window in samples; `speed` is the reduced
    /// ratio `(num, den)`.
    pub(crate) fn new(
        rate: u32,
        speed: (i64, i64),
        pitch: Pitch,
        length: i64,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        if speed.0 <= 0 || speed.1 <= 0 || length < 0 || !(8000..=192_000).contains(&rate) {
            return Err(Error::Invalid("retime parameters"));
        }
        let stretch = Stretch::new(rate);
        // Fast resampling holds up to five source frames per output frame,
        // in stereo plus mono, with a chunk of read-ahead and filter margins.
        // Allow amortized Vec capacity growth for that source storage; also cover
        // the pull block, overlap-add rows/weights and output staging.
        let max_speed = usize::from(time::Speed::MAX_THOUSANDTHS).div_ceil(1000);
        let source_samples = MAX_BLOCK * max_speed + CHUNK + 32;
        let samples = 6 * source_samples + 7 * MAX_BLOCK + stretch.grain * 32;
        let lease = hooks.reserve(MemoryKind::Compositor, samples as u64 * 4)?;
        Ok(Self {
            rate,
            speed,
            pitch,
            length,
            next: 0,
            source: Source::new(length),
            stretch,
            _lease: lease,
        })
    }

    pub(crate) fn matches(&self, rate: u32, speed: (i64, i64), pitch: Pitch, length: i64) -> bool {
        self.rate == rate && self.speed == speed && self.pitch == pitch && self.length == length
    }

    fn restart(&mut self) {
        self.next = 0;
        self.source = Source::new(self.length);
        self.stretch = Stretch::new(self.rate);
    }

    /// Item-local output samples `[start, start + frames)`, planar. A start that
    /// does not continue the previous call replays from the item start.
    pub(crate) fn read(
        &mut self,
        start: i64,
        frames: usize,
        pull: &mut dyn FnMut(i64, usize) -> Result<Planes>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Planes> {
        if frames > MAX_BLOCK || start < 0 {
            return Err(Error::Invalid("retime window"));
        }
        if start != self.next {
            if start < self.next {
                self.restart();
            }
            while self.next < start {
                check(hooks)?;
                let skip = ((start - self.next) as usize).min(MAX_BLOCK);
                self.run(self.next, skip, pull, hooks)?;
            }
        }
        check(hooks)?;
        self.run(start, frames, pull, hooks)
    }

    fn run(
        &mut self,
        start: i64,
        frames: usize,
        pull: &mut dyn FnMut(i64, usize) -> Result<Planes>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Planes> {
        let out = match self.pitch {
            Pitch::Follow => follow(&mut self.source, self.speed, start, frames, pull)?,
            Pitch::Keep => {
                self.stretch
                    .read(&mut self.source, self.speed, start, frames, pull, hooks)?
            }
        };
        self.next = start + frames as i64;
        Ok(out)
    }
}

/// Held source samples `[start, start + len)`; anything outside `0..length` is silence.
struct Source {
    length: i64,
    start: i64,
    planes: Planes,
    mono: Vec<f32>,
}

impl Source {
    fn new(length: i64) -> Self {
        Self {
            length,
            start: 0,
            planes: Vec::new(),
            mono: Vec::new(),
        }
    }

    fn held_end(&self) -> i64 {
        self.start + self.mono.len() as i64
    }

    fn channels(&self) -> usize {
        self.planes.len().max(1)
    }

    /// Hold every sample of `[from, to)` that lies inside the item's source window.
    /// `pull` receives source frames `(start, frames)` relative to the item's source start.
    fn ensure(
        &mut self,
        from: i64,
        to: i64,
        pull: &mut dyn FnMut(i64, usize) -> Result<Planes>,
    ) -> Result<()> {
        let to = to.min(self.length);
        if self.mono.is_empty() {
            self.start = self.start.max(from.clamp(0, self.length));
        }
        while self.held_end() < to {
            let at = self.held_end();
            let frames = ((to - at) as usize)
                .max(CHUNK)
                .min((self.length - at) as usize)
                .min(MAX_BLOCK);
            let got = pull(at, frames)?;
            if !matches!(got.len(), 1 | 2)
                || got.iter().any(|p| p.len() != frames)
                || got.iter().flatten().any(|v| !v.is_finite())
                || (!self.planes.is_empty() && got.len() != self.planes.len())
            {
                return Err(Error::Invalid("source audio block"));
            }
            if self.planes.is_empty() {
                self.planes = vec![Vec::new(); got.len()];
            }
            for (held, new) in self.planes.iter_mut().zip(&got) {
                held.extend_from_slice(new);
            }
            if let [left, right] = got.as_slice() {
                self.mono
                    .extend(left.iter().zip(right).map(|(l, r)| (l + r) * 0.5));
            } else if let Some(only) = got.first() {
                self.mono.extend_from_slice(only);
            }
        }
        Ok(())
    }

    /// Drop samples before `index`; later pulls continue forward from the held end.
    fn release_before(&mut self, index: i64) {
        let drop = (index - self.start).clamp(0, self.mono.len() as i64) as usize;
        if drop > 0 {
            for plane in &mut self.planes {
                plane.drain(..drop);
            }
            self.mono.drain(..drop);
            self.start += drop as i64;
        }
        if self.mono.is_empty() && index > self.start {
            self.start = index.min(self.length);
        }
    }

    fn at(&self, plane: usize, index: i64) -> f32 {
        if index < 0 || index >= self.length {
            return 0.0;
        }
        self.planes
            .get(plane)
            .and_then(|p| p.get(usize::try_from(index - self.start).ok()?))
            .copied()
            .unwrap_or(0.0)
    }

    fn mono_at(&self, index: i64) -> f32 {
        if index < 0 || index >= self.length {
            return 0.0;
        }
        usize::try_from(index - self.start)
            .ok()
            .and_then(|i| self.mono.get(i))
            .copied()
            .unwrap_or(0.0)
    }
}

/// Pitch follows speed: output `n` reads source position `n * num / den`.
fn follow(
    source: &mut Source,
    (num, den): (i64, i64),
    start: i64,
    frames: usize,
    pull: &mut dyn FnMut(i64, usize) -> Result<Planes>,
) -> Result<Planes> {
    // Area average over one output step when decimating: ceil(speed) linear taps.
    let taps = (num + den - 1) / den;
    let reach = taps / 2 + 2;
    let position = |n: i64| -> (i64, f64) {
        let scaled = i128::from(n) * i128::from(num);
        (
            scaled.div_euclid(i128::from(den)) as i64,
            scaled.rem_euclid(i128::from(den)) as f64 / den as f64,
        )
    };
    let (first, _) = position(start);
    let (last, _) = position(start + frames as i64);
    source.ensure(first - reach, last + reach + 1, pull)?;
    let spacing = num as f64 / den as f64 / taps as f64;
    let mut out = vec![vec![0.0f32; frames]; source.channels()];
    for i in 0..frames {
        let (whole, fraction) = position(start + i as i64);
        for (c, plane) in out.iter_mut().enumerate() {
            let mut sum = 0.0f64;
            for t in 0..taps {
                let offset = if taps == 1 {
                    0.0
                } else {
                    (t as f64 + 0.5) * spacing - num as f64 / den as f64 / 2.0
                };
                let at = fraction + offset;
                let base = at.floor();
                let s = whole + base as i64;
                let f = at - base;
                let a = f64::from(source.at(c, s));
                let b = f64::from(source.at(c, s + 1));
                sum += a + (b - a) * f;
            }
            if let Some(sample) = plane.get_mut(i) {
                *sample = (sum / taps as f64) as f32;
            }
        }
    }
    let (next, _) = position(start + frames as i64);
    source.release_before(next - reach - 1);
    Ok(out)
}

/// WSOLA state: grains of `grain` samples every `hop` output samples.
struct Stretch {
    hop: i64,
    grain: usize,
    reach: i64,
    window: Vec<f32>,
    next_grain: i64,
    previous: Option<i64>,
    origin: i64,
    sum: Planes,
    weight: Vec<f32>,
    target: Vec<f32>,
    candidates: Vec<f32>,
}

impl Stretch {
    fn new(rate: u32) -> Self {
        let hop = (i64::from(rate) / 50).max(64);
        let grain = (hop * 2) as usize;
        let window = (0..grain)
            .map(|j| {
                let phase = std::f64::consts::PI * (j as f64 + 0.5) / grain as f64;
                phase.sin().powi(2) as f32
            })
            .collect();
        Self {
            hop,
            grain,
            reach: (i64::from(rate) / 125).max(16).min(hop - 1),
            window,
            next_grain: -1,
            previous: None,
            origin: -hop,
            sum: Vec::new(),
            weight: Vec::new(),
            target: Vec::new(),
            candidates: Vec::new(),
        }
    }

    fn read(
        &mut self,
        source: &mut Source,
        speed: (i64, i64),
        start: i64,
        frames: usize,
        pull: &mut dyn FnMut(i64, usize) -> Result<Planes>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Planes> {
        let end = start + frames as i64;
        while self.next_grain * self.hop < end {
            check(hooks)?;
            self.synthesize(source, speed, pull)?;
            check(hooks)?;
        }
        let channels = source.channels();
        if self.sum.len() != channels {
            self.sum = vec![vec![0.0; self.weight.len()]; channels];
        }
        let mut out = vec![vec![0.0f32; frames]; channels];
        for (plane, sum) in out.iter_mut().zip(&self.sum) {
            for (i, sample) in plane.iter_mut().enumerate() {
                let index = (start - self.origin) as usize + i;
                let weight = self.weight.get(index).copied().unwrap_or(0.0);
                if weight > 0.0 {
                    *sample = sum.get(index).copied().unwrap_or(0.0) / weight;
                }
            }
        }
        // Samples before `end` are final: no later grain starts before `end - grain + hop`.
        let drop = ((end - self.origin) as usize).min(self.weight.len());
        for plane in &mut self.sum {
            plane.drain(..drop);
        }
        self.weight.drain(..drop);
        self.origin += drop as i64;
        Ok(out)
    }

    fn synthesize(
        &mut self,
        source: &mut Source,
        (num, den): (i64, i64),
        pull: &mut dyn FnMut(i64, usize) -> Result<Planes>,
    ) -> Result<()> {
        let hop = self.hop;
        let k = self.next_grain;
        let output = k * hop;
        // The grain centre plays source position (output + hop) * speed.
        let centre = (i128::from(output + hop) * i128::from(num) + i128::from(den) / 2)
            .div_euclid(i128::from(den)) as i64;
        let nominal = centre - hop;
        let length = source.length;
        let span_end = nominal + self.reach + self.grain as i64;
        let live = nominal - self.reach < length && span_end > 0;
        let mut chosen = nominal;
        if live {
            let held_from = self.previous.map_or(nominal - self.reach, |p| {
                (p + hop).min(nominal - self.reach)
            });
            source.ensure(held_from.max(0), span_end, pull)?;
            if k > 0
                && let Some(previous) = self.previous
            {
                chosen = nominal + self.best_offset(source, previous + hop, nominal);
            }
            self.add_grain(source, output, chosen);
        }
        self.previous = Some(chosen);
        self.next_grain += 1;
        let next_nominal = {
            let c = (i128::from((k + 1) * hop + hop) * i128::from(num) + i128::from(den) / 2)
                .div_euclid(i128::from(den)) as i64;
            c - hop
        };
        source.release_before((next_nominal - self.reach).min(chosen + hop));
        Ok(())
    }

    /// Offset within +/-reach whose first `hop` samples best continue `target`.
    fn best_offset(&mut self, source: &Source, target: i64, nominal: i64) -> i64 {
        let hop = self.hop as usize;
        let reach = self.reach;
        self.target.clear();
        self.target
            .extend((0..hop).map(|j| source.mono_at(target + j as i64)));
        if self.target.iter().all(|v| *v == 0.0) {
            return 0;
        }
        self.candidates.clear();
        self.candidates.extend(
            (0..hop + 2 * reach as usize).map(|j| source.mono_at(nominal - reach + j as i64)),
        );
        let mut energy = self.candidates.iter().take(hop).map(|v| v * v).sum::<f32>();
        let mut scores = Vec::with_capacity(2 * reach as usize + 1);
        for shift in 0..=2 * reach as usize {
            let dot: f32 = self
                .target
                .iter()
                .zip(self.candidates.iter().skip(shift))
                .map(|(a, b)| a * b)
                .sum();
            scores.push(dot / (energy + 1e-6).sqrt());
            if let (Some(out), Some(entering)) =
                (self.candidates.get(shift), self.candidates.get(shift + hop))
            {
                energy = (energy - out * out + entering * entering).max(0.0);
            }
        }
        // Prefer the smallest displacement on ties: 0, -1, +1, -2, +2, ...
        let mut best = (f32::NEG_INFINITY, 0i64);
        for step in 0..=2 * reach {
            let d = if step % 2 == 0 {
                step / 2
            } else {
                -(step + 1) / 2
            };
            let score = scores
                .get((d + reach) as usize)
                .copied()
                .unwrap_or(f32::MIN);
            if score > best.0 {
                best = (score, d);
            }
        }
        best.1
    }

    fn add_grain(&mut self, source: &Source, output: i64, from: i64) {
        let channels = source.channels();
        let end = (output + self.grain as i64 - self.origin) as usize;
        if self.weight.len() < end {
            self.weight.resize(end, 0.0);
        }
        if self.sum.len() != channels {
            self.sum = vec![Vec::new(); channels];
        }
        for plane in &mut self.sum {
            if plane.len() < end {
                plane.resize(end, 0.0);
            }
        }
        for (j, w) in self.window.iter().enumerate() {
            let s = from + j as i64;
            if s < 0 || s >= source.length {
                continue;
            }
            let index = (output - self.origin) as usize + j;
            if let Some(weight) = self.weight.get_mut(index) {
                *weight += w;
            }
            for (c, plane) in self.sum.iter_mut().enumerate() {
                if let Some(slot) = plane.get_mut(index) {
                    *slot += w * source.at(c, s);
                }
            }
        }
    }
}
