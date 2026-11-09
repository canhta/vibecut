//! Computes a bounded min/max peak summary of one source's decoded audio.

use crate::{
    AudioDecoder, AudioResampler, AudioSampleFormat, MediaError, PacketReader, Result, SourceAsset,
    StreamKind, init,
};
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use time::{Rounding, SourceRange, Tick};

/// Samples per peak bucket at the source rate. The implementation reference
/// builds its timeline waveform with 128-sample buckets
/// (`media/waveform-summary.ts:7`, `DEFAULT_SOURCE_WAVEFORM_BUCKET_SIZE`).
pub const PEAK_BUCKET_SAMPLES: u32 = 128;

/// Bound for one encoded summary. At 48 kHz stereo this covers about 3.1
/// hours of 128-sample buckets (16 bytes each).
pub const PEAK_SUMMARY_MAX_BYTES: usize = 64 << 20;

const MAGIC: &[u8; 8] = b"VCWAVE01";
const HEADER: usize = 8 + 4 + 2 + 4 + 8 + 8;
const MAX_CHANNELS: u16 = 2;

/// Lowest and highest sample in one bucket of one channel, linear amplitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BucketPeak {
    pub min: f32,
    pub max: f32,
}

/// Bucket-aligned min/max peaks for every channel of one decoded source.
///
/// Sample zero sits at `anchor` in source ticks and every bucket covers
/// `PEAK_BUCKET_SAMPLES` consecutive samples at `sample_rate`. Values are the
/// decoded linear amplitudes; a bucket is over 0 dBFS when any value exceeds
/// `[-1, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct PeakSummary {
    anchor: Tick,
    sample_rate: u32,
    channels: u16,
    total_samples: u64,
    /// Bucket-major: per bucket each channel's minimum then maximum.
    values: Vec<f32>,
}

impl PeakSummary {
    pub fn anchor(&self) -> Tick {
        self.anchor
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn bucket_samples(&self) -> u32 {
        PEAK_BUCKET_SAMPLES
    }

    pub fn bucket_count(&self) -> usize {
        let stride = usize::from(self.channels) * 2;
        self.values.len() / stride.max(1)
    }

    /// One channel's peaks in `bucket`, or `None` past its bounds.
    pub fn peak(&self, bucket: usize, channel: u16) -> Option<BucketPeak> {
        if channel >= self.channels {
            return None;
        }
        let stride = usize::from(self.channels).checked_mul(2)?;
        let base = bucket
            .checked_mul(stride)?
            .checked_add(usize::from(channel).checked_mul(2)?)?;
        Some(BucketPeak {
            min: *self.values.get(base)?,
            max: *self.values.get(base.checked_add(1)?)?,
        })
    }

    /// Whether any channel in `bucket` leaves `[-1, 1]`, exceeding 0 dBFS.
    pub fn over(&self, bucket: usize) -> bool {
        (0..self.channels).any(|channel| {
            self.peak(bucket, channel)
                .is_some_and(|peak| peak.min < -1.0 || peak.max > 1.0)
        })
    }

    /// Half-open bucket span covering `range`, clamped to the decoded audio.
    /// `None` when the range lies outside the summary.
    pub fn bucket_span(&self, range: SourceRange) -> Option<Range<usize>> {
        if self.total_samples == 0 {
            return None;
        }
        let start = self.sample_index(range.start(), Rounding::Floor).max(0);
        let end = self.sample_index(range.end(), Rounding::Ceil).max(0);
        let start = u64::try_from(start).ok()?;
        let end = u64::try_from(end).ok()?;
        if start >= self.total_samples || end <= start {
            return None;
        }
        let end = end.min(self.total_samples);
        let bucket = u64::from(PEAK_BUCKET_SAMPLES);
        let first = usize::try_from(start / bucket).ok()?;
        let last = usize::try_from(end.div_ceil(bucket)).ok()?;
        (last > first).then_some(first..last)
    }

    /// Source ticks covered by `bucket`, with a final partial bucket trimmed
    /// to the decoded sample count.
    pub fn bucket_source_range(&self, bucket: usize) -> Option<SourceRange> {
        if bucket >= self.bucket_count() {
            return None;
        }
        let bucket = bucket as u64;
        let samples = u64::from(PEAK_BUCKET_SAMPLES);
        let start = bucket.checked_mul(samples)?;
        let end = start.checked_add(samples)?.min(self.total_samples);
        let ticks = |samples: u64| {
            Tick::from_units(
                i64::try_from(samples).ok()?,
                i64::from(self.sample_rate),
                Rounding::Nearest,
            )
            .ok()
            .and_then(|time| self.anchor.checked_add(time))
        };
        SourceRange::new(ticks(start)?, ticks(end)?).ok()
    }

    fn sample_index(&self, tick: Tick, rounding: Rounding) -> i64 {
        let Some(delta) = tick.checked_sub(self.anchor) else {
            return if tick.0 < self.anchor.0 {
                i64::MIN
            } else {
                i64::MAX
            };
        };
        delta
            .to_units(i64::from(self.sample_rate), rounding)
            .unwrap_or(if delta.0 < 0 { i64::MIN } else { i64::MAX })
    }

    /// Compact binary form for the engine artifact cache.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let stride = usize::from(self.channels)
            .checked_mul(2)
            .ok_or(MediaError::TooLarge("waveform summary"))?;
        let values = self
            .values
            .len()
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or(MediaError::TooLarge("waveform summary"))?;
        let bytes = HEADER
            .checked_add(values)
            .ok_or(MediaError::TooLarge("waveform summary"))?;
        if bytes > PEAK_SUMMARY_MAX_BYTES || !self.values.len().is_multiple_of(stride) {
            return Err(MediaError::TooLarge("waveform summary"));
        }
        let mut out = Vec::with_capacity(bytes);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&PEAK_BUCKET_SAMPLES.to_le_bytes());
        out.extend_from_slice(&self.anchor.0.to_le_bytes());
        out.extend_from_slice(&self.total_samples.to_le_bytes());
        for value in &self.values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        Ok(out)
    }

    /// Validate untrusted cached bytes. Any malformed field rejects the whole
    /// summary; callers treat that as a cache miss, never as zero peaks.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > PEAK_SUMMARY_MAX_BYTES || bytes.get(..8) != Some(MAGIC) {
            return Err(MediaError::Invalid("waveform summary header"));
        }
        let sample_rate = u32::from_le_bytes(field(bytes, 8)?);
        let channels = u16::from_le_bytes(field(bytes, 12)?);
        let bucket_samples = u32::from_le_bytes(field(bytes, 14)?);
        let anchor = Tick(i64::from_le_bytes(field(bytes, 18)?));
        let total_samples = u64::from_le_bytes(field(bytes, 26)?);
        if !(8_000..=192_000).contains(&sample_rate)
            || channels == 0
            || channels > MAX_CHANNELS
            || bucket_samples != PEAK_BUCKET_SAMPLES
            || total_samples == 0
        {
            return Err(MediaError::Invalid("waveform summary format"));
        }
        let stride = usize::from(channels) * 2;
        let payload = bytes.len().checked_sub(HEADER).ok_or_else(summary)?;
        if payload % (stride * std::mem::size_of::<f32>()) != 0 {
            return Err(MediaError::Invalid("waveform summary size"));
        }
        let count = payload / (stride * std::mem::size_of::<f32>());
        if u64::try_from(count).map_err(|_| summary())?
            != total_samples.div_ceil(u64::from(PEAK_BUCKET_SAMPLES))
        {
            return Err(MediaError::Invalid("waveform summary buckets"));
        }
        let mut values = Vec::with_capacity(count * stride);
        let (chunks, remainder) = bytes.get(HEADER..).unwrap_or_default().as_chunks::<4>();
        if !remainder.is_empty() {
            return Err(MediaError::Invalid("waveform summary size"));
        }
        for chunk in chunks {
            let value = f32::from_le_bytes(*chunk);
            if !value.is_finite() {
                return Err(MediaError::Invalid("waveform summary values"));
            }
            values.push(value);
        }
        let summary = Self {
            anchor,
            sample_rate,
            channels,
            total_samples,
            values,
        };
        if (0..summary.bucket_count()).any(|bucket| {
            (0..channels).any(|channel| summary.peak(bucket, channel).is_none_or(|p| p.min > p.max))
        }) {
            return Err(MediaError::Invalid("waveform summary bounds"));
        }
        Ok(summary)
    }
}

fn summary() -> MediaError {
    MediaError::TooLarge("waveform summary")
}

fn invalid(_error: std::array::TryFromSliceError) -> MediaError {
    MediaError::Invalid("waveform summary field")
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    let end = at.checked_add(N).ok_or_else(summary)?;
    bytes
        .get(at..end)
        .ok_or_else(summary)?
        .try_into()
        .map_err(invalid)
}

/// Decode one source's audio stream and summarize it into 128-sample buckets.
///
/// Streams one frame at a time, so memory follows the bounded summary rather
/// than the source duration. Checks cancellation between packets and frames.
/// Mono and stereo are admitted; other layouts are `Unsupported`. A source
/// without audio is `NoAudioStream`.
pub fn peak_summary(asset: &SourceAsset, cancel: Arc<AtomicBool>) -> Result<PeakSummary> {
    init()?;
    let mut reader = PacketReader::open_with_cancel(asset.path(), cancel.clone())?;
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Audio)
        .cloned()
        .ok_or(MediaError::NoAudioStream)?;
    let info = stream.audio.clone().ok_or(MediaError::NoAudioStream)?;
    if !(8_000..=192_000).contains(&info.sample_rate)
        || info.channels == 0
        || info.channels > MAX_CHANNELS
    {
        return Err(MediaError::Unsupported("waveform audio layout"));
    }
    let mut decoder = AudioDecoder::new(&stream)?;
    let mut accumulator = Accumulator::new(info.sample_rate, info.channels);
    let mut resampler = None;
    if let Some(duration) = stream.duration {
        accumulator.reserve(duration)?;
    }
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        match reader.next_packet()? {
            Some(packet) if packet.stream() == stream.index => {
                for frame in decoder.decode(&packet)? {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(MediaError::Cancelled);
                    }
                    push_frame(&mut accumulator, &mut resampler, &frame, &cancel)?;
                }
            }
            Some(_) => {}
            None => {
                for frame in decoder.flush()? {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(MediaError::Cancelled);
                    }
                    push_frame(&mut accumulator, &mut resampler, &frame, &cancel)?;
                }
                break;
            }
        }
    }
    if let Some(resampler) = resampler.as_mut() {
        while let Some(output) = resampler.flush()? {
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            accumulator.push(&output)?;
        }
    }
    accumulator.finish()
}

/// Convert every decoded frame to the summary's planar f32 layout through the
/// existing explicit resampler owner; the rate and channel count stay equal.
fn push_frame(
    accumulator: &mut Accumulator,
    resampler: &mut Option<AudioResampler>,
    frame: &crate::AudioFrame,
    cancel: &AtomicBool,
) -> Result<()> {
    let outputs = match resampler {
        Some(resampler) => resampler.convert(frame)?,
        None => {
            let mut started = AudioResampler::new(
                frame,
                accumulator.sample_rate,
                accumulator.channels,
                AudioSampleFormat::PlanarF32,
            )?;
            let outputs = started.convert(frame)?;
            *resampler = Some(started);
            outputs
        }
    };
    for output in outputs {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        accumulator.push(&output)?;
    }
    Ok(())
}

struct Accumulator {
    anchor: Option<Tick>,
    sample_rate: u32,
    channels: u16,
    total_samples: u64,
    values: Vec<f32>,
}

impl Accumulator {
    fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            anchor: None,
            sample_rate,
            channels,
            total_samples: 0,
            values: Vec::new(),
        }
    }

    fn stride(&self) -> usize {
        usize::from(self.channels) * 2
    }

    /// Pre-reserve a bounded summary from the declared duration. The decoded
    /// count still defines the result; this only avoids growth copies.
    fn reserve(&mut self, duration: Tick) -> Result<()> {
        let samples = duration.to_units(i64::from(self.sample_rate), Rounding::Ceil)?;
        let samples = u64::try_from(samples).map_err(|_| summary())?;
        let buckets = samples
            .div_ceil(u64::from(PEAK_BUCKET_SAMPLES))
            .checked_add(1)
            .ok_or_else(summary)?;
        let values = buckets
            .checked_mul(self.stride() as u64)
            .ok_or_else(summary)?;
        if values
            .checked_mul(std::mem::size_of::<f32>() as u64)
            .is_none_or(|bytes| bytes > PEAK_SUMMARY_MAX_BYTES as u64)
        {
            return Err(summary());
        }
        self.values
            .reserve(usize::try_from(values).map_err(|_| summary())?);
        Ok(())
    }

    fn push(&mut self, frame: &crate::AudioFrame) -> Result<()> {
        if frame.sample_rate() != self.sample_rate || frame.channels() != self.channels {
            return Err(MediaError::Invalid("waveform audio layout changed"));
        }
        let pts = frame
            .pts()?
            .ok_or(MediaError::Unsupported("waveform requires source PTS"))?;
        let anchor = *self.anchor.get_or_insert(pts);
        let expected = anchor
            .checked_add(Tick::from_units(
                i64::try_from(self.total_samples).map_err(|_| summary())?,
                i64::from(self.sample_rate),
                Rounding::Nearest,
            )?)
            .ok_or(MediaError::TooLarge("waveform sample clock"))?;
        if pts != expected {
            return Err(MediaError::Invalid("waveform sample clock"));
        }
        let planes = frame.planar_f32()?;
        let samples = planes.first().map_or(0, Vec::len);
        if samples == 0 {
            return Ok(());
        }
        if planes.iter().any(|plane| plane.len() != samples) {
            return Err(MediaError::Invalid("waveform channels"));
        }
        let samples = u64::try_from(samples).map_err(|_| summary())?;
        let base = self.total_samples;
        let mut offset = 0u64;
        while offset < samples {
            let position = base.checked_add(offset).ok_or_else(summary)?;
            let bucket = position / u64::from(PEAK_BUCKET_SAMPLES);
            let bucket_start = bucket
                .checked_mul(u64::from(PEAK_BUCKET_SAMPLES))
                .ok_or_else(summary)?;
            let in_bucket = position.checked_sub(bucket_start).ok_or_else(summary)?;
            let count = (u64::from(PEAK_BUCKET_SAMPLES) - in_bucket).min(samples - offset);
            let bucket_index = usize::try_from(bucket).map_err(|_| summary())?;
            while self.values.len() / self.stride() <= bucket_index {
                for _ in 0..self.channels {
                    self.values.push(f32::INFINITY);
                    self.values.push(f32::NEG_INFINITY);
                }
                if self.values.len() * std::mem::size_of::<f32>() > PEAK_SUMMARY_MAX_BYTES {
                    return Err(summary());
                }
            }
            let start = usize::try_from(offset).map_err(|_| summary())?;
            let end = usize::try_from(offset.checked_add(count).ok_or_else(summary)?)
                .map_err(|_| summary())?;
            for (channel, plane) in planes.iter().enumerate() {
                let samples = plane
                    .get(start..end)
                    .ok_or(MediaError::Invalid("waveform bounds"))?;
                let base = bucket_index
                    .checked_mul(self.stride())
                    .and_then(|base| base.checked_add(channel.checked_mul(2)?))
                    .ok_or_else(summary)?;
                let mut min = *self
                    .values
                    .get(base)
                    .ok_or(MediaError::Invalid("waveform bounds"))?;
                let mut max = *self
                    .values
                    .get(base.checked_add(1).ok_or_else(summary)?)
                    .ok_or(MediaError::Invalid("waveform bounds"))?;
                for value in samples {
                    if !value.is_finite() {
                        return Err(MediaError::Invalid("waveform samples"));
                    }
                    min = min.min(*value);
                    max = max.max(*value);
                }
                *self
                    .values
                    .get_mut(base)
                    .ok_or(MediaError::Invalid("waveform bounds"))? = min;
                *self
                    .values
                    .get_mut(base.checked_add(1).ok_or_else(summary)?)
                    .ok_or(MediaError::Invalid("waveform bounds"))? = max;
            }
            offset = offset.checked_add(count).ok_or_else(summary)?;
        }
        self.total_samples = self
            .total_samples
            .checked_add(samples)
            .ok_or_else(summary)?;
        Ok(())
    }

    fn finish(self) -> Result<PeakSummary> {
        let anchor = self
            .anchor
            .ok_or(MediaError::Unsupported("waveform has no samples"))?;
        if self.total_samples == 0 {
            return Err(MediaError::Unsupported("waveform has no samples"));
        }
        if self.values.iter().any(|value| !value.is_finite()) {
            return Err(MediaError::Invalid("waveform samples"));
        }
        Ok(PeakSummary {
            anchor,
            sample_rate: self.sample_rate,
            channels: self.channels,
            total_samples: self.total_samples,
            values: self.values,
        })
    }
}
