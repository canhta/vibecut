//! Computes bounded MFCC features from mono 16 kHz source audio.

use crate::{
    AudioDecoder, AudioFrame, AudioResampler, AudioSampleFormat, MediaError, PacketReader, Result,
    SourceAsset, StreamKind, init,
};
use ffmpeg_next as ff;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

use super::limits::VisionPrepLimits;
use crate::source::MfccAnchor;

/// Mono sample rate used by the vision models.
pub const VISION_SAMPLE_RATE: u32 = 16_000;
/// Cepstral coefficients returned by [`mfcc_range`].
pub const MFCC_COEFFICIENTS: usize = 13;

const MFCC_HOP_SAMPLES: i64 = 160;
const MFCC_WINDOW_SAMPLES: usize = 400;
const MFCC_FFT_SIZE: usize = 512;
const MFCC_FILTERS: usize = 26;
const MFCC_PREEMPHASIS: f64 = 0.97;
const MFCC_CEP_LIFTER: f64 = 22.0;
const TICKS_PER_MFCC_SAMPLE: i64 = TICKS_PER_SECOND / VISION_SAMPLE_RATE as i64;
const TICKS_PER_MFCC_HOP: i64 = TICKS_PER_MFCC_SAMPLE * MFCC_HOP_SAMPLES;
const MFCC_AUDIO_PREROLL: Tick = Tick(TICKS_PER_SECOND);

/// Cepstral coefficients for one 25 ms MFCC window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MfccFrame {
    /// Exact source tick at the start of the 25 ms analysis window.
    pub source_start: Tick,
    /// Cepstral coefficients `c0..c12`, already liftered. `c0` is the natural
    /// log of the pre-emphasized frame energy, as in the reference recipe.
    pub coefficients: [f32; MFCC_COEFFICIENTS],
}

/// Progress of automatic first-source preparation and requested-range analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MfccProgress {
    /// Exact source position reached by a bounded preparation slice, in ticks.
    Preparing { decoded_to: Tick },
    /// Preparation is complete; this range's required source samples start
    /// here (including its preceding pre-emphasis sample).
    Analyzing { source_start: Tick },
}

/// Compute the 13-coefficient MFCC stream for a source range.
///
/// The analysis grid is anchored at the start of the source audio stream, so a
/// subrange returns exactly the corresponding suffix of a full-range call:
/// 25 ms windows, 10 ms hops, 26 mel filters, 512-point FFT, pre-emphasis 0.97,
/// cepstral lifter 22 and `c0` replaced by log frame energy. Samples are
/// interpreted as 16-bit full-scale PCM (the reference recipe's units), even
/// though the decoder hands them over as normalized f32.
///
/// Only frames whose window starts inside `range` are emitted. A window that
/// extends past the range but still has source audio is computed from the real
/// source samples; only the source's real final window is zero-padded like the
/// reference. Exact decoded-sample anchors are retained on the source asset and
/// shared by its clones. Late calls seek before the nearest recorded anchor,
/// match its packet/frame identity, and count samples from there; timestamps
/// are only seek hints. A cold distant request automatically prepares the
/// source in one sequential pass of bounded, cancellable slices before serving
/// the range; no manual retry is required. Analysis-range and individual
/// preparation-slice work obey the audio decode/byte limits. Total first-pass
/// work is linear in the unindexed prefix, independent of the requested output
/// size. Use [`mfcc_range_with_progress`] to observe preparation checkpoints.
/// Resampling keeps the global phase and one second of preroll.
/// A sub-chunk range still yields the frames that start inside it.
pub fn mfcc_range(
    asset: &SourceAsset,
    range: SourceRange,
    limits: &VisionPrepLimits,
    cancel: Arc<AtomicBool>,
    frame: impl FnMut(MfccFrame) -> Result<()>,
) -> Result<()> {
    mfcc_range_with_progress(asset, range, limits, cancel, |_| Ok(()), frame)
}

/// Compute [`mfcc_range`] with preparation checkpoints for an analysis job.
///
/// A cold distant range automatically makes one sequential pass, building exact
/// anchors in bounded slices before serving the requested windows. Preparation
/// checkpoints preserve the reader/decoder state and report the exact source
/// cursor; no caller retry or enlarged range budget is required. All work is
/// cancellable and memory is bounded. A callback error stops the pass immediately.
/// Already prepared ranges enter `Analyzing` directly. Jobs should reuse the
/// source asset and submit ranges in source order to share this initial pass.
pub fn mfcc_range_with_progress(
    asset: &SourceAsset,
    range: SourceRange,
    limits: &VisionPrepLimits,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(MfccProgress) -> Result<()>,
    frame: impl FnMut(MfccFrame) -> Result<()>,
) -> Result<()> {
    let limits = limits.validate()?;
    if range.duration() > limits.max_shot_duration {
        return Err(MediaError::TooLarge("vision MFCC duration"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    init()?;
    let stream = asset
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Audio)
        .ok_or(MediaError::NoAudioStream)?;
    let origin = stream.start.unwrap_or(Tick::ZERO);
    let audio_end = stream
        .duration
        .and_then(|duration| origin.checked_add(duration));
    if range.end() <= origin || audio_end.is_some_and(|end| range.start() >= end) {
        return Ok(());
    }
    let from = range.start().max(origin);
    let first_frame = ceil_div(
        from.checked_sub(origin)
            .ok_or(MediaError::Invalid("MFCC range origin"))?
            .0,
        TICKS_PER_MFCC_HOP,
    )?;
    let end_frame = ceil_div(
        range
            .end()
            .checked_sub(origin)
            .ok_or(MediaError::Invalid("MFCC range origin"))?
            .0,
        TICKS_PER_MFCC_HOP,
    )?;
    if end_frame <= first_frame {
        return Ok(());
    }
    let frame_count = end_frame
        .checked_sub(first_frame)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or(MediaError::TooLarge("vision MFCC frames"))?;
    if frame_count > limits.max_mfcc_frames {
        return Err(MediaError::TooLarge("vision MFCC frames"));
    }
    let first_sample = first_frame
        .checked_mul(MFCC_HOP_SAMPLES)
        .and_then(|value| value.checked_sub(1))
        .unwrap_or(0)
        .max(0);
    let last_sample = end_frame
        .checked_sub(1)
        .and_then(|value| value.checked_mul(MFCC_HOP_SAMPLES))
        .and_then(|value| value.checked_add(MFCC_WINDOW_SAMPLES as i64))
        .ok_or(MediaError::TooLarge("vision MFCC samples"))?;
    let mut plan = MfccPlan::new()?;
    let mut sink = MfccSink {
        plan: &mut plan,
        origin,
        next_frame: first_frame,
        end_frame,
        buffer: Vec::new(),
        buffer_start: first_sample,
        received_end: first_sample,
        finished: false,
        received_any: false,
        cancel: &cancel,
        frame,
    };
    let result = decode_mfcc_audio(
        asset,
        first_sample,
        last_sample,
        &limits,
        &cancel,
        &mut progress,
        &mut |samples, start| sink.push(samples, start),
    );
    if let Err(error) = result {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        return Err(error);
    }
    if !sink.received_any {
        return Ok(());
    }
    sink.finish()
}

struct MfccSink<'a, F> {
    plan: &'a mut MfccPlan,
    origin: Tick,
    next_frame: i64,
    end_frame: i64,
    buffer: Vec<f32>,
    buffer_start: i64,
    received_end: i64,
    finished: bool,
    received_any: bool,
    cancel: &'a AtomicBool,
    frame: F,
}
impl<F: FnMut(MfccFrame) -> Result<()>> MfccSink<'_, F> {
    fn push(&mut self, samples: &[f32], start: i64) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        if self.received_any && start != self.received_end {
            return Err(MediaError::Invalid("MFCC audio discontinuity"));
        }
        if !self.received_any {
            self.buffer_start = start;
            self.received_end = start;
        }
        self.buffer.extend_from_slice(samples);
        self.received_end = start
            .checked_add(
                i64::try_from(samples.len()).map_err(|_| MediaError::TooLarge("MFCC samples"))?,
            )
            .ok_or(MediaError::TooLarge("MFCC samples"))?;
        self.received_any = true;
        self.emit(false)
    }
    fn finish(&mut self) -> Result<()> {
        self.finished = true;
        let legal = total_mfcc_frames_from_samples(self.received_end)?;
        self.end_frame = self.end_frame.min(legal);
        self.emit(true)
    }
    fn emit(&mut self, finished: bool) -> Result<()> {
        while self.next_frame < self.end_frame {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            let frame_start = self
                .next_frame
                .checked_mul(MFCC_HOP_SAMPLES)
                .ok_or(MediaError::TooLarge("MFCC frame index"))?;
            let required_end = frame_start
                .checked_add(MFCC_WINDOW_SAMPLES as i64)
                .ok_or(MediaError::TooLarge("MFCC frame window"))?;
            if !finished && self.received_end < required_end {
                break;
            }
            let mut window = [0.0f64; MFCC_WINDOW_SAMPLES];
            for (offset, slot) in window.iter_mut().enumerate() {
                let index = frame_start
                    .checked_add(offset as i64)
                    .ok_or(MediaError::TooLarge("MFCC sample index"))?;
                let value = if self.finished && index >= self.received_end {
                    // The reference pads the already pre-emphasized signal with
                    // zeros; do not leak the last real sample into the padding.
                    0.0
                } else {
                    let sample = f64::from(self.sample(index)?) * 32_768.0;
                    if index == 0 {
                        sample
                    } else {
                        sample - MFCC_PREEMPHASIS * f64::from(self.sample(index - 1)?) * 32_768.0
                    }
                };
                *slot = value;
            }
            let coefficients = self.plan.coefficients(&window)?;
            let source_start = sample_tick(self.origin, frame_start)?;
            (self.frame)(MfccFrame {
                source_start,
                coefficients,
            })?;
            self.next_frame = self
                .next_frame
                .checked_add(1)
                .ok_or(MediaError::TooLarge("MFCC frame index"))?;
            let next_need = self
                .next_frame
                .checked_mul(MFCC_HOP_SAMPLES)
                .and_then(|value| value.checked_sub(1))
                .unwrap_or(0)
                .max(0);
            self.discard_before(next_need)?;
        }
        Ok(())
    }
    fn sample(&self, index: i64) -> Result<f32> {
        if index < self.buffer_start {
            return Err(MediaError::Invalid("MFCC sample was discarded"));
        }
        if index >= self.received_end {
            if self.finished {
                return Ok(0.0);
            }
            return Err(MediaError::Invalid("MFCC sample unavailable"));
        }
        let offset = usize::try_from(index - self.buffer_start)
            .map_err(|_| MediaError::TooLarge("MFCC sample offset"))?;
        self.buffer
            .get(offset)
            .copied()
            .ok_or(MediaError::Invalid("MFCC sample offset"))
    }
    fn discard_before(&mut self, index: i64) -> Result<()> {
        if index <= self.buffer_start {
            return Ok(());
        }
        let count = usize::try_from(index - self.buffer_start)
            .map_err(|_| MediaError::TooLarge("MFCC buffer"))?;
        let count = count.min(self.buffer.len());
        self.buffer.drain(..count);
        self.buffer_start = self
            .buffer_start
            .checked_add(count as i64)
            .ok_or(MediaError::TooLarge("MFCC buffer"))?;
        Ok(())
    }
}

fn sample_tick_for(origin: Tick, index: i64, ticks_per_sample: i64) -> Result<Tick> {
    let delta = index
        .checked_mul(ticks_per_sample)
        .ok_or(MediaError::TooLarge("audio sample clock"))?;
    origin
        .checked_add(Tick(delta))
        .ok_or(MediaError::TooLarge("audio sample clock"))
}
fn sample_tick(origin: Tick, index: i64) -> Result<Tick> {
    sample_tick_for(origin, index, TICKS_PER_MFCC_SAMPLE)
}
fn total_mfcc_frames_from_samples(samples: i64) -> Result<i64> {
    if samples <= 0 {
        return Err(MediaError::Invalid("no audio samples"));
    }
    if samples <= MFCC_WINDOW_SAMPLES as i64 {
        return Ok(1);
    }
    Ok(1 + ceil_div(samples - MFCC_WINDOW_SAMPLES as i64, MFCC_HOP_SAMPLES)?)
}
fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}
fn round_up(value: i64, step: i64) -> Result<i64> {
    if step <= 0 {
        return Err(MediaError::Invalid("audio sample step"));
    }
    if value <= 0 {
        return Ok(0);
    }
    let remainder = value % step;
    if remainder == 0 {
        return Ok(value);
    }
    value
        .checked_add(step - remainder)
        .ok_or(MediaError::TooLarge("audio sample index"))
}
fn output_sample_index(input_index: i64, input_rate: u32) -> Result<i64> {
    let value = i128::from(input_index) * i128::from(VISION_SAMPLE_RATE);
    let value = value / i128::from(input_rate);
    i64::try_from(value).map_err(|_| MediaError::TooLarge("audio sample index"))
}
/// Convert one decoded frame to planar f32 at its source rate.
///
/// Decoder frames may carry an unspecified channel layout; build the default
/// layout for the actual channel count instead of asking the resampler to
/// change layout mid-stream.
fn audio_planes_f32(frame: &AudioFrame) -> Result<Vec<Vec<f32>>> {
    let channels = frame.channels();
    if channels == 0 || channels > 32 {
        return Err(MediaError::Invalid("audio channels"));
    }
    let layout = frame.inner.channel_layout();
    let layout = if layout.is_empty() {
        ff::ChannelLayout::default(i32::from(channels))
    } else {
        layout
    };
    let mut source = crate::ffi::reference_audio(&frame.inner)?;
    source.set_channel_layout(layout);
    let mut converter = ff::software::resampling::Context::get(
        source.format(),
        layout,
        source.rate(),
        ff::format::Sample::F32(ff::format::sample::Type::Planar),
        layout,
        source.rate(),
    )
    .map_err(crate::error("convert planar audio"))?;
    let mut output = ff::frame::Audio::empty();
    converter
        .run(&source, &mut output)
        .map_err(crate::error("convert planar audio"))?;
    if output.samples() > 65_536 || output.channels() > 32 {
        return Err(MediaError::TooLarge("audio planar conversion"));
    }
    if output.samples() != frame.samples() {
        return Err(MediaError::Unsupported(
            "audio planar conversion changed sample count",
        ));
    }
    Ok((0..usize::from(output.channels()))
        .map(|channel| output.plane::<f32>(channel).to_vec())
        .collect())
}

fn emit_mfcc_audio(
    frame: &AudioFrame,
    index: &mut i64,
    push: &mut impl FnMut(&[f32], i64) -> Result<()>,
) -> Result<()> {
    let planes = frame.planar_f32()?;
    let mono = planes
        .first()
        .ok_or(MediaError::Invalid("audio output channels"))?;
    if mono.is_empty() {
        return Ok(());
    }
    push(mono, *index)?;
    *index = index
        .checked_add(i64::try_from(mono.len()).map_err(|_| MediaError::TooLarge("audio samples"))?)
        .ok_or(MediaError::TooLarge("audio samples"))?;
    Ok(())
}

/// Decode source audio and resample it to the global 16 kHz grid.
///
/// A fresh `AudioResampler` starts on an input sample whose phase matches the
/// stream-global output grid, then runs for at least one second before the
/// first requested window. Count from stream start or a verified recorded
/// anchor: neither rounded PTS nor frame size alone identifies a seek boundary.
/// Frames before the preroll are discarded without conversion. `push` receives
/// contiguous absolute 16 kHz sample indexes; the caller caps the final window count from
/// the actual received extent, so an unknown-duration EOF never fabricates
/// windows.
fn decode_mfcc_audio(
    asset: &SourceAsset,
    first_sample: i64,
    last_sample: i64,
    limits: &VisionPrepLimits,
    cancel: &Arc<AtomicBool>,
    progress: &mut impl FnMut(MfccProgress) -> Result<()>,
    push: &mut impl FnMut(&[f32], i64) -> Result<()>,
) -> Result<()> {
    init()?;
    let mut reader =
        PacketReader::open_with_read_limit(asset.path(), cancel.clone(), limits.max_audio_bytes)?;
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Audio)
        .cloned()
        .ok_or(MediaError::NoAudioStream)?;
    let input_rate = stream
        .audio
        .as_ref()
        .ok_or(MediaError::NoAudioStream)?
        .sample_rate;
    if input_rate == 0 {
        return Err(MediaError::Invalid("audio sample rate"));
    }
    if TICKS_PER_SECOND % i64::from(input_rate) != 0 {
        return Err(MediaError::Unsupported("audio sample clock"));
    }
    let ticks_per_input = TICKS_PER_SECOND / i64::from(input_rate);
    let origin = stream.start.unwrap_or(Tick::ZERO);
    let first_tick = sample_tick_for(origin, first_sample, TICKS_PER_MFCC_SAMPLE)?;
    let target_tick = first_tick
        .checked_sub(MFCC_AUDIO_PREROLL)
        .unwrap_or(Tick::ZERO)
        .max(origin);
    let mut decoder = AudioDecoder::new(&stream)?;
    let step = i64::from(input_rate / gcd(input_rate, VISION_SAMPLE_RATE));
    let target_index = ceil_div(
        target_tick
            .checked_sub(origin)
            .ok_or(MediaError::Invalid("audio sample clock"))?
            .0,
        ticks_per_input,
    )?;
    let (mut resume, seek) = asset
        .anchors()
        .lock()
        .map_err(|_| MediaError::Invalid("MFCC anchor state"))?
        .resume(target_index);
    if let Some(anchor) = seek {
        // Warm the decoder before matching the exact anchor. Round the seek
        // hint up to microseconds so a PCM demuxer cannot start one sample early.
        let hint = anchor
            .tick
            .checked_add(Tick(TICKS_PER_SECOND / 1_000_000))
            .ok_or(MediaError::TooLarge("MFCC anchor time"))?;
        if reader.seek(hint).is_err() {
            return Err(if reader.bytes_read() >= limits.max_audio_bytes {
                MediaError::TooLarge("input read bytes")
            } else if cancel.load(Ordering::Relaxed) {
                MediaError::Cancelled
            } else {
                MediaError::Unsupported("MFCC anchor seek")
            });
        }
    }
    let max_samples = i128::from(limits.max_audio_decode_duration.0) * i128::from(input_rate)
        / i128::from(TICKS_PER_SECOND);
    let required_end =
        (i128::from(last_sample) * i128::from(input_rate) + i128::from(VISION_SAMPLE_RATE) - 1)
            / i128::from(VISION_SAMPLE_RATE);
    let nearby = resume.is_some()
        && required_end - i128::from(resume.map_or(0, |anchor| anchor.index)) <= max_samples / 2;
    let mut preparing = target_index > 0 && !nearby;
    let mut work = MfccWorkWindow::default();
    let mut state = MfccAudioState {
        origin,
        input_rate,
        ticks_per_input,
        aligned_index: round_up(target_index, step)?,
        last_sample,
        resampler: None,
        out_index: 0,
        next_input: 0,
        cancel,
        push,
    };
    progress(if preparing {
        MfccProgress::Preparing { decoded_to: origin }
    } else {
        MfccProgress::Analyzing {
            source_start: first_tick,
        }
    })?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        if preparing
            && (work.packets >= 1_000_000
                || reader.bytes_read().saturating_sub(work.read_start)
                    >= limits.max_audio_bytes / 2)
        {
            work.restart(&mut reader, limits.max_audio_bytes);
            progress(MfccProgress::Preparing {
                decoded_to: sample_tick_for(origin, state.next_input, ticks_per_input)?,
            })?;
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
        }
        work.packets += 1;
        if work.packets > 1_000_000 {
            return Err(MediaError::TooLarge("MFCC input packets"));
        }
        let packet = reader.next_packet()?;
        let (reached_end, position) = match packet {
            Some(packet) if packet.stream() == stream.index => {
                let position = packet.inner.position();
                decoder.push_packet(&packet)?;
                (false, position)
            }
            Some(_) => continue,
            None => {
                decoder.finish_input()?;
                (true, -1)
            }
        };
        let mut frame_count = 0usize;
        while let Some(frame) = decoder.pull_frame()? {
            let ordinal = frame_count;
            frame_count += 1;
            if frame_count > 64 {
                return Err(MediaError::TooLarge("audio output"));
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            let samples = frame.samples() as i128;
            let tick = frame.pts()?;
            let matched = if let Some(anchor) = resume {
                if position == anchor.position
                    && ordinal == anchor.ordinal
                    && frame.samples() == anchor.samples
                    && tick == Some(anchor.tick)
                {
                    state.next_input = anchor.index;
                    resume = None;
                    true
                } else {
                    false
                }
            } else {
                true
            };
            if preparing
                && matched
                && i128::from(state.next_input) + samples > i128::from(state.aligned_index)
            {
                // Continue with this very frame: no seek, retry, discarded
                // decoder state or timestamp-derived phase at the transition.
                preparing = false;
                work.restart(&mut reader, limits.max_audio_bytes);
                progress(MfccProgress::Analyzing {
                    source_start: first_tick,
                })?;
            }
            if preparing && samples <= max_samples && work.samples + samples > max_samples {
                work.restart(&mut reader, limits.max_audio_bytes);
                progress(MfccProgress::Preparing {
                    decoded_to: sample_tick_for(origin, state.next_input, ticks_per_input)?,
                })?;
            }
            work.samples += samples;
            if work.samples > max_samples {
                return Err(MediaError::TooLarge("MFCC decoded audio"));
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            if frame.sample_rate() != input_rate {
                return Err(MediaError::Unsupported("audio sample rate changed"));
            }
            if !matched {
                continue;
            }
            if let Some(tick) = tick.filter(|_| position >= 0) {
                asset
                    .anchors()
                    .lock()
                    .map_err(|_| MediaError::Invalid("MFCC anchor state"))?
                    .record(
                        MfccAnchor {
                            index: state.next_input,
                            tick,
                            position,
                            ordinal,
                            samples: frame.samples(),
                        },
                        input_rate,
                    );
            }
            if state.feed(frame)? {
                return Ok(());
            }
        }
        if reached_end {
            if resume.is_some() {
                return Err(MediaError::Unsupported("MFCC exact anchor unavailable"));
            }
            state.finish()?;
            return Ok(());
        }
    }
}

#[derive(Default)]
struct MfccWorkWindow {
    samples: i128,
    packets: usize,
    read_start: u64,
}
impl MfccWorkWindow {
    fn restart(&mut self, reader: &mut PacketReader, max_bytes: u64) {
        *self = Self {
            read_start: reader.bytes_read(),
            ..Self::default()
        };
        reader.start_read_window(max_bytes);
    }
}

struct MfccAudioState<'a, F> {
    origin: Tick,
    input_rate: u32,
    ticks_per_input: i64,
    aligned_index: i64,
    last_sample: i64,
    resampler: Option<AudioResampler>,
    out_index: i64,
    next_input: i64,
    cancel: &'a AtomicBool,
    push: &'a mut F,
}
impl<F: FnMut(&[f32], i64) -> Result<()>> MfccAudioState<'_, F> {
    fn feed(&mut self, frame: AudioFrame) -> Result<bool> {
        let samples =
            i64::try_from(frame.samples()).map_err(|_| MediaError::TooLarge("audio samples"))?;
        if let Some(active) = self.resampler.as_mut() {
            let index = self.next_input;
            let pts = sample_tick_for(self.origin, index, self.ticks_per_input)?;
            let planes = audio_planes_f32(&frame)?;
            let input = AudioFrame::from_planar_f32(pts, self.input_rate, &planes)?;
            self.next_input = index
                .checked_add(samples)
                .ok_or(MediaError::TooLarge("audio sample cursor"))?;
            for output in active.convert(&input)? {
                emit_mfcc_audio(&output, &mut self.out_index, &mut *self.push)?;
                if self.out_index >= self.last_sample {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let start_index = self.next_input;
        if start_index
            .checked_add(samples)
            .is_none_or(|end| end <= self.aligned_index)
        {
            self.next_input = start_index
                .checked_add(samples)
                .ok_or(MediaError::TooLarge("audio sample cursor"))?;
            return Ok(false);
        }
        let skip = usize::try_from(self.aligned_index - start_index)
            .map_err(|_| MediaError::Invalid("audio sample trim"))?;
        if skip >= frame.samples() {
            self.next_input = start_index
                .checked_add(samples)
                .ok_or(MediaError::TooLarge("audio sample cursor"))?;
            return Ok(false);
        }
        let planes = audio_planes_f32(&frame)?;
        let mut trimmed = Vec::with_capacity(planes.len());
        for plane in planes {
            let remainder = plane
                .get(skip..)
                .ok_or(MediaError::Invalid("audio sample trim"))?;
            trimmed.push(remainder.to_vec());
        }
        if trimmed.first().is_none_or(|plane| plane.is_empty()) {
            self.next_input = start_index
                .checked_add(samples)
                .ok_or(MediaError::TooLarge("audio sample cursor"))?;
            return Ok(false);
        }
        let aligned_tick = sample_tick_for(self.origin, self.aligned_index, self.ticks_per_input)?;
        let first = AudioFrame::from_planar_f32(aligned_tick, self.input_rate, &trimmed)?;
        let mut active =
            AudioResampler::new(&first, VISION_SAMPLE_RATE, 1, AudioSampleFormat::PlanarF32)?;
        self.out_index = output_sample_index(self.aligned_index, self.input_rate)?;
        self.next_input = start_index
            .checked_add(samples)
            .ok_or(MediaError::TooLarge("audio sample cursor"))?;
        for output in active.convert(&first)? {
            emit_mfcc_audio(&output, &mut self.out_index, &mut *self.push)?;
            if self.out_index >= self.last_sample {
                return Ok(true);
            }
        }
        self.resampler = Some(active);
        Ok(false)
    }
    fn finish(&mut self) -> Result<()> {
        if self.out_index >= self.last_sample {
            return Ok(());
        }
        if let Some(active) = self.resampler.as_mut() {
            while let Some(output) = active.flush()? {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(MediaError::Cancelled);
                }
                emit_mfcc_audio(&output, &mut self.out_index, &mut *self.push)?;
                if self.out_index >= self.last_sample {
                    break;
                }
            }
        }
        Ok(())
    }
}

fn ceil_div(value: i64, divisor: i64) -> Result<i64> {
    if value <= 0 {
        return Ok(0);
    }
    Ok(value
        .checked_add(divisor - 1)
        .ok_or(MediaError::TooLarge("MFCC frame index"))?
        / divisor)
}

struct MfccPlan {
    twiddles: Vec<Complex>,
    fft: Vec<Complex>,
    spectrum: Vec<f64>,
    filterbank: Vec<f64>,
    dct: Vec<f64>,
    lifter: [f64; MFCC_COEFFICIENTS],
}
impl MfccPlan {
    fn new() -> Result<Self> {
        let filterbank = mel_filterbank()?;
        let mut dct = vec![0.0f64; MFCC_COEFFICIENTS * MFCC_FILTERS];
        for k in 0..MFCC_COEFFICIENTS {
            let scale = if k == 0 {
                (1.0 / MFCC_FILTERS as f64).sqrt()
            } else {
                (2.0 / MFCC_FILTERS as f64).sqrt()
            };
            for n in 0..MFCC_FILTERS {
                let value = scale
                    * (std::f64::consts::PI * k as f64 * (2 * n + 1) as f64
                        / (2 * MFCC_FILTERS) as f64)
                        .cos();
                if let Some(slot) = dct.get_mut(k * MFCC_FILTERS + n) {
                    *slot = value;
                }
            }
        }
        let mut lifter = [0.0f64; MFCC_COEFFICIENTS];
        for (n, slot) in lifter.iter_mut().enumerate() {
            *slot = 1.0
                + (MFCC_CEP_LIFTER / 2.0)
                    * (std::f64::consts::PI * n as f64 / MFCC_CEP_LIFTER).sin();
        }
        let mut twiddles = Vec::with_capacity(MFCC_FFT_SIZE / 2);
        for index in 0..MFCC_FFT_SIZE / 2 {
            let angle = -2.0 * std::f64::consts::PI * index as f64 / MFCC_FFT_SIZE as f64;
            twiddles.push(Complex {
                re: angle.cos(),
                im: angle.sin(),
            });
        }
        Ok(Self {
            twiddles,
            fft: vec![Complex::default(); MFCC_FFT_SIZE],
            spectrum: vec![0.0f64; MFCC_FFT_SIZE / 2 + 1],
            filterbank,
            dct,
            lifter,
        })
    }
    fn coefficients(
        &mut self,
        window: &[f64; MFCC_WINDOW_SAMPLES],
    ) -> Result<[f32; MFCC_COEFFICIENTS]> {
        if self.fft.len() != MFCC_FFT_SIZE {
            return Err(MediaError::Invalid("MFCC FFT workspace"));
        }
        for (index, slot) in self.fft.iter_mut().enumerate() {
            let value = window.get(index).copied().unwrap_or(0.0);
            *slot = Complex { re: value, im: 0.0 };
        }
        fft(&mut self.fft, &self.twiddles)?;
        let mut energy = 0.0f64;
        for (index, slot) in self.spectrum.iter_mut().enumerate() {
            let value = self.fft.get(index).copied().unwrap_or_default();
            let power = (value.re * value.re + value.im * value.im) / MFCC_FFT_SIZE as f64;
            *slot = power;
            energy += power;
        }
        if energy == 0.0 {
            energy = f64::EPSILON;
        }
        let mut log_energies = [0.0f64; MFCC_FILTERS];
        for filter in 0..MFCC_FILTERS {
            let mut sum = 0.0f64;
            for (index, power) in self.spectrum.iter().enumerate() {
                let weight = self
                    .filterbank
                    .get(filter * (MFCC_FFT_SIZE / 2 + 1) + index)
                    .copied()
                    .unwrap_or(0.0);
                sum += power * weight;
            }
            if sum == 0.0 {
                sum = f64::EPSILON;
            }
            if let Some(slot) = log_energies.get_mut(filter) {
                *slot = sum.ln();
            }
        }
        let mut coefficients = [0.0f32; MFCC_COEFFICIENTS];
        for (k, slot) in coefficients.iter_mut().enumerate() {
            let mut sum = 0.0f64;
            for (n, value) in log_energies.iter().enumerate() {
                let coefficient = self.dct.get(k * MFCC_FILTERS + n).copied().unwrap_or(0.0);
                sum += value * coefficient;
            }
            let lifted = sum * self.lifter.get(k).copied().unwrap_or(1.0);
            *slot = if k == 0 {
                energy.ln() as f32
            } else {
                lifted as f32
            };
        }
        Ok(coefficients)
    }
}

fn mel_filterbank() -> Result<Vec<f64>> {
    let bins = MFCC_FFT_SIZE / 2 + 1;
    let low = hz_to_mel(0.0);
    let high = hz_to_mel(VISION_SAMPLE_RATE as f64 / 2.0);
    let mut points = Vec::with_capacity(MFCC_FILTERS + 2);
    for index in 0..=MFCC_FILTERS + 1 {
        let value = low + (high - low) * index as f64 / (MFCC_FILTERS + 1) as f64;
        points.push(value);
    }
    let mut edges = Vec::with_capacity(points.len());
    for point in points {
        let hz = mel_to_hz(point);
        let bin = ((MFCC_FFT_SIZE + 1) as f64 * hz / VISION_SAMPLE_RATE as f64).floor();
        edges.push(bin as usize);
    }
    let mut filterbank = vec![0.0f64; MFCC_FILTERS * bins];
    for filter in 0..MFCC_FILTERS {
        let left = edges.get(filter).copied().unwrap_or(0);
        let middle = edges.get(filter + 1).copied().unwrap_or(0);
        let right = edges.get(filter + 2).copied().unwrap_or(0);
        for bin in left..middle {
            if let Some(slot) = filterbank.get_mut(filter * bins + bin) {
                let value = (bin as f64 - left as f64) / (middle as f64 - left as f64);
                *slot = value;
            }
        }
        for bin in middle..right {
            if let Some(slot) = filterbank.get_mut(filter * bins + bin) {
                let value = (right as f64 - bin as f64) / (right as f64 - middle as f64);
                *slot = value;
            }
        }
    }
    Ok(filterbank)
}
fn hz_to_mel(hz: f64) -> f64 {
    2595.0 * (1.0 + hz / 700.0).log10()
}
fn mel_to_hz(mel: f64) -> f64 {
    700.0 * (10.0f64.powf(mel / 2595.0) - 1.0)
}

#[derive(Clone, Copy, Debug, Default)]
struct Complex {
    re: f64,
    im: f64,
}
impl Complex {
    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
}
fn fft(values: &mut [Complex], twiddles: &[Complex]) -> Result<()> {
    let n = values.len();
    if n == 0 || !n.is_power_of_two() {
        return Err(MediaError::Invalid("MFCC FFT size"));
    }
    let mut reversed = 0usize;
    for index in 1..n {
        let mut bit = n >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            values.swap(index, reversed);
        }
    }
    let mut length = 2usize;
    while length <= n {
        let half = length / 2;
        let step = n / length;
        for start in (0..n).step_by(length) {
            for offset in 0..half {
                let twiddle = twiddles
                    .get(offset * step)
                    .copied()
                    .ok_or(MediaError::Invalid("MFCC FFT twiddle"))?;
                let left = values
                    .get(start + offset)
                    .copied()
                    .ok_or(MediaError::Invalid("MFCC FFT bounds"))?;
                let right = values
                    .get(start + offset + half)
                    .copied()
                    .ok_or(MediaError::Invalid("MFCC FFT bounds"))?
                    .mul(twiddle);
                if let Some(slot) = values.get_mut(start + offset) {
                    *slot = Complex {
                        re: left.re + right.re,
                        im: left.im + right.im,
                    };
                }
                if let Some(slot) = values.get_mut(start + offset + half) {
                    *slot = Complex {
                        re: left.re - right.re,
                        im: left.im - right.im,
                    };
                }
            }
        }
        length <<= 1;
    }
    Ok(())
}
