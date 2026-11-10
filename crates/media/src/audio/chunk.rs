//! Splits a speech range into bounded PCM windows.

use crate::{MediaError, Result, SourceAsset};
use std::sync::{Arc, atomic::AtomicBool};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

use super::extract::{
    ENERGY_FRAME_SAMPLES, SPEECH_SAMPLE_RATE, decode_speech, mean_square, samples_to_ticks,
};

const SILENCE_RMS: f64 = 1e-4;
const SEARCH_SAMPLES: usize = 2 * SPEECH_SAMPLE_RATE as usize;

/// One mono 16 kHz PCM invocation window in absolute source ticks.
#[derive(Clone, Debug)]
pub struct SpeechWindow {
    /// Exact source ticks decoded into this window.
    pub source_range: SourceRange,
    /// Ticks shared with the preceding window; zero for the first window.
    pub overlap: Tick,
    /// Mono finite f32 PCM at [`SPEECH_SAMPLE_RATE`].
    pub samples: Vec<f32>,
    /// Energy hint: false when the window is effectively silent, not a VAD result.
    pub speech_detected: bool,
}

/// Bounds for [`extract_speech_windows_with_cancel`].
#[derive(Clone, Debug)]
pub struct SpeechExtractSettings {
    /// Hard upper bound for one window's audio duration.
    pub max_duration: Tick,
    /// Audio repeated at the start of the next window.
    pub overlap: Tick,
}
impl Default for SpeechExtractSettings {
    fn default() -> Self {
        Self {
            max_duration: Tick(600 * TICKS_PER_SECOND),
            overlap: Tick(2 * TICKS_PER_SECOND),
        }
    }
}

/// Stream bounded mono 16 kHz PCM windows in speech-safe boundaries without
/// producing an encoded file. Each callback runs before decoding continues; at
/// most one window of PCM is retained by this crate.
pub fn extract_speech_windows_with_cancel(
    asset: &SourceAsset,
    range: SourceRange,
    settings: &SpeechExtractSettings,
    cancel: Arc<AtomicBool>,
    window: impl FnMut(SpeechWindow) -> Result<()>,
) -> Result<()> {
    let plan = Plan::new(settings)?;
    let mut sink = PcmSink {
        overlap: Tick::ZERO,
        samples: Vec::new(),
        energy: 0.0,
        sample_count: 0,
        windows: 0,
        window,
    };
    extract_windows(asset, range, plan, cancel, &mut sink)
}

fn extract_windows<F: FnMut(SpeechWindow) -> Result<()>>(
    asset: &SourceAsset,
    range: SourceRange,
    plan: Plan,
    cancel: Arc<AtomicBool>,
    sink: &mut PcmSink<F>,
) -> Result<()> {
    let mut chunker = Chunker::new(plan, sink);
    decode_speech(asset, range, &cancel, &mut |samples, tick| {
        chunker.push(samples, tick)
    })?;
    chunker.finish()
}

#[derive(Clone, Copy)]
struct Plan {
    target: usize,
    overlap_samples: usize,
    overlap_ticks: Tick,
    search: usize,
    confirmed: usize,
}
impl Plan {
    fn new(settings: &SpeechExtractSettings) -> Result<Self> {
        if settings.max_duration <= Tick::ZERO || settings.overlap < Tick::ZERO {
            return Err(MediaError::Invalid("speech extract settings"));
        }
        let target = usize::try_from(
            settings
                .max_duration
                .to_units(i64::from(SPEECH_SAMPLE_RATE), Rounding::Floor)?,
        )
        .map_err(|_| MediaError::Invalid("speech chunk duration"))?;
        let overlap_samples = usize::try_from(
            settings
                .overlap
                .to_units(i64::from(SPEECH_SAMPLE_RATE), Rounding::Nearest)?,
        )
        .map_err(|_| MediaError::Invalid("speech overlap"))?;
        let search = SEARCH_SAMPLES.min(target / 4);
        if target
            <= search
                .saturating_add(overlap_samples.saturating_mul(2))
                .saturating_add(1)
        {
            return Err(MediaError::Invalid("speech chunk plan too small"));
        }
        Ok(Self {
            target,
            overlap_samples,
            overlap_ticks: samples_to_ticks(overlap_samples as i64)?,
            search,
            confirmed: target - search - overlap_samples * 2 - 1,
        })
    }
}

struct PcmSink<F> {
    overlap: Tick,
    samples: Vec<f32>,
    energy: f64,
    sample_count: usize,
    windows: usize,
    window: F,
}
impl<F: FnMut(SpeechWindow) -> Result<()>> PcmSink<F> {
    fn begin(&mut self, overlap: Tick) {
        self.overlap = overlap;
        self.samples.clear();
        self.energy = 0.0;
        self.sample_count = 0;
    }

    fn feed(&mut self, samples: &[f32]) -> Result<()> {
        self.samples.extend_from_slice(samples);
        self.energy += samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>();
        self.sample_count = self
            .sample_count
            .checked_add(samples.len())
            .ok_or(MediaError::TooLarge("speech window samples"))?;
        Ok(())
    }

    fn commit(&mut self, start: Tick, end: Tick) -> Result<()> {
        let speech_detected =
            self.sample_count > 0 && (self.energy / self.sample_count as f64).sqrt() >= SILENCE_RMS;
        let window = SpeechWindow {
            source_range: SourceRange::new(start, end)?,
            overlap: self.overlap,
            samples: std::mem::take(&mut self.samples),
            speech_detected,
        };
        self.windows += 1;
        (self.window)(window)
    }

    fn count(&self) -> usize {
        self.windows
    }
}

struct Chunker<'a, F> {
    plan: Plan,
    pending: Vec<f32>,
    pending_start: Option<Tick>,
    expected_input: Option<Tick>,
    fed: usize,
    current_start: Option<Tick>,
    sink: &'a mut PcmSink<F>,
}
impl<'a, F: FnMut(SpeechWindow) -> Result<()>> Chunker<'a, F> {
    fn new(plan: Plan, sink: &'a mut PcmSink<F>) -> Self {
        Chunker {
            plan,
            pending: Vec::new(),
            pending_start: None,
            expected_input: None,
            fed: 0,
            current_start: None,
            sink,
        }
    }

    fn push(&mut self, samples: &[f32], start: Tick) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        if let Some(expected) = self.expected_input
            && expected != start
        {
            return Err(MediaError::Invalid("speech audio discontinuity"));
        }
        let count =
            i64::try_from(samples.len()).map_err(|_| MediaError::TooLarge("speech samples"))?;
        self.expected_input = Some(
            start
                .checked_add(samples_to_ticks(count)?)
                .ok_or(MediaError::TooLarge("speech timestamp"))?,
        );
        if self.pending.is_empty() {
            self.pending_start = Some(start);
        }
        self.pending.extend_from_slice(samples);
        self.pump(false)
    }

    fn finish(mut self) -> Result<()> {
        self.pump(true)?;
        if self.sink.count() == 0 {
            return Err(MediaError::Invalid("no audio samples in range"));
        }
        Ok(())
    }

    fn pump(&mut self, finalizing: bool) -> Result<()> {
        loop {
            let available = self
                .fed
                .checked_add(self.pending.len())
                .ok_or(MediaError::TooLarge("speech chunk samples"))?;
            if available == 0 {
                return Ok(());
            }
            self.ensure_chunk()?;
            let decision = self
                .plan
                .target
                .saturating_add(self.plan.overlap_samples)
                .saturating_add(1);
            if available < decision {
                let limit = self
                    .plan
                    .confirmed
                    .min(available.saturating_sub(self.plan.overlap_samples + 1));
                self.feed_to(limit)?;
                if !finalizing {
                    return Ok(());
                }
                if available <= self.plan.target {
                    self.feed_to(available)?;
                    self.commit(available, available, None)?;
                    return Ok(());
                }
            }
            let hi = self
                .plan
                .target
                .min(available.saturating_sub(self.plan.overlap_samples + 1));
            let lo = hi
                .saturating_sub(self.plan.search)
                .max(self.plan.overlap_samples + 1);
            let split = choose_split(&self.pending, self.fed, lo, hi)?;
            let tail = if split < available {
                let start = split - self.plan.overlap_samples - self.fed;
                let end = split - self.fed;
                Some(
                    self.pending
                        .get(start..end)
                        .ok_or(MediaError::Invalid("speech overlap bounds"))?
                        .to_vec(),
                )
            } else {
                None
            };
            self.feed_to(split)?;
            self.commit(split, available, tail)?;
        }
    }

    fn ensure_chunk(&mut self) -> Result<()> {
        if self.current_start.is_some() {
            return Ok(());
        }
        let start = self
            .pending_start
            .ok_or(MediaError::Invalid("speech chunk start"))?;
        let overlap = if self.sink.count() == 0 {
            Tick::ZERO
        } else {
            self.plan.overlap_ticks
        };
        self.sink.begin(overlap);
        self.current_start = Some(start);
        Ok(())
    }

    fn feed_to(&mut self, limit: usize) -> Result<()> {
        if limit <= self.fed {
            return Ok(());
        }
        let count = limit - self.fed;
        let samples = self
            .pending
            .get(..count)
            .ok_or(MediaError::Invalid("speech feed bounds"))?;
        self.sink.feed(samples)?;
        self.fed = limit;
        if let Some(start) = self.pending_start {
            self.pending_start = Some(
                start
                    .checked_add(samples_to_ticks(count as i64)?)
                    .ok_or(MediaError::TooLarge("speech timestamp"))?,
            );
        }
        self.pending.drain(..count);
        Ok(())
    }

    fn commit(&mut self, split: usize, available: usize, tail: Option<Vec<f32>>) -> Result<()> {
        let start = self
            .current_start
            .take()
            .ok_or(MediaError::Invalid("speech chunk start"))?;
        let end = start
            .checked_add(samples_to_ticks(split as i64)?)
            .ok_or(MediaError::TooLarge("speech chunk range"))?;
        self.sink.commit(start, end)?;
        if split < available {
            let tail = tail.ok_or(MediaError::Invalid("speech overlap state"))?;
            let mut pending = Vec::with_capacity(tail.len() + self.pending.len());
            pending.extend_from_slice(&tail);
            pending.extend_from_slice(&self.pending);
            self.pending = pending;
            self.pending_start = Some(
                start
                    .checked_add(samples_to_ticks(
                        (split - self.plan.overlap_samples) as i64,
                    )?)
                    .ok_or(MediaError::TooLarge("speech timestamp"))?,
            );
        } else {
            self.pending.clear();
            self.pending_start = None;
        }
        self.fed = 0;
        Ok(())
    }
}

/// Pick the lowest-energy 20 ms frame inside `[lo, hi)`; the latest frame wins a
/// tie, which keeps the split close to the duration target.
fn choose_split(pending: &[f32], fed: usize, lo: usize, hi: usize) -> Result<usize> {
    let start = lo.saturating_sub(fed);
    let end = hi.saturating_sub(fed).min(pending.len());
    if end <= start {
        return Ok(hi);
    }
    let mut best = hi;
    let mut best_energy = f64::INFINITY;
    let mut position = start;
    while position < end {
        let frame_end = position.saturating_add(ENERGY_FRAME_SAMPLES).min(end);
        let frame = pending
            .get(position..frame_end)
            .ok_or(MediaError::Invalid("speech energy bounds"))?;
        let energy = mean_square(frame);
        if energy <= best_energy {
            best_energy = energy;
            let middle = position + (frame_end - position) / 2;
            best = (fed + middle).max(lo).min(hi);
        }
        position = frame_end;
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::{Plan, SpeechExtractSettings, choose_split};
    use time::{Rounding, Tick};

    fn seconds(value: i64) -> Tick {
        Tick::from_units(value, 1, Rounding::Nearest).unwrap()
    }

    #[test]
    fn plan_bounds_every_chunk_and_keeps_room_for_overlap() {
        let settings = SpeechExtractSettings {
            max_duration: seconds(30),
            overlap: seconds(2),
        };
        let plan = Plan::new(&settings).unwrap();
        assert!(plan.target <= 30 * 16_000);
        assert!(plan.target > plan.overlap_samples);
        assert!(plan.target > plan.search + plan.overlap_samples * 2 + 1);
    }

    #[test]
    fn plan_rejects_unusable_bounds() {
        let small = SpeechExtractSettings {
            overlap: seconds(2),
            max_duration: seconds(4),
        };
        assert!(Plan::new(&small).is_err());
        let empty = SpeechExtractSettings {
            max_duration: Tick::ZERO,
            ..SpeechExtractSettings::default()
        };
        assert!(Plan::new(&empty).is_err());
        let negative = SpeechExtractSettings {
            overlap: Tick(-1),
            ..SpeechExtractSettings::default()
        };
        assert!(Plan::new(&negative).is_err());
    }

    #[test]
    fn split_chooses_the_lowest_energy_frame_near_the_target() {
        let mut pending = vec![0.5f32; 16_000];
        for sample in pending.get_mut(9_600..10_000).unwrap() {
            *sample = 0.0;
        }
        let split = choose_split(&pending, 0, 8_000, 12_000).unwrap();
        assert!((9_600..=10_000).contains(&split), "split {split}");
    }
}
