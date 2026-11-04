//! Bounded media preparation for local vision analysis.
//!
//! Decodes source frames at exact presentation times, prepares the face crops
//! and 16 kHz mono MFCC frames used by the Light-ASD/CViT preprocessing recipe
//! from `p1-clips/12`, and keeps all work bounded by caller-supplied limits.
//! Detector inference, shot tracking and job orchestration belong to their
//! owners; this module only prepares media.

use crate::{
    MediaError, PixelFormat, Result, RgbaConverter, SourceAsset, StreamKind, VideoFrame,
    VideoReader, init,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

/// Mono sample rate used by the vision models.
pub const VISION_SAMPLE_RATE: u32 = 16_000;
/// Cepstral coefficients returned by [`mfcc_range`].
pub const MFCC_COEFFICIENTS: usize = 13;
/// Side length of the grayscale face crop returned by [`VisionFrame::face_crop`].
pub const FACE_CROP_SIZE: u32 = 112;

const MFCC_HOP_SAMPLES: i64 = 160;
const MFCC_WINDOW_SAMPLES: usize = 400;
const MFCC_FFT_SIZE: usize = 512;
const MFCC_FILTERS: usize = 26;
const MFCC_PREEMPHASIS: f64 = 0.97;
const MFCC_CEP_LIFTER: f64 = 22.0;
const TICKS_PER_MFCC_SAMPLE: i64 = TICKS_PER_SECOND / VISION_SAMPLE_RATE as i64;
const TICKS_PER_MFCC_HOP: i64 = TICKS_PER_MFCC_SAMPLE * MFCC_HOP_SAMPLES;
const FACE_CROP_RESIZE: u32 = 224;
const FACE_CROP_INSET: u32 = 56;
const FACE_CROP_EXPANSION: f64 = 0.40;
const FACE_PADDING: u8 = 110;
const RGBA_GRAY_R: u32 = 19_596;
const RGBA_GRAY_G: u32 = 38_470;
const RGBA_GRAY_B: u32 = 7_470;
const RGBA_GRAY_ROUND: u32 = 1 << 15;

/// Hard bounds for one preparation request. The defaults mirror the existing
/// speech limits: ten minutes of source, one hundred thousand video frames and
/// 60,001 MFCC frames (ten minutes at a 10 ms hop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisionPrepLimits {
    /// Longest shot or audio range admitted by one call.
    pub max_shot_duration: Tick,
    /// Maximum number of decoded video frames emitted by one call.
    pub max_frames: usize,
    /// Maximum number of MFCC frames emitted by one call.
    pub max_mfcc_frames: usize,
    /// Maximum pixel area of one face-crop rectangle before resizing.
    pub max_face_crop_pixels: usize,
}
impl Default for VisionPrepLimits {
    fn default() -> Self {
        Self {
            max_shot_duration: Tick(600 * TICKS_PER_SECOND),
            max_frames: 100_000,
            max_mfcc_frames: 60_001,
            max_face_crop_pixels: 1 << 24,
        }
    }
}
impl VisionPrepLimits {
    fn validate(self) -> Result<Self> {
        if self.max_shot_duration <= Tick::ZERO
            || self.max_frames == 0
            || self.max_frames > 1_000_000
            || self.max_mfcc_frames == 0
            || self.max_mfcc_frames > 1_000_000
            || self.max_face_crop_pixels == 0
            || self.max_face_crop_pixels > 1 << 26
        {
            return Err(MediaError::Invalid("vision preparation limits"));
        }
        Ok(self)
    }
}

/// A decoded source frame packed as straight RGBA8, with its exact source PTS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionFrame {
    source_pts: Tick,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl VisionFrame {
    /// Construct a packed RGBA8 frame. `pixels` must contain exactly
    /// `width * height * 4` bytes; dimensions are bounded like decoded frames.
    pub fn from_rgba8(source_pts: Tick, width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        let area = u64::from(width) * u64::from(height);
        if width == 0
            || height == 0
            || area > 1 << 26
            || area.checked_mul(4) != Some(pixels.len() as u64)
        {
            return Err(MediaError::Invalid("vision frame geometry"));
        }
        Ok(Self {
            source_pts,
            width,
            height,
            pixels,
        })
    }
    /// Exact source presentation timestamp of this frame.
    pub fn source_pts(&self) -> Tick {
        self.source_pts
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Packed straight RGBA8, row-major with no padding.
    pub fn rgba(&self) -> &[u8] {
        &self.pixels
    }
    /// Crop, grayscale and resize a face region using the `p1-clips/12`
    /// reference recipe: expand by 0.40, pad outside pixels with gray 110,
    /// resize to 224x224 with linear interpolation and keep the centre 112.
    /// The returned pixels are raw 0-255 grayscale.
    pub fn face_crop(&self, face: FaceBox, limits: &VisionPrepLimits) -> Result<FaceCrop> {
        let limits = limits.validate()?;
        let geometry = FaceGeometry::new(face, limits.max_face_crop_pixels)?;
        let mut gray = vec![0u8; geometry.width * geometry.height];
        for row in 0..geometry.height {
            let source_y = geometry.top + row as i64;
            for column in 0..geometry.width {
                let source_x = geometry.left + column as i64;
                let value = if source_x < 0
                    || source_y < 0
                    || source_x >= i64::from(self.width)
                    || source_y >= i64::from(self.height)
                {
                    FACE_PADDING
                } else {
                    self.gray_at(source_x as u32, source_y as u32)?
                };
                if let Some(slot) = gray.get_mut(row * geometry.width + column) {
                    *slot = value;
                }
            }
        }
        let resized = resize_center(&gray, geometry.width, geometry.height)?;
        FaceCrop::new(self.source_pts, FACE_CROP_SIZE, FACE_CROP_SIZE, resized)
    }
    fn gray_at(&self, x: u32, y: u32) -> Result<u8> {
        if x >= self.width || y >= self.height {
            return Err(MediaError::Invalid("vision frame pixel bounds"));
        }
        let start = usize::try_from((u64::from(y) * u64::from(self.width) + u64::from(x)) * 4)
            .map_err(|_| MediaError::TooLarge("vision frame pixel offset"))?;
        let end = start
            .checked_add(4)
            .ok_or(MediaError::TooLarge("vision frame pixel offset"))?;
        let pixel = self
            .pixels
            .get(start..end)
            .ok_or(MediaError::Invalid("vision frame pixel bounds"))?;
        let r = u32::from(*pixel.first().ok_or(MediaError::Invalid("RGBA pixel"))?);
        let g = u32::from(*pixel.get(1).ok_or(MediaError::Invalid("RGBA pixel"))?);
        let b = u32::from(*pixel.get(2).ok_or(MediaError::Invalid("RGBA pixel"))?);
        let value = (RGBA_GRAY_R * r + RGBA_GRAY_G * g + RGBA_GRAY_B * b + RGBA_GRAY_ROUND) >> 16;
        u8::try_from(value).map_err(|_| MediaError::Invalid("RGB to gray"))
    }
}

/// Face box in source pixels. The box is expanded by the reference recipe, so
/// it may validly extend outside the decoded frame; missing pixels are padded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Raw 0-255 grayscale face crop for one source frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceCrop {
    source_pts: Tick,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl FaceCrop {
    fn new(source_pts: Tick, width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) != pixels.len() as u64
        {
            return Err(MediaError::Invalid("face crop geometry"));
        }
        Ok(Self {
            source_pts,
            width,
            height,
            pixels,
        })
    }
    pub fn source_pts(&self) -> Tick {
        self.source_pts
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Cepstral coefficients for one 25 ms MFCC window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MfccFrame {
    /// Exact source tick at the start of the 25 ms analysis window.
    pub source_start: Tick,
    /// Cepstral coefficients `c0..c12`, already liftered. `c0` is the natural
    /// log of the pre-emphasized frame energy, as in the reference recipe.
    pub coefficients: [f32; MFCC_COEFFICIENTS],
}

/// Decode the source frame whose presentation interval contains `at`.
///
/// The returned frame may have a PTS at or before `at`. A time after the last
/// decoded frame returns that frame; a time before the first decoded frame or a
/// source without a video stream is an error. Cancellation is checked between
/// decoded frames, so at most one frame of decode latency remains.
pub fn decode_frame_at(
    asset: &SourceAsset,
    at: Tick,
    cancel: Arc<AtomicBool>,
) -> Result<VisionFrame> {
    if cancel.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    let mut reader = VideoReader::open(asset.path())?;
    reader.seek_keyframe(at)?;
    let mut converter = RgbaConverter::new()?;
    let mut selected = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let Some(frame) = reader.next_frame()? else {
            return selected
                .map(|frame| pack_frame(frame, &mut converter))
                .transpose()?
                .ok_or(MediaError::Invalid("frame time outside video stream"));
        };
        let pts = frame.pts()?.ok_or(MediaError::Unsupported(
            "video presentation timestamp unavailable",
        ))?;
        if pts <= at {
            selected = Some(frame);
        } else {
            return selected
                .map(|frame| pack_frame(frame, &mut converter))
                .transpose()?
                .ok_or(MediaError::Invalid("frame time outside video stream"));
        }
    }
}

/// Stream the source frames whose presentation intervals intersect `range`.
///
/// Frames are emitted in presentation order and include the frame covering
/// `range.start`, whose PTS may precede the range. A range containing no frame
/// interval emits nothing. The callback runs before the next frame is decoded;
/// cancellation is checked before every frame and every callback. The range
/// duration and emitted frame count are bounded by `limits`.
pub fn decode_shot_frames(
    asset: &SourceAsset,
    range: SourceRange,
    limits: &VisionPrepLimits,
    cancel: Arc<AtomicBool>,
    mut frame: impl FnMut(VisionFrame) -> Result<()>,
) -> Result<()> {
    let limits = limits.validate()?;
    if range.duration() > limits.max_shot_duration {
        return Err(MediaError::TooLarge("vision shot duration"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    let video_end = asset
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Video)
        .and_then(|stream| {
            let start = stream.start.unwrap_or(Tick::ZERO);
            stream
                .duration
                .and_then(|duration| start.checked_add(duration))
        });
    let mut reader = VideoReader::open(asset.path())?;
    reader.seek_keyframe(range.start())?;
    let mut converter = RgbaConverter::new()?;
    let mut candidate: Option<VideoFrame> = None;
    let mut emitted = 0usize;
    let mut emit = |raw: VideoFrame, emitted: &mut usize| -> Result<()> {
        if *emitted >= limits.max_frames {
            return Err(MediaError::TooLarge("vision shot frames"));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let packed = pack_frame(raw, &mut converter)?;
        *emitted = emitted
            .checked_add(1)
            .ok_or(MediaError::TooLarge("vision shot frames"))?;
        frame(packed)
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let Some(raw) = reader.next_frame()? else {
            if let Some(raw) = candidate.take()
                && video_end.is_none_or(|end| range.start() < end)
            {
                emit(raw, &mut emitted)?;
            }
            return Ok(());
        };
        let pts = raw.pts()?.ok_or(MediaError::Unsupported(
            "video presentation timestamp unavailable",
        ))?;
        if pts < range.start() {
            candidate = Some(raw);
            continue;
        }
        if let Some(raw) = candidate.take()
            && pts > range.start()
        {
            emit(raw, &mut emitted)?;
        }
        if pts >= range.end() {
            return Ok(());
        }
        emit(raw, &mut emitted)?;
    }
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
/// source samples; windows past the end of the source are zero-padded like the
/// reference. A sub-chunk range still yields the frames that start inside it.
pub fn mfcc_range(
    asset: &SourceAsset,
    range: SourceRange,
    limits: &VisionPrepLimits,
    cancel: Arc<AtomicBool>,
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
    if range.end() <= origin {
        return Err(MediaError::Invalid("audio range outside stream"));
    }
    if audio_end.is_some_and(|end| range.start() >= end) {
        return Err(MediaError::Invalid("audio range outside stream"));
    }
    let from = range.start().max(origin);
    let first_frame = ceil_div(
        from.checked_sub(origin)
            .ok_or(MediaError::Invalid("MFCC range origin"))?
            .0,
        TICKS_PER_MFCC_HOP,
    )?;
    let mut end_frame = ceil_div(
        range
            .end()
            .checked_sub(origin)
            .ok_or(MediaError::Invalid("MFCC range origin"))?
            .0,
        TICKS_PER_MFCC_HOP,
    )?;
    if let Some(total_frames) = audio_end
        .map(|end| total_mfcc_frames(end, origin))
        .transpose()?
    {
        end_frame = end_frame.min(total_frames);
    }
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
    let decode_start = sample_tick(origin, first_sample)?;
    let decode_end = sample_tick(origin, last_sample)?;
    let decode_range = SourceRange::new(decode_start, decode_end)?;
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
    let result =
        crate::audio_extract::decode_speech(asset, decode_range, &cancel, &mut |samples, tick| {
            sink.push(samples, tick)
        });
    if let Err(error) = result {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        return Err(error);
    }
    if !sink.received_any {
        return Err(MediaError::Invalid("no audio samples in range"));
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
    fn push(&mut self, samples: &[f32], tick: Tick) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let start = sample_index(self.origin, tick)?;
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

fn sample_index(origin: Tick, tick: Tick) -> Result<i64> {
    let delta = tick
        .checked_sub(origin)
        .ok_or(MediaError::Invalid("MFCC sample clock"))?
        .0;
    if delta % TICKS_PER_MFCC_SAMPLE != 0 {
        return Err(MediaError::Invalid("MFCC sample clock"));
    }
    Ok(delta / TICKS_PER_MFCC_SAMPLE)
}
fn sample_tick(origin: Tick, index: i64) -> Result<Tick> {
    let delta = index
        .checked_mul(TICKS_PER_MFCC_SAMPLE)
        .ok_or(MediaError::TooLarge("MFCC sample clock"))?;
    origin
        .checked_add(Tick(delta))
        .ok_or(MediaError::TooLarge("MFCC sample clock"))
}
fn total_mfcc_frames(audio_end: Tick, origin: Tick) -> Result<i64> {
    let samples = audio_end
        .checked_sub(origin)
        .ok_or(MediaError::Invalid("MFCC audio duration"))?
        .to_units(i64::from(VISION_SAMPLE_RATE), Rounding::Floor)?;
    if samples <= MFCC_WINDOW_SAMPLES as i64 {
        return Ok(1);
    }
    Ok(1 + (samples - MFCC_WINDOW_SAMPLES as i64 + MFCC_HOP_SAMPLES - 1) / MFCC_HOP_SAMPLES)
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

fn pack_frame(raw: VideoFrame, converter: &mut RgbaConverter) -> Result<VisionFrame> {
    let pts = raw.pts()?.ok_or(MediaError::Unsupported(
        "video presentation timestamp unavailable",
    ))?;
    let converted = converter.convert(&raw)?;
    if converted.pixel_format() != PixelFormat::RGBA {
        return Err(MediaError::Unsupported("vision frame is not RGBA"));
    }
    let width = converted.width();
    let height = converted.height();
    let row = usize::try_from(u64::from(width) * 4)
        .map_err(|_| MediaError::TooLarge("vision frame stride"))?;
    let stride = converted.stride(0)?;
    let plane = converted.plane(0)?;
    let expected = row
        .checked_mul(height as usize)
        .ok_or(MediaError::TooLarge("vision frame bytes"))?;
    if stride < row {
        return Err(MediaError::Invalid("vision frame stride"));
    }
    let mut pixels = Vec::with_capacity(expected);
    for y in 0..height as usize {
        let start = y
            .checked_mul(stride)
            .ok_or(MediaError::TooLarge("vision frame stride"))?;
        let end = start
            .checked_add(row)
            .ok_or(MediaError::TooLarge("vision frame stride"))?;
        let line = plane
            .get(start..end)
            .ok_or(MediaError::Invalid("vision frame plane bounds"))?;
        pixels.extend_from_slice(line);
    }
    VisionFrame::from_rgba8(pts, width, height, pixels)
}

struct FaceGeometry {
    left: i64,
    top: i64,
    width: usize,
    height: usize,
}
impl FaceGeometry {
    fn new(face: FaceBox, max_pixels: usize) -> Result<Self> {
        if !face.x.is_finite()
            || !face.y.is_finite()
            || !face.width.is_finite()
            || !face.height.is_finite()
            || face.width <= 0.0
            || face.height <= 0.0
        {
            return Err(MediaError::Invalid("face box"));
        }
        let half = face.width.max(face.height) / 2.0;
        if half <= 0.0 || !half.is_finite() {
            return Err(MediaError::Invalid("face box"));
        }
        let center_x = face.x + face.width / 2.0;
        let center_y = face.y + face.height / 2.0;
        let border = trunc_f64(half * (1.0 + 2.0 * FACE_CROP_EXPANSION))?;
        let my = trunc_f64(center_y + border as f64)?;
        let mx = trunc_f64(center_x + border as f64)?;
        let top = trunc_f64(my as f64 - half)?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let bottom = trunc_f64(my as f64 + half * (1.0 + 2.0 * FACE_CROP_EXPANSION))?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let left = trunc_f64(mx as f64 - half * (1.0 + FACE_CROP_EXPANSION))?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let right = trunc_f64(mx as f64 + half * (1.0 + FACE_CROP_EXPANSION))?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let height = bottom
            .checked_sub(top)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let width = right
            .checked_sub(left)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        if width <= 0 || height <= 0 {
            return Err(MediaError::Invalid("face crop geometry"));
        }
        let width =
            usize::try_from(width).map_err(|_| MediaError::TooLarge("face crop geometry"))?;
        let height =
            usize::try_from(height).map_err(|_| MediaError::TooLarge("face crop geometry"))?;
        let pixels = width
            .checked_mul(height)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        if pixels > max_pixels {
            return Err(MediaError::TooLarge("face crop pixels"));
        }
        Ok(Self {
            left,
            top,
            width,
            height,
        })
    }
}
fn trunc_f64(value: f64) -> Result<i64> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(MediaError::TooLarge("face crop coordinate"));
    }
    Ok(value.trunc() as i64)
}

#[derive(Clone, Copy)]
struct LinearCoefficient {
    index: usize,
    first: i64,
    second: i64,
}
fn linear_coefficients(
    source: usize,
    destination: u32,
    start: u32,
    count: u32,
) -> Result<Vec<LinearCoefficient>> {
    if source == 0 || destination == 0 {
        return Err(MediaError::Invalid("resize geometry"));
    }
    if start.checked_add(count).is_none_or(|end| end > destination) {
        return Err(MediaError::Invalid("resize region"));
    }
    let scale = source as f64 / destination as f64;
    let last = source - 1;
    let mut coefficients = Vec::with_capacity(count as usize);
    for value in start..start + count {
        let coordinate = (value as f64 + 0.5) * scale - 0.5;
        let mut index = coordinate.floor() as i64;
        let mut fraction = coordinate - index as f64;
        if index < 0 {
            index = 0;
            fraction = 0.0;
        }
        if index > last as i64 {
            index = last as i64;
            fraction = 0.0;
        }
        let first = ((1.0 - fraction) * 2048.0).round_ties_even() as i64;
        let second = (fraction * 2048.0).round_ties_even() as i64;
        coefficients.push(LinearCoefficient {
            index: index as usize,
            first,
            second,
        });
    }
    Ok(coefficients)
}
fn resize_center(crop: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if width == 0
        || height == 0
        || width
            .checked_mul(height)
            .is_none_or(|pixels| pixels != crop.len())
    {
        return Err(MediaError::Invalid("face crop geometry"));
    }
    let columns = FACE_CROP_RESIZE - FACE_CROP_INSET * 2;
    let x = linear_coefficients(width, FACE_CROP_RESIZE, FACE_CROP_INSET, columns)?;
    let y = linear_coefficients(height, FACE_CROP_RESIZE, FACE_CROP_INSET, columns)?;
    let row_length = usize::try_from(columns).map_err(|_| MediaError::TooLarge("resize width"))?;
    let mut horizontal = vec![0i64; height * row_length];
    for source_y in 0..height {
        let row_start = source_y
            .checked_mul(width)
            .ok_or(MediaError::TooLarge("resize row"))?;
        let row = crop
            .get(row_start..row_start + width)
            .ok_or(MediaError::Invalid("resize row"))?;
        for (destination_x, coefficient) in x.iter().enumerate() {
            let first = i64::from(
                *row.get(coefficient.index)
                    .ok_or(MediaError::Invalid("resize column"))?,
            );
            let second = i64::from(
                *row.get((coefficient.index + 1).min(width - 1))
                    .ok_or(MediaError::Invalid("resize column"))?,
            );
            let value = first * coefficient.first + second * coefficient.second;
            if let Some(slot) = horizontal.get_mut(source_y * row_length + destination_x) {
                *slot = value;
            }
        }
    }
    let mut output = vec![0u8; row_length * row_length];
    for (destination_y, coefficient) in y.iter().enumerate() {
        let first_row = coefficient.index * row_length;
        let second_row = (coefficient.index + 1).min(height - 1) * row_length;
        for destination_x in 0..row_length {
            let first = horizontal
                .get(first_row + destination_x)
                .copied()
                .ok_or(MediaError::Invalid("resize row"))?;
            let second = horizontal
                .get(second_row + destination_x)
                .copied()
                .ok_or(MediaError::Invalid("resize row"))?;
            let value = (first * coefficient.first + second * coefficient.second + (1 << 21)) >> 22;
            let clamped = value.clamp(0, 255) as u8;
            if let Some(slot) = output.get_mut(destination_y * row_length + destination_x) {
                *slot = clamped;
            }
        }
    }
    Ok(output)
}
