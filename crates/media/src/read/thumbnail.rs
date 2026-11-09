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
/// Source bytes one sampled frame's decode leg may read: the same allowance a
/// single [`thumbnail_at`] call gets for one frame.
const PASS_LEG_BYTES: u64 = 64 << 20;
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

/// Decode one frame per ascending source position from one sequential pass.
///
/// The reader opens and seeks once, then decodes forward; each position
/// receives the first frame at or after it, the same frame [`thumbnail_at`]
/// returns for that position. `sample` receives positions in order before the
/// next frame is decoded. Each leg may read at most the 64 MiB a single
/// [`thumbnail_at`] call may; a longer leg, a position past the last frame, a
/// non-ascending or empty list and cancellation are typed errors.
pub fn thumbnails_at(
    path: &Path,
    positions: &[Tick],
    max_edge: u32,
    cancelled: Arc<AtomicBool>,
    sample: impl FnMut(Tick, Thumbnail),
) -> Result<()> {
    sample_positions(path, positions, max_edge, cancelled, sample).map(|_| ())
}

/// Same as [`thumbnail_at`], also reporting how many source bytes the first
/// frame needed. Used to prove bounded reads on large files.
pub fn thumbnail_at_with_read_bytes(
    path: &Path,
    at: Tick,
    max_edge: u32,
    cancelled: Arc<AtomicBool>,
) -> Result<(Thumbnail, u64)> {
    let mut decoded = None;
    let bytes = sample_positions(path, &[at], max_edge, cancelled, |_, image| {
        decoded = Some(image);
    })?;
    let thumbnail = decoded.ok_or(MediaError::Unsupported("video has no decodable frame"))?;
    Ok((thumbnail, bytes))
}

/// The one open/seek/decode loop behind [`thumbnail_at`] and
/// [`thumbnails_at`]: open once, seek once, decode forward and hand each
/// ascending position the first frame at or after it. Returns the source bytes
/// read when the first frame was delivered.
fn sample_positions(
    path: &Path,
    positions: &[Tick],
    max_edge: u32,
    cancelled: Arc<AtomicBool>,
    mut sample: impl FnMut(Tick, Thumbnail),
) -> Result<u64> {
    if !(1..=MAX_THUMBNAIL_EDGE).contains(&max_edge) {
        return Err(MediaError::Invalid("thumbnail max edge"));
    }
    if positions.is_empty()
        || positions
            .iter()
            .zip(positions.iter().skip(1))
            .any(|(before, after)| before >= after)
    {
        return Err(MediaError::Invalid("thumbnail positions"));
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(MediaError::Cancelled);
    }
    let Some(first) = positions.first().copied() else {
        return Err(MediaError::Invalid("thumbnail positions"));
    };
    let mut reader = VideoReader::open_thumbnail(path, cancelled.clone())?;
    let mut target = 0;
    let mut first_bytes = 0;
    // Opening recovery already decoded and rewound the first frame; publish it
    // instead of decoding position zero again.
    if first == Tick::ZERO
        && max_edge == THUMBNAIL_MAX_EDGE
        && let Some(thumbnail) = reader.take_recovered_thumbnail()
    {
        first_bytes = reader.bytes_read();
        target = 1;
        sample(first, thumbnail);
    }
    if first > Tick::ZERO {
        reader.seek(first)?;
    }
    let mut converter = RgbaConverter::new()?;
    while target < positions.len() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let Some(frame) = reader.next_frame()? else {
            return Err(MediaError::Unsupported("video has no decodable frame"));
        };
        let pts = frame.pts()?.ok_or(MediaError::Unsupported(
            "video presentation timestamp unavailable",
        ))?;
        // The first position is satisfied by the first delivered frame: a seek
        // withholds earlier frames and position zero means the first frame.
        // Later positions take the first frame at or after them, so one frame
        // may satisfy several positions just as one call per position does.
        while let Some(&at) = positions.get(target) {
            if target > 0 && pts < at {
                break;
            }
            let image = from_frame_with(&mut converter, &frame, at, max_edge)?;
            if target == 0 {
                first_bytes = reader.bytes_read();
            }
            target += 1;
            sample(at, image);
            if cancelled.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
        }
        if target < positions.len() {
            reader.start_read_window(PASS_LEG_BYTES);
        }
    }
    Ok(first_bytes)
}

pub(crate) fn from_frame(frame: &crate::VideoFrame, at: Tick, max_edge: u32) -> Result<Thumbnail> {
    from_frame_with(&mut RgbaConverter::new()?, frame, at, max_edge)
}

/// Same conversion with a caller-owned converter, so one sequential pass reuses
/// one scaler for every sampled frame.
fn from_frame_with(
    converter: &mut RgbaConverter,
    frame: &crate::VideoFrame,
    at: Tick,
    max_edge: u32,
) -> Result<Thumbnail> {
    let (width, height) = fit(frame.width(), frame.height(), max_edge)?;
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

/// Manual timing harness for a real interview cut: one thumbnail per tile of a
/// visible minute, the same tiles from one sequential pass, and where the
/// per-tile time goes. Peak RSS is sampled externally around this process.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod benchmark {
    use super::*;
    use std::time::{Duration, Instant};
    use time::TICKS_PER_SECOND;

    #[test]
    #[ignore = "manual: set VIBECUT_INTERVIEW_SAMPLE to interview-30m.mp4"]
    fn interview_thumbnail_tile_and_pass_timing() {
        let Some(path) = std::env::var_os("VIBECUT_INTERVIEW_SAMPLE").map(std::path::PathBuf::from)
        else {
            eprintln!("skipping: VIBECUT_INTERVIEW_SAMPLE is not set");
            return;
        };
        if !path.is_file() {
            eprintln!("skipping: {} does not exist", path.display());
            return;
        }
        // 39 px filmstrip at 71.5 px/s: one 16:9 tile every 0.970 s.
        let interval = (TICKS_PER_SECOND as f64 * 39.0 * 16.0 / (9.0 * 71.5)).round() as i64;
        let positions: Vec<Tick> = (0..62).map(|index| Tick(interval * index)).collect();
        let cancelled = Arc::new(AtomicBool::new(false));

        let (mut open, mut seek, mut decode, mut scale) = (
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );
        for at in &positions {
            let mark = Instant::now();
            let mut reader = VideoReader::open_thumbnail(&path, cancelled.clone()).unwrap();
            open += mark.elapsed();
            let mark = Instant::now();
            if *at > Tick::ZERO {
                reader.seek(*at).unwrap();
            }
            seek += mark.elapsed();
            let mark = Instant::now();
            let frame = reader.next_frame().unwrap().expect("first frame");
            decode += mark.elapsed();
            let mark = Instant::now();
            from_frame(&frame, *at, THUMBNAIL_MAX_EDGE).unwrap();
            scale += mark.elapsed();
        }
        eprintln!(
            "per-tile breakdown: open={:.2?} seek={:.2?} decode={:.2?} scale={:.2?} total={:.2?}",
            open,
            seek,
            decode,
            scale,
            open + seek + decode + scale
        );

        let mark = Instant::now();
        for at in &positions {
            thumbnail_at(&path, *at, THUMBNAIL_MAX_EDGE, cancelled.clone()).unwrap();
        }
        eprintln!("per-tile public path: {:.2?}", mark.elapsed());

        let mut decode_only = Duration::ZERO;
        let mut pass_scale = Duration::ZERO;
        let mark = Instant::now();
        let mut reader = VideoReader::open_thumbnail(&path, cancelled.clone()).unwrap();
        if positions[0] > Tick::ZERO {
            reader.seek(positions[0]).unwrap();
        }
        let mut next = 0;
        let mut frames = 0_u64;
        while next < positions.len() {
            let step = Instant::now();
            let Some(frame) = reader.next_frame().unwrap() else {
                panic!("sample ended before the range")
            };
            decode_only += step.elapsed();
            frames += 1;
            let pts = frame.pts().unwrap().expect("frame pts");
            while next < positions.len() && pts >= positions[next] {
                let step = Instant::now();
                from_frame(&frame, positions[next], THUMBNAIL_MAX_EDGE).unwrap();
                pass_scale += step.elapsed();
                next += 1;
            }
        }
        eprintln!(
            "one sequential pass: decode_only={decode_only:.2?} scale={pass_scale:.2?} total={:.2?} frames={frames}",
            mark.elapsed()
        );

        let mark = Instant::now();
        let mut samples = Vec::with_capacity(positions.len());
        thumbnails_at(
            &path,
            &positions,
            THUMBNAIL_MAX_EDGE,
            cancelled.clone(),
            |at, image| samples.push((at, image)),
        )
        .unwrap();
        eprintln!(
            "thumbnails_at pass: {:.2?} tiles={}",
            mark.elapsed(),
            samples.len()
        );
        assert_eq!(samples.len(), positions.len());
        assert!(
            samples
                .iter()
                .map(|(at, _)| *at)
                .eq(positions.iter().copied())
        );
    }
}
