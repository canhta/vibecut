//! Streams a source audio range as mono 16 kHz PCM for speech consumers.

use crate::{
    AudioDecoder, AudioFrame, AudioResampler, AudioSampleFormat, MediaError, PacketReader, Result,
    SourceAsset, StreamKind, init,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

/// Mono speech sample rate produced by the speech consumers.
pub const SPEECH_SAMPLE_RATE: u32 = 16_000;
pub(super) const TICKS_PER_SAMPLE: i64 = TICKS_PER_SECOND / SPEECH_SAMPLE_RATE as i64;
pub(super) const ENERGY_FRAME_SAMPLES: usize = 320;
const PREROLL: Tick = Tick(TICKS_PER_SECOND);

pub(super) fn mean_square(samples: &[f32]) -> f64 {
    samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64
}

pub(super) fn decode_speech(
    asset: &SourceAsset,
    range: SourceRange,
    cancel: &Arc<AtomicBool>,
    push: &mut impl FnMut(&[f32], Tick) -> Result<()>,
) -> Result<()> {
    init()?;
    if !asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Audio)
    {
        return Err(MediaError::NoAudioStream);
    }
    let mut reader = PacketReader::open_with_cancel(asset.path(), cancel.clone())?;
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Audio)
        .cloned()
        .ok_or(MediaError::NoAudioStream)?;
    let end = match stream.duration {
        Some(duration) => stream
            .start
            .unwrap_or(Tick::ZERO)
            .checked_add(duration)
            .ok_or(MediaError::TooLarge("audio stream duration"))?,
        None => range.end(),
    }
    .min(range.end());
    if end <= range.start() {
        return Err(MediaError::Invalid("audio range outside stream"));
    }
    let range = SourceRange::new(range.start(), end)?;
    let mut decoder = AudioDecoder::new(&stream)?;
    if range.start() > Tick::ZERO {
        let target = range.start().saturating_sub(PREROLL);
        if reader.seek(target).is_err() {
            reader = PacketReader::open_with_cancel(asset.path(), cancel.clone())?;
        }
    }
    let mut state = DecodeState::default();
    let mut reached_end = false;
    'decode: while !reached_end {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        match reader.next_packet()? {
            Some(packet) if packet.stream() == stream.index => {
                for frame in decoder.decode(&packet)? {
                    if decode_frame(frame, &mut state, push, range, cancel)? {
                        reached_end = true;
                        break 'decode;
                    }
                }
            }
            Some(_) => {}
            None => {
                for frame in decoder.flush()? {
                    if decode_frame(frame, &mut state, push, range, cancel)? {
                        reached_end = true;
                        break;
                    }
                }
                break;
            }
        }
    }
    if !reached_end {
        let mut resampler = state.resampler.take();
        if let Some(resampler) = resampler.as_mut() {
            while let Some(frame) = resampler.flush()? {
                if cancel.load(Ordering::Relaxed) {
                    return Err(MediaError::Cancelled);
                }
                if push_resampled(frame, &mut state, push, range)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct DecodeState {
    resampler: Option<AudioResampler>,
    anchor: Option<Tick>,
    produced: i64,
}

fn decode_frame(
    frame: AudioFrame,
    state: &mut DecodeState,
    push: &mut impl FnMut(&[f32], Tick) -> Result<()>,
    range: SourceRange,
    cancel: &AtomicBool,
) -> Result<bool> {
    let outputs = match state.resampler.as_mut() {
        Some(resampler) => resampler.convert(&frame)?,
        None => {
            let mut resampler =
                AudioResampler::new(&frame, SPEECH_SAMPLE_RATE, 1, AudioSampleFormat::PlanarF32)?;
            let outputs = resampler.convert(&frame)?;
            state.resampler = Some(resampler);
            outputs
        }
    };
    for output in outputs {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        if push_resampled(output, state, push, range)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn push_resampled(
    output: AudioFrame,
    state: &mut DecodeState,
    push: &mut impl FnMut(&[f32], Tick) -> Result<()>,
    range: SourceRange,
) -> Result<bool> {
    let pts = output.pts()?.ok_or(MediaError::Unsupported(
        "resampled audio timestamp unavailable",
    ))?;
    let anchor = *state.anchor.get_or_insert(pts);
    let expected = anchor
        .checked_add(samples_to_ticks(state.produced)?)
        .ok_or(MediaError::TooLarge("resampled audio timestamp"))?;
    if pts != expected {
        return Err(MediaError::Invalid("resampled audio clock"));
    }
    let mono = output
        .planar_f32()?
        .into_iter()
        .next()
        .ok_or(MediaError::Invalid("resampled audio channels"))?;
    if mono.is_empty() {
        return Ok(false);
    }
    let frame_start = state.produced;
    let frame_end = frame_start
        .checked_add(i64::try_from(mono.len()).map_err(|_| MediaError::TooLarge("audio frame"))?)
        .ok_or(MediaError::TooLarge("resampled audio sample count"))?;
    let first = ceil_samples(range.start().0, anchor.0);
    let end = ceil_samples(range.end().0, anchor.0);
    let keep_start = first.max(frame_start);
    let keep_end = end.min(frame_end);
    state.produced = frame_end;
    if keep_end > keep_start {
        let offset_start = usize::try_from(keep_start - frame_start)
            .map_err(|_| MediaError::TooLarge("speech sample bounds"))?;
        let offset_end = usize::try_from(keep_end - frame_start)
            .map_err(|_| MediaError::TooLarge("speech sample bounds"))?;
        let samples = mono
            .get(offset_start..offset_end)
            .ok_or(MediaError::Invalid("speech sample bounds"))?;
        let tick = anchor
            .checked_add(samples_to_ticks(keep_start)?)
            .ok_or(MediaError::TooLarge("speech timestamp"))?;
        push(samples, tick)?;
    }
    Ok(end <= frame_end)
}

/// First output sample index whose source tick is at or after `tick`.
fn ceil_samples(tick: i64, anchor: i64) -> i64 {
    let delta = i128::from(tick) - i128::from(anchor);
    let step = i128::from(TICKS_PER_SAMPLE);
    let value = delta.div_euclid(step) + i128::from(delta.rem_euclid(step) != 0);
    i64::try_from(value).unwrap_or(if value.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

pub(super) fn samples_to_ticks(samples: i64) -> Result<Tick> {
    Ok(Tick::from_units(
        samples,
        i64::from(SPEECH_SAMPLE_RATE),
        Rounding::Nearest,
    )?)
}
