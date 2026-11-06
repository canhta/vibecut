//! Bounded first-frame thumbnails decoded through the existing video reader.
//!
//! No browser API and no second frame decoder: the same `VideoReader` that
//! preview and export use decodes the still. Only the first frame at the
//! requested position is decoded, converted to RGBA8 and scaled down, so memory
//! uses a bounded software decoder and only thumbnail-sized RGBA output planes.
use crate::{MediaError, Result, RgbaConverter, SourceAsset, VideoReader};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use time::Tick;

/// Ticket default: the first decodable frame, at most 320 px on the long edge.
pub const THUMBNAIL_MAX_EDGE: u32 = 320;
/// One thumbnail never exceeds this many bytes (1024 * 1024 * 4).
const MAX_THUMBNAIL_EDGE: u32 = 1024;
/// Native thumbnail/recovery image and scratch ceiling (separate from playback).
pub(crate) const MAX_SOURCE_PIXELS: u64 = 1 << 21;
const MAGIC: &[u8; 8] = b"VTMB0001";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// Presentation timestamp of the decoded source frame.
    pub frame: Tick,
    /// Straight RGBA8, exactly `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

impl Thumbnail {
    /// Cache payload; `decode` revalidates every bound before trusting bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + 4 + 4 + 8 + self.pixels.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.frame.0.to_le_bytes());
        bytes.extend_from_slice(&self.pixels);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.get(..8) != Some(MAGIC) {
            return Err(MediaError::Invalid("thumbnail header"));
        }
        let width = read_u32(bytes, 8)?;
        let height = read_u32(bytes, 12)?;
        let frame = read_i64(bytes, 16)?;
        let Some(pixels) = bytes.get(24..) else {
            return Err(MediaError::Invalid("thumbnail payload"));
        };
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or(MediaError::TooLarge("thumbnail geometry"))?;
        if width == 0
            || height == 0
            || width > MAX_THUMBNAIL_EDGE
            || height > MAX_THUMBNAIL_EDGE
            || count != pixels.len() as u64
        {
            return Err(MediaError::Invalid("thumbnail geometry"));
        }
        Ok(Self {
            width,
            height,
            frame: Tick(frame),
            pixels: pixels.to_vec(),
        })
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    let value = bytes
        .get(at..at + 4)
        .ok_or(MediaError::Invalid("thumbnail header"))?;
    let mut buffer = [0u8; 4];
    buffer.copy_from_slice(value);
    Ok(u32::from_le_bytes(buffer))
}

fn read_i64(bytes: &[u8], at: usize) -> Result<i64> {
    let value = bytes
        .get(at..at + 8)
        .ok_or(MediaError::Invalid("thumbnail header"))?;
    let mut buffer = [0u8; 8];
    buffer.copy_from_slice(value);
    Ok(i64::from_le_bytes(buffer))
}

/// Decode one frame of `asset` at `at` and scale it to `max_edge`.
pub fn thumbnail(
    asset: &SourceAsset,
    at: Tick,
    max_edge: u32,
    cancelled: Arc<AtomicBool>,
) -> Result<Thumbnail> {
    thumbnail_at(asset.path(), at, max_edge, cancelled)
}

/// Decode one frame from `path`. `at == 0` reads the first frame; any later
/// position seeks to the preceding keyframe and decodes forward to that frame.
pub fn thumbnail_at(
    path: &Path,
    at: Tick,
    max_edge: u32,
    cancelled: Arc<AtomicBool>,
) -> Result<Thumbnail> {
    thumbnail_at_with_read_bytes(path, at, max_edge, cancelled).map(|(thumbnail, _)| thumbnail)
}

/// Same as [`thumbnail_at`], also reporting how many source bytes the first
/// frame needed. Used to prove bounded reads on large files.
pub fn thumbnail_at_with_read_bytes(
    path: &Path,
    at: Tick,
    max_edge: u32,
    cancelled: Arc<AtomicBool>,
) -> Result<(Thumbnail, u64)> {
    if !(1..=MAX_THUMBNAIL_EDGE).contains(&max_edge) {
        return Err(MediaError::Invalid("thumbnail max edge"));
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    let mut reader = VideoReader::open_thumbnail(path, cancelled.clone())?;
    if at == Tick::ZERO
        && max_edge == THUMBNAIL_MAX_EDGE
        && let Some(thumbnail) = reader.take_recovered_thumbnail()
    {
        return Ok((thumbnail, reader.bytes_read()));
    }
    if at > Tick::ZERO {
        reader.seek(at)?;
    }
    let frame = reader
        .next_frame()?
        .ok_or(MediaError::Unsupported("video has no decodable frame"))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    let thumbnail = from_frame(&frame, at, max_edge)?;
    Ok((thumbnail, reader.bytes_read()))
}

pub(crate) fn from_frame(frame: &crate::VideoFrame, at: Tick, max_edge: u32) -> Result<Thumbnail> {
    let (width, height) = fit(frame.width(), frame.height(), max_edge)?;
    let mut converter = RgbaConverter::new()?;
    let rgba = converter.convert_scaled(frame, width, height)?;
    let stride = rgba.stride(0)?;
    let row_bytes = width as usize * 4;
    let plane = rgba.plane(0)?;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for y in 0..height as usize {
        let start = y
            .checked_mul(stride)
            .ok_or(MediaError::TooLarge("thumbnail row"))?;
        pixels.extend_from_slice(
            plane
                .get(start..start + row_bytes)
                .ok_or(MediaError::Invalid("thumbnail stride"))?,
        );
    }
    Ok(Thumbnail {
        width,
        height,
        frame: frame.pts()?.unwrap_or(at),
        pixels,
    })
}

/// Preserve aspect ratio inside a square `max_edge` box without upscaling.
fn fit(width: u32, height: u32, max_edge: u32) -> Result<(u32, u32)> {
    let longest = width.max(height);
    if longest <= max_edge {
        return Ok((width, height));
    }
    let scaled = |value: u32| -> Result<u32> {
        let value = u64::from(value)
            .checked_mul(u64::from(max_edge))
            .ok_or(MediaError::TooLarge("thumbnail scale"))?
            .checked_add(u64::from(longest) / 2)
            .ok_or(MediaError::TooLarge("thumbnail scale"))?
            / u64::from(longest);
        u32::try_from(value.max(1)).map_err(|_| MediaError::TooLarge("thumbnail scale"))
    };
    Ok((scaled(width)?, scaled(height)?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn display_scale_preserves_aspect_and_never_upscales() {
        assert_eq!(fit(1920, 1080, 320).unwrap(), (320, 180));
        assert_eq!(fit(1080, 1920, 320).unwrap(), (180, 320));
        assert_eq!(fit(100, 50, 320).unwrap(), (100, 50));
        assert_eq!(fit(1921, 1081, 320).unwrap(), (320, 180));
    }

    #[test]
    fn cache_payload_round_trips_and_rejects_corrupt_bytes() {
        let thumbnail = Thumbnail {
            width: 2,
            height: 1,
            frame: Tick(7),
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert_eq!(Thumbnail::decode(&thumbnail.encode()).unwrap(), thumbnail);
        assert!(Thumbnail::decode(b"short").is_err());
        let mut corrupt = thumbnail.encode();
        corrupt.truncate(20);
        assert!(Thumbnail::decode(&corrupt).is_err());
        let mut wrong = thumbnail.encode();
        if let Some(byte) = wrong.get_mut(24) {
            *byte ^= 0xff;
        }
        // Pixel changes are still a valid payload; geometry stays authoritative.
        assert!(Thumbnail::decode(&wrong).is_ok());
    }
}
