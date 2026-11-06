//! Measures bounded mono speech energy over a source range.

use crate::{MediaError, Result, SourceAsset};
use std::sync::{Arc, atomic::AtomicBool};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

use super::extract::{ENERGY_FRAME_SAMPLES, decode_speech, mean_square, samples_to_ticks};

/// Measured mono 16 kHz energy over at most 20 ms, in source coordinates.
/// RMS is an amplitude ratio; this is an energy hint, not speech recognition.
#[derive(Clone, Copy, Debug)]
pub struct SpeechEnergyWindow {
    pub range: SourceRange,
    pub rms: f64,
}

/// Stream the same decoded/resampled samples used by speech extraction.
/// Returns at most 360,001 windows (two hours); longer requests are refused.
/// Checks cancellation between packets/frames; no files are written.
pub fn analyze_speech_energy(
    asset: &SourceAsset,
    range: SourceRange,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<SpeechEnergyWindow>> {
    if range.start() < Tick::ZERO || range.duration() > Tick(7200 * TICKS_PER_SECOND) {
        return Err(MediaError::TooLarge("speech energy duration"));
    }
    let mut windows = Vec::new();
    let mut pending = Vec::with_capacity(ENERGY_FRAME_SAMPLES);
    let mut start = None;
    let mut expected = None;
    decode_speech(asset, range, &cancel, &mut |mut samples, mut tick| {
        if expected.is_some_and(|end| end != tick) {
            return Err(MediaError::Invalid("speech energy clock"));
        }
        expected = Some(
            tick.checked_add(samples_to_ticks(samples.len() as i64)?)
                .ok_or(MediaError::TooLarge("speech energy clock"))?,
        );
        while !samples.is_empty() {
            start.get_or_insert(tick);
            let count = (ENERGY_FRAME_SAMPLES - pending.len()).min(samples.len());
            pending.extend_from_slice(
                samples
                    .get(..count)
                    .ok_or(MediaError::Invalid("speech energy bounds"))?,
            );
            tick = tick
                .checked_add(samples_to_ticks(count as i64)?)
                .ok_or(MediaError::TooLarge("speech energy clock"))?;
            samples = samples
                .get(count..)
                .ok_or(MediaError::Invalid("speech energy bounds"))?;
            if pending.len() == ENERGY_FRAME_SAMPLES {
                append_energy(&mut windows, &mut pending, &mut start)?;
            }
        }
        Ok(())
    })?;
    if !pending.is_empty() {
        append_energy(&mut windows, &mut pending, &mut start)?;
    }
    Ok(windows)
}

fn append_energy(
    windows: &mut Vec<SpeechEnergyWindow>,
    samples: &mut Vec<f32>,
    start: &mut Option<Tick>,
) -> Result<()> {
    if windows.len() >= 360_001 {
        return Err(MediaError::TooLarge("speech energy windows"));
    }
    let start = start
        .take()
        .ok_or(MediaError::Invalid("speech energy start"))?;
    let end = start
        .checked_add(samples_to_ticks(samples.len() as i64)?)
        .ok_or(MediaError::TooLarge("speech energy clock"))?;
    let rms = mean_square(samples).sqrt();
    if !rms.is_finite() {
        return Err(MediaError::Invalid("speech energy samples"));
    }
    windows.push(SpeechEnergyWindow {
        range: SourceRange::new(start, end)?,
        rms,
    });
    samples.clear();
    Ok(())
}
