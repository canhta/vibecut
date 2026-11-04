//! Speech-audio preparation: decode a source range, resample to mono 16 kHz and
//! write AAC-LC m4a chunks bounded by upload bytes and duration.
//!
//! Boundaries are quantized to the 16 kHz output sample grid, so reported source
//! ranges are exact ticks. Consecutive chunks share the configured overlap; each
//! file uses chunk-local presentation timestamps starting at zero. The decoder
//! reads one packet at a time and only a few seconds of PCM stay buffered.

use crate::{
    AudioDecoder, AudioEncoder, AudioEncoderSettings, AudioFrame, AudioResampler,
    AudioSampleFormat, Container, MediaError, Muxer, PacketReader, Result, SourceAsset, StreamKind,
    init,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

/// Mono speech sample rate produced by [`extract_speech_audio`].
pub const SPEECH_SAMPLE_RATE: u32 = 16_000;
const TICKS_PER_SAMPLE: i64 = TICKS_PER_SECOND / SPEECH_SAMPLE_RATE as i64;
const ENCODE_FRAME_SAMPLES: usize = 16_384;
const ENERGY_FRAME_SAMPLES: usize = 320;
const SILENCE_RMS: f64 = 1e-4;
const DEFAULT_MAX_BYTES: u64 = 24_000_000;
const BYTE_HEADROOM: u64 = 64 << 10;
const SIZE_MARGIN_DIVISOR: usize = 20;
const SEARCH_SAMPLES: usize = 2 * SPEECH_SAMPLE_RATE as usize;
const PREROLL: Tick = Tick(TICKS_PER_SECOND);
const DEFAULT_BIT_RATE: usize = 48_000;

/// One uploaded speech file and the exact source interval it represents.
#[derive(Clone, Debug)]
pub struct AudioChunk {
    /// Finished m4a path inside the requested output directory.
    pub path: PathBuf,
    /// Exact source ticks decoded into this file.
    pub source_range: SourceRange,
    /// Ticks shared with the preceding chunk; zero for the first chunk.
    pub overlap: Tick,
    /// Energy hint: false when the chunk is effectively silent, not a VAD result.
    pub speech_detected: bool,
}

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

fn mean_square(samples: &[f32]) -> f64 {
    samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64
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

/// Bounds for [`extract_speech_audio_with`]. The OpenAI transcription endpoint
/// accepts at most 25 MB per file, so the default keeps one megabyte of headroom.
#[derive(Clone, Debug)]
pub struct SpeechExtractSettings {
    /// Hard upper bound for one finished file.
    pub max_bytes: u64,
    /// Hard upper bound for one chunk's audio duration.
    pub max_duration: Tick,
    /// Audio repeated at the start of the next chunk.
    pub overlap: Tick,
    /// AAC-LC bit rate in bits per second.
    pub bit_rate: usize,
}
impl Default for SpeechExtractSettings {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_duration: Tick(600 * TICKS_PER_SECOND),
            overlap: Tick(2 * TICKS_PER_SECOND),
            bit_rate: DEFAULT_BIT_RATE,
        }
    }
}

/// Extract a source range into default settings: 24 MB, 10 minutes, 2 s overlap.
pub fn extract_speech_audio(
    asset: &SourceAsset,
    range: SourceRange,
    out_dir: impl AsRef<Path>,
) -> Result<Vec<AudioChunk>> {
    extract_speech_audio_with(asset, range, out_dir, &SpeechExtractSettings::default())
}

/// Extract with explicit bounds. Existing `speech-NNNN.m4a` files are replaced.
pub fn extract_speech_audio_with(
    asset: &SourceAsset,
    range: SourceRange,
    out_dir: impl AsRef<Path>,
    settings: &SpeechExtractSettings,
) -> Result<Vec<AudioChunk>> {
    extract_files(
        asset,
        range,
        out_dir.as_ref(),
        settings,
        Arc::new(AtomicBool::new(false)),
    )
}

/// Extract with a cancellation token checked between packets and resampled frames.
pub fn extract_speech_audio_with_cancel(
    asset: &SourceAsset,
    range: SourceRange,
    out_dir: impl AsRef<Path>,
    settings: &SpeechExtractSettings,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<AudioChunk>> {
    extract_files(asset, range, out_dir.as_ref(), settings, cancel)
}

/// Stream bounded mono 16 kHz PCM windows in the same speech-safe boundaries as
/// [`extract_speech_audio`], without producing an encoded file. Each callback runs
/// before decoding continues; at most one window of PCM is retained by this crate.
pub fn extract_speech_windows_with_cancel(
    asset: &SourceAsset,
    range: SourceRange,
    settings: &SpeechExtractSettings,
    cancel: Arc<AtomicBool>,
    window: impl FnMut(SpeechWindow) -> Result<()>,
) -> Result<()> {
    let plan = Plan::new(settings)?;
    let mut sink = PcmSink {
        start: None,
        overlap: Tick::ZERO,
        samples: Vec::new(),
        energy: 0.0,
        sample_count: 0,
        windows: 0,
        window,
    };
    extract_windows(asset, range, plan, cancel, &mut sink)
}

fn extract_files(
    asset: &SourceAsset,
    range: SourceRange,
    out_dir: &Path,
    settings: &SpeechExtractSettings,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<AudioChunk>> {
    let plan = Plan::new(settings)?;
    fs::create_dir_all(out_dir)?;
    let mut sink = FileSink {
        out_dir,
        settings,
        current: None,
        chunks: Vec::new(),
    };
    extract_windows(asset, range, plan, cancel, &mut sink)?;
    Ok(sink.chunks)
}

fn extract_windows(
    asset: &SourceAsset,
    range: SourceRange,
    plan: Plan,
    cancel: Arc<AtomicBool>,
    sink: &mut dyn Sink,
) -> Result<()> {
    let mut chunker = Chunker::new(plan, sink);
    decode_speech(asset, range, &cancel, &mut |samples, tick| {
        chunker.push(samples, tick)
    })?;
    chunker.finish()
}

pub(crate) fn decode_speech(
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

fn samples_to_ticks(samples: i64) -> Result<Tick> {
    Ok(Tick::from_units(
        samples,
        i64::from(SPEECH_SAMPLE_RATE),
        Rounding::Nearest,
    )?)
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
        if settings.max_bytes <= BYTE_HEADROOM + 4096
            || !(16_000..=128_000).contains(&settings.bit_rate)
            || settings.max_duration <= Tick::ZERO
            || settings.overlap < Tick::ZERO
        {
            return Err(MediaError::Invalid("speech extract settings"));
        }
        let duration_samples = usize::try_from(
            settings
                .max_duration
                .to_units(i64::from(SPEECH_SAMPLE_RATE), Rounding::Floor)?,
        )
        .map_err(|_| MediaError::Invalid("speech chunk duration"))?;
        let usable = settings.max_bytes - BYTE_HEADROOM;
        let byte_seconds = usable.saturating_mul(8) / settings.bit_rate as u64;
        let byte_samples =
            usize::try_from(byte_seconds.saturating_mul(u64::from(SPEECH_SAMPLE_RATE)))
                .unwrap_or(usize::MAX);
        let byte_samples = byte_samples - byte_samples / SIZE_MARGIN_DIVISOR;
        let target = duration_samples.min(byte_samples);
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

/// Receives bounded windows so file and PCM consumers share one split policy.
trait Sink {
    fn begin(&mut self, index: usize, start: Tick, overlap: Tick) -> Result<()>;
    fn feed(&mut self, samples: &[f32]) -> Result<()>;
    fn commit(&mut self, start: Tick, end: Tick) -> Result<()>;
    fn count(&self) -> usize;
}

struct FileSink<'a> {
    out_dir: &'a Path,
    settings: &'a SpeechExtractSettings,
    current: Option<ChunkWriter>,
    chunks: Vec<AudioChunk>,
}
impl Sink for FileSink<'_> {
    fn begin(&mut self, index: usize, _start: Tick, overlap: Tick) -> Result<()> {
        self.current = Some(ChunkWriter::new(
            index,
            self.out_dir,
            overlap,
            self.settings,
        )?);
        Ok(())
    }
    fn feed(&mut self, samples: &[f32]) -> Result<()> {
        self.current
            .as_mut()
            .ok_or(MediaError::Invalid("speech chunk state"))?
            .feed(samples)
    }
    fn commit(&mut self, start: Tick, end: Tick) -> Result<()> {
        let writer = self
            .current
            .take()
            .ok_or(MediaError::Invalid("speech chunk state"))?;
        let overlap = writer.overlap;
        let (path, speech_detected) = writer.finish(self.settings.max_bytes)?;
        self.chunks.push(AudioChunk {
            path,
            source_range: SourceRange::new(start, end)?,
            overlap,
            speech_detected,
        });
        Ok(())
    }
    fn count(&self) -> usize {
        self.chunks.len()
    }
}

struct PcmSink<F> {
    start: Option<Tick>,
    overlap: Tick,
    samples: Vec<f32>,
    energy: f64,
    sample_count: usize,
    windows: usize,
    window: F,
}
impl<F: FnMut(SpeechWindow) -> Result<()>> Sink for PcmSink<F> {
    fn begin(&mut self, _index: usize, start: Tick, overlap: Tick) -> Result<()> {
        self.start = Some(start);
        self.overlap = overlap;
        self.samples.clear();
        self.energy = 0.0;
        self.sample_count = 0;
        Ok(())
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
        self.start = None;
        self.windows += 1;
        (self.window)(window)
    }
    fn count(&self) -> usize {
        self.windows
    }
}

struct Chunker<'a> {
    plan: Plan,
    pending: Vec<f32>,
    pending_start: Option<Tick>,
    expected_input: Option<Tick>,
    fed: usize,
    current_start: Option<Tick>,
    sink: &'a mut dyn Sink,
    next_index: usize,
}
impl<'a> Chunker<'a> {
    fn new(plan: Plan, sink: &'a mut dyn Sink) -> Self {
        Self {
            plan,
            pending: Vec::new(),
            pending_start: None,
            expected_input: None,
            fed: 0,
            current_start: None,
            sink,
            next_index: 0,
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
        let overlap = if self.next_index == 0 {
            Tick::ZERO
        } else {
            self.plan.overlap_ticks
        };
        self.sink.begin(self.next_index, start, overlap)?;
        self.current_start = Some(start);
        self.next_index += 1;
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

struct ChunkWriter {
    encoder: AudioEncoder,
    muxer: Muxer,
    output: OutputFile,
    overlap: Tick,
    fed: usize,
    energy: f64,
    samples: usize,
    scratch: Vec<f32>,
}
impl ChunkWriter {
    fn new(
        index: usize,
        out_dir: &Path,
        overlap: Tick,
        settings: &SpeechExtractSettings,
    ) -> Result<Self> {
        let encoder = AudioEncoder::new(AudioEncoderSettings {
            sample_rate: SPEECH_SAMPLE_RATE,
            channels: 1,
            bit_rate: settings.bit_rate,
            webm: false,
        })?;
        let output = OutputFile::new(out_dir, index)?;
        let mut muxer = Muxer::create(&output.temp, Container::Mp4)?;
        muxer.add_parameters(&encoder.parameters(), encoder.time_base())?;
        Ok(Self {
            encoder,
            muxer,
            output,
            overlap,
            fed: 0,
            energy: 0.0,
            samples: 0,
            scratch: Vec::new(),
        })
    }

    fn feed(&mut self, samples: &[f32]) -> Result<()> {
        let mut offset = 0;
        while offset < samples.len() {
            let count = (samples.len() - offset).min(ENCODE_FRAME_SAMPLES);
            let slice = samples
                .get(offset..offset + count)
                .ok_or(MediaError::Invalid("speech encode bounds"))?;
            let pts = samples_to_ticks(self.fed as i64)?;
            self.scratch.clear();
            self.scratch.extend_from_slice(slice);
            let frame = AudioFrame::from_planar_f32(
                pts,
                SPEECH_SAMPLE_RATE,
                std::slice::from_ref(&self.scratch),
            )?;
            for packet in self.encoder.encode(&frame)? {
                self.muxer.write_packet(0, &packet)?;
            }
            self.energy += slice
                .iter()
                .map(|sample| f64::from(*sample) * f64::from(*sample))
                .sum::<f64>();
            self.samples += count;
            self.fed = self
                .fed
                .checked_add(count)
                .ok_or(MediaError::TooLarge("speech samples"))?;
            offset += count;
        }
        Ok(())
    }

    fn finish(self, max_bytes: u64) -> Result<(PathBuf, bool)> {
        let Self {
            mut encoder,
            mut muxer,
            output,
            energy,
            samples,
            ..
        } = self;
        for packet in encoder.flush()? {
            muxer.write_packet(0, &packet)?;
        }
        muxer.finish()?;
        if fs::metadata(&output.temp)?.len() > max_bytes {
            return Err(MediaError::TooLarge("encoded speech chunk bytes"));
        }
        let speech_detected = samples > 0 && (energy / samples as f64).sqrt() >= SILENCE_RMS;
        Ok((output.commit()?, speech_detected))
    }
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct OutputFile {
    temp: PathBuf,
    final_path: PathBuf,
    committed: bool,
}
impl OutputFile {
    fn new(dir: &Path, index: usize) -> Result<Self> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            temp: dir.join(format!(
                ".speech-{index:04}-{}-{sequence}.part",
                std::process::id()
            )),
            final_path: dir.join(format!("speech-{index:04}.m4a")),
            committed: false,
        })
    }

    fn commit(mut self) -> Result<PathBuf> {
        // Same-directory rename replaces atomically and preserves the old file on failure.
        fs::rename(&self.temp, &self.final_path)?;
        self.committed = true;
        Ok(self.final_path.clone())
    }
}
impl Drop for OutputFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OutputFile, Plan, SpeechExtractSettings, choose_split};
    use time::{Rounding, Tick};

    fn seconds(value: i64) -> Tick {
        Tick::from_units(value, 1, Rounding::Nearest).unwrap()
    }

    #[test]
    fn failed_publication_preserves_existing_chunk() {
        let directory =
            std::env::temp_dir().join(format!("vibecut-speech-publication-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = OutputFile::new(&directory, 0).unwrap();
        std::fs::write(&output.final_path, b"previous chunk").unwrap();
        let final_path = output.final_path.clone();
        // A staging file lost before publication must not delete an older result.
        assert!(output.commit().is_err());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"previous chunk");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn plan_bounds_every_chunk_and_keeps_room_for_overlap() {
        let settings = SpeechExtractSettings {
            max_duration: seconds(30),
            overlap: seconds(2),
            ..SpeechExtractSettings::default()
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
            ..SpeechExtractSettings::default()
        };
        assert!(Plan::new(&small).is_err());
        let no_bit_rate = SpeechExtractSettings {
            bit_rate: 0,
            ..SpeechExtractSettings::default()
        };
        assert!(Plan::new(&no_bit_rate).is_err());
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
