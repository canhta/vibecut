//! Decodes source frames at exact presentation times and packs them as straight RGBA8.

use crate::{
    MediaError, PixelFormat, Result, RgbaConverter, SourceAsset, StreamKind, VideoFrame,
    VideoReader,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{SourceRange, Tick};

use super::limits::VisionPrepLimits;

const RGBA_GRAY_R: u32 = 19_596;
const RGBA_GRAY_G: u32 = 38_470;
const RGBA_GRAY_B: u32 = 7_470;
const RGBA_GRAY_ROUND: u32 = 1 << 15;

/// A decoded source frame packed as straight RGBA8, with its exact source PTS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionFrame {
    pub(super) source_pts: Tick,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: Vec<u8>,
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
}

impl VisionFrame {
    pub(super) fn sample_resized(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        channel: usize,
        cubic: bool,
    ) -> Result<f32> {
        let sx = (f64::from(x) + 0.5) * f64::from(self.width) / f64::from(width) - 0.5;
        let sy = (f64::from(y) + 0.5) * f64::from(self.height) / f64::from(height) - 0.5;
        let ix = sx.floor() as i64;
        let iy = sy.floor() as i64;
        let mut sum = 0.;
        let offsets = if cubic { -1..=2 } else { 0..=1 };
        for dy in offsets.clone() {
            let wy = resize_weight(sy - (iy + dy) as f64, cubic);
            for dx in offsets.clone() {
                let wx = resize_weight(sx - (ix + dx) as f64, cubic);
                let px = (ix + dx).clamp(0, i64::from(self.width) - 1) as usize;
                let py = (iy + dy).clamp(0, i64::from(self.height) - 1) as usize;
                let value = self
                    .pixels
                    .get((py * self.width as usize + px) * 4 + channel)
                    .ok_or(MediaError::Invalid("resize pixel index"))?;
                sum += f64::from(*value) * wx * wy;
            }
        }
        Ok(sum.round_ties_even().clamp(0., 255.) as f32)
    }
}

impl VisionFrame {
    pub(super) fn gray_at(&self, x: u32, y: u32) -> Result<u8> {
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

/// Decode the source frame whose presentation interval contains `at`.
///
/// The returned frame may have a PTS at or before `at`. A time after the last
/// decoded frame returns that frame; a time before the first decoded frame or a
/// source without a video stream is an error. `limits.max_frames` bounds every
/// frame decoded from the preceding keyframe, not just the returned frame.
/// Cancellation is checked between decoded frames, so at most one frame of
/// decode latency remains.
pub fn decode_frame_at(
    asset: &SourceAsset,
    at: Tick,
    limits: &VisionPrepLimits,
    cancel: Arc<AtomicBool>,
) -> Result<VisionFrame> {
    let limits = limits.validate()?;
    if cancel.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    let mut reader = VideoReader::open_with_cancel(asset.path(), cancel.clone())?;
    reader.seek_keyframe(at)?;
    let mut converter = RgbaConverter::new()?;
    let mut selected = None;
    let mut decoded = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let Some(frame) = reader.next_frame()? else {
            return selected
                .map(|frame| pack_frame(frame, &mut converter, limits.max_frame_pixels))
                .transpose()?
                .ok_or(MediaError::Invalid("frame time outside video stream"));
        };
        decoded = decoded
            .checked_add(1)
            .ok_or(MediaError::TooLarge("vision decoded frames"))?;
        if decoded > limits.max_frames {
            return Err(MediaError::TooLarge("vision decoded frames"));
        }
        let pts = frame.pts()?.ok_or(MediaError::Unsupported(
            "video presentation timestamp unavailable",
        ))?;
        if pts <= at {
            selected = Some(frame);
        } else {
            return selected
                .map(|frame| pack_frame(frame, &mut converter, limits.max_frame_pixels))
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
/// duration and every decoded frame (seek preroll, replay and output) are
/// bounded by `limits`.
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
    let mut reader = VideoReader::open_with_cancel(asset.path(), cancel.clone())?;
    reader.seek_keyframe(range.start())?;
    let mut converter = RgbaConverter::new()?;
    let mut candidate: Option<VideoFrame> = None;
    let mut decoded = 0usize;
    let mut emit = |raw: VideoFrame| -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let packed = pack_frame(raw, &mut converter, limits.max_frame_pixels)?;
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
                emit(raw)?;
            }
            return Ok(());
        };
        decoded = decoded
            .checked_add(1)
            .ok_or(MediaError::TooLarge("vision decoded frames"))?;
        if decoded > limits.max_frames {
            return Err(MediaError::TooLarge("vision decoded frames"));
        }
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
            emit(raw)?;
        }
        if pts >= range.end() {
            return Ok(());
        }
        emit(raw)?;
    }
}

fn pack_frame(
    raw: VideoFrame,
    converter: &mut RgbaConverter,
    max_pixels: u64,
) -> Result<VisionFrame> {
    if u64::from(raw.width()) * u64::from(raw.height()) > max_pixels {
        return Err(MediaError::TooLarge("vision packed frame pixels"));
    }
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

fn resize_weight(distance: f64, cubic: bool) -> f64 {
    let x = distance.abs();
    if !cubic {
        return (1. - x).max(0.);
    }
    let a = -0.75;
    if x <= 1. {
        (a + 2.) * x.powi(3) - (a + 3.) * x.powi(2) + 1.
    } else if x < 2. {
        a * x.powi(3) - 5. * a * x.powi(2) + 8. * a * x - 4. * a
    } else {
        0.
    }
}
