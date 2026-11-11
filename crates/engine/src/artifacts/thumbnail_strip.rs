//! Samples a clip's visible source range into cached thumbnail tiles.
use super::frame_thumbnails::{RetainedThumbnail, ThumbnailCache, ThumbnailLookup, thumbnail_key};
use crate::{
    EditError,
    jobs::{
        CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState,
        ResourceClass,
    },
};
use media::{SourceAsset, StreamKind, THUMBNAIL_MAX_EDGE, Thumbnail};
use std::collections::BTreeMap;
use time::{SourceRange, TICKS_PER_SECOND, Tick, TimeRange};

/// One tile is 16:9 wide: its width is the filmstrip height times 16/9 (the
/// implementation reference's `THUMBNAIL_ASPECT_RATIO = 16 / 9` in
/// `apps/web/src/timeline/components/timeline-element.tsx:97`, used as
/// `tileWidth = trackHeight * THUMBNAIL_ASPECT_RATIO` at `:1108`).
pub const TILE_ASPECT_RATIO: (u32, u32) = (16, 9);
/// One request or visible range never asks for more than this many tiles.
pub const MAX_TILES: usize = 128;
/// Measured filmstrip height in logical px: the 39 px strip inside a 74 px clip.
pub const STRIP_HEIGHT_PX: u32 = 39;
const MAGIC: &[u8; 8] = b"VTCSTR01";
const MAX_BYTES: usize = 64 << 20;
/// One decoded source frame with its RGBA conversion and converter scratch.
const DECODE_BYTES: u64 = 32 << 20;

/// Output ticks between two strip tiles: one 16:9 tile whose height is
/// `strip_height_px`, on a timeline drawn at `pixels_per_second`.
pub fn tile_interval(strip_height_px: u32, pixels_per_second: f64) -> Result<Tick, EditError> {
    if strip_height_px == 0 || !pixels_per_second.is_finite() || pixels_per_second <= 0.0 {
        return Err(EditError::Invalid("thumbnail strip interval"));
    }
    let ticks =
        TICKS_PER_SECOND as f64 * f64::from(strip_height_px) * f64::from(TILE_ASPECT_RATIO.0)
            / (f64::from(TILE_ASPECT_RATIO.1) * pixels_per_second);
    if !(1.0..=i64::MAX as f64).contains(&ticks) {
        return Err(EditError::Invalid("thumbnail strip interval"));
    }
    Ok(Tick(ticks.round() as i64))
}

/// One source range's strip samples, cached one frame per sampled position.
#[derive(Clone)]
pub struct ThumbnailStripRequest {
    asset: SourceAsset,
    positions: Vec<Tick>,
    max_edge: u32,
}

/// Retained runner job. `finish` publishes every decoded frame; cancellation
/// leaves the cache untouched and is retryable.
pub struct ThumbnailStripJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: ThumbnailStripRequest,
}

/// Positions whose frames the cache holds after `finish`.
pub struct ThumbnailStrip {
    positions: Vec<Tick>,
}

impl ThumbnailStrip {
    pub fn positions(&self) -> &[Tick] {
        &self.positions
    }
}

impl ThumbnailStripRequest {
    /// Admit a video or image source and the source range to sample. Sample
    /// positions lie on a grid anchored at the source origin, so overlapping
    /// requests share the existing thumbnail cache entry for a position.
    pub fn new(
        asset: SourceAsset,
        range: SourceRange,
        interval: Tick,
        max_edge: u32,
    ) -> Result<Self, EditError> {
        admit_grid(interval, max_edge)?;
        if range.start() < Tick::ZERO {
            return Err(EditError::Invalid("thumbnail strip source range"));
        }
        if !has_video(&asset) {
            return Err(EditError::Unsupported("thumbnail strip requires video"));
        }
        // A still image has one decodable frame; the strip repeats it.
        let positions = if asset.is_image() {
            vec![Tick::ZERO]
        } else {
            sample_positions(range, interval)?
        };
        Ok(Self {
            asset,
            positions,
            max_edge,
        })
    }

    /// Source positions this request caches, in ascending order.
    pub fn positions(&self) -> &[Tick] {
        &self.positions
    }

    /// Admit the source positions one clip's visible output range needs on the
    /// same grid `display` reads, mapping trim and retime. `pixels_per_second`
    /// is the timeline zoom; the interval is one 16:9 tile at
    /// [`STRIP_HEIGHT_PX`]. `visible` is clamped to the clip; a range outside
    /// it, a mismatched asset or a hostile tile count is `Invalid`.
    pub fn for_clip(
        asset: SourceAsset,
        item: &project::Item,
        visible: TimeRange,
        pixels_per_second: f64,
        max_edge: u32,
    ) -> Result<Self, EditError> {
        let interval = tile_interval(STRIP_HEIGHT_PX, pixels_per_second)?;
        admit_grid(interval, max_edge)?;
        if item.source_asset != Some(asset.id()) {
            return Err(EditError::Invalid("thumbnail strip display asset"));
        }
        if !has_video(&asset) {
            return Err(EditError::Unsupported("thumbnail strip requires video"));
        }
        let clip = TimeRange::new(item.start, item.end()?)?;
        let span = visible
            .intersect(clip)
            .ok_or(EditError::Invalid("thumbnail strip display outside clip"))?;
        let positions = if asset.is_image() {
            vec![Tick::ZERO]
        } else {
            let count = tile_count(span.duration(), interval)?;
            let source_end =
                item.source_range
                    .end()
                    .checked_sub(Tick(1))
                    .ok_or(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    ))?;
            let first = clip_position(item, span.start(), source_end, interval)?;
            let offset = i64::try_from(count - 1)
                .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?
                .checked_mul(interval.0)
                .ok_or(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ))?;
            let last = clip_position(
                item,
                Tick(
                    span.start()
                        .0
                        .checked_add(offset)
                        .ok_or(EditError::Control(
                            crate::session::wire::ControlError::Limit,
                        ))?,
                ),
                source_end,
                interval,
            )?;
            let end = last.checked_add(interval).ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
            sample_positions(SourceRange::new(first, end)?, interval)?
        };
        Ok(Self {
            asset,
            positions,
            max_edge,
        })
    }

    /// Enqueue one CPU job for the positions the cache lacks. A warm cache opens
    /// no source. Only `finish` publishes completed frames.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ThumbnailCache,
    ) -> Result<ThumbnailStripJob, EditError> {
        let mut pending = Vec::new();
        for at in &self.positions {
            match cache.lookup(&self.asset, *at, self.max_edge) {
                Ok(ThumbnailLookup::Hit(_)) => {}
                Ok(ThumbnailLookup::Missing | ThumbnailLookup::Corrupt) => pending.push(*at),
                Err(error) => return Err(error),
            }
        }
        let input = thumbnail_key(
            self.asset.content_hash(),
            self.positions.first().copied().unwrap_or(Tick::ZERO),
            self.max_edge,
        )?;
        let request = self.clone();
        let spec = JobSpec {
            kind: JobKind::GenerateClipThumbnails,
            class: ResourceClass::Cpu,
            input,
            bytes: JobBytes {
                decoded_frames: DECODE_BYTES,
                result: MAX_BYTES as u64,
                ..Default::default()
            },
        };
        let total = pending.len().max(1) as f32;
        let (id, cancel) = runner.submit(spec, move |context| {
            context.check_cancelled()?;
            let mut samples = Vec::with_capacity(pending.len());
            if !pending.is_empty() {
                context.progress(JobStage::Thumbnail, 0.0)?;
                // One sequential pass: the decoder opens and seeks once, then
                // takes the first frame at or after each missing position.
                media::thumbnails_at(
                    request.asset.path(),
                    &pending,
                    request.max_edge,
                    context.media_cancel_flag(),
                    |at, image| {
                        samples.push((at, image));
                        context
                            .progress(JobStage::Thumbnail, samples.len() as f32 / total)
                            .ok();
                    },
                )?;
            }
            context.check_cancelled()?;
            context.progress(JobStage::Thumbnail, 1.0)?;
            encode(&samples)
        })?;
        Ok(ThumbnailStripJob {
            id,
            cancel,
            request: self.clone(),
        })
    }

    fn decode(&self, bytes: &[u8]) -> Result<Vec<(Tick, Thumbnail)>, EditError> {
        if bytes.len() > MAX_BYTES || bytes.get(..8) != Some(MAGIC) {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let count = read_u32(bytes, 8)?;
        let count = usize::try_from(count)
            .map_err(|_| EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
        if count > self.positions.len() {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let mut samples = Vec::with_capacity(count);
        let mut offset = 12_usize;
        for _ in 0..count {
            let at = Tick(read_i64(bytes, offset)?);
            offset = offset
                .checked_add(8)
                .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
            let length = read_u32(bytes, offset)?;
            offset = offset
                .checked_add(4)
                .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
            let end = offset
                .checked_add(
                    usize::try_from(length)
                        .map_err(|_| EditError::Runtime(crate::RuntimeError::CacheCorrupt))?,
                )
                .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
            let payload = bytes
                .get(offset..end)
                .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
            offset = end;
            let image = Thumbnail::decode(payload)
                .map_err(|_| EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
            if image.width > self.max_edge
                || image.height > self.max_edge
                || self.positions.binary_search(&at).is_err()
            {
                return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
            }
            samples.push((at, image));
        }
        if offset != bytes.len() {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        Ok(samples)
    }
}

impl ThumbnailStripJob {
    /// Validate and publish every decoded frame through the existing thumbnail
    /// cache; typed terminal failures and cancellation propagate.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ThumbnailCache,
    ) -> Result<Option<ThumbnailStrip>, EditError> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => Err(EditError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(EditError::Runtime(crate::RuntimeError::Failed(
                snapshot
                    .error
                    .ok_or(EditError::Session(crate::SessionError::StaleResult))?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let samples = self.request.decode(bytes)?;
                for (at, image) in &samples {
                    cache.put(&self.request.asset, *at, self.request.max_edge, image)?;
                }
                runner
                    .take_output(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                Ok(Some(ThumbnailStrip {
                    positions: self.request.positions.clone(),
                }))
            }
            _ => Ok(None),
        }
    }
}

/// One cached frame for one output-time slice of a clip's visible range.
#[derive(Debug)]
pub struct StripTile {
    /// Output-time slice this tile covers.
    pub output: TimeRange,
    /// Source position of the cached frame.
    pub at: Tick,
    pub image: RetainedThumbnail,
}

/// Tiles for a clip's visible output range, in output order.
#[derive(Debug)]
pub struct ThumbnailStripDisplay {
    tiles: Vec<StripTile>,
}

impl ThumbnailStripDisplay {
    pub fn tiles(&self) -> &[StripTile] {
        &self.tiles
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

/// Cache-only answer for one clip's visible range. Never decodes media.
#[derive(Debug)]
pub enum ThumbnailStripState {
    /// Every tile's frame is cached.
    Ready(ThumbnailStripDisplay),
    /// At least one tile's frame has not been generated yet.
    Pending,
    /// A cached frame cannot be decoded and must be regenerated.
    Failed(EditError),
    /// The source has no video stream to sample.
    Unsupported,
}

/// Map one clip's visible output range through trim and retime and look up one
/// cached frame per 16:9 tile. Reads only the thumbnail cache.
///
/// `pixels_per_second` is the timeline zoom; the interval is one 16:9 tile at
/// [`STRIP_HEIGHT_PX`]. `visible` is clamped to the clip. A range outside the
/// clip, a mismatched asset, a non-positive or non-finite zoom or more than
/// [`MAX_TILES`] tiles is `Invalid`; a missing entry is `Pending`; an
/// undecodable one is `Failed`.
pub fn display(
    asset: &SourceAsset,
    item: &project::Item,
    visible: TimeRange,
    pixels_per_second: f64,
    max_edge: u32,
    cache: &mut ThumbnailCache,
) -> Result<ThumbnailStripState, EditError> {
    let interval = tile_interval(STRIP_HEIGHT_PX, pixels_per_second)?;
    admit_grid(interval, max_edge)?;
    if item.source_asset != Some(asset.id()) {
        return Err(EditError::Invalid("thumbnail strip display asset"));
    }
    if !has_video(asset) {
        return Ok(ThumbnailStripState::Unsupported);
    }
    let clip = TimeRange::new(item.start, item.end()?)?;
    let span = visible
        .intersect(clip)
        .ok_or(EditError::Invalid("thumbnail strip display outside clip"))?;
    let count = tile_count(span.duration(), interval)?;
    let source_end = item
        .source_range
        .end()
        .checked_sub(Tick(1))
        .ok_or(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ))?;
    let mut tiles = Vec::with_capacity(count);
    let mut fetched: BTreeMap<Tick, RetainedThumbnail> = BTreeMap::new();
    for index in 0..count {
        let offset = i64::try_from(index)
            .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?
            .checked_mul(interval.0)
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
        let start = Tick(
            span.start()
                .0
                .checked_add(offset)
                .ok_or(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ))?,
        );
        let end = Tick(span.end().0.min(start.0.checked_add(interval.0).ok_or(
            EditError::Control(crate::session::wire::ControlError::Limit),
        )?));
        let output = TimeRange::new(start, end)?;
        let at = if asset.is_image() {
            Tick::ZERO
        } else {
            clip_position(item, start, source_end, interval)?
        };
        let image = match fetched.get(&at) {
            Some(image) => image.clone(),
            None => match cache.lookup(asset, at, max_edge)? {
                ThumbnailLookup::Hit(image) => {
                    fetched.insert(at, image.clone());
                    image
                }
                ThumbnailLookup::Missing => return Ok(ThumbnailStripState::Pending),
                ThumbnailLookup::Corrupt => {
                    return Ok(ThumbnailStripState::Failed(EditError::Runtime(
                        crate::RuntimeError::CacheCorrupt,
                    )));
                }
            },
        };
        tiles.push(StripTile { output, at, image });
    }
    Ok(ThumbnailStripState::Ready(ThumbnailStripDisplay { tiles }))
}

fn admit_grid(interval: Tick, max_edge: u32) -> Result<(), EditError> {
    if interval <= Tick::ZERO {
        return Err(EditError::Invalid("thumbnail strip interval"));
    }
    if max_edge == 0 || max_edge > THUMBNAIL_MAX_EDGE {
        return Err(EditError::Invalid("thumbnail max edge"));
    }
    Ok(())
}

fn has_video(asset: &SourceAsset) -> bool {
    asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Video)
}

/// The grid position `display` reads for one output tick of a clip.
fn clip_position(
    item: &project::Item,
    output: Tick,
    source_end: Tick,
    interval: Tick,
) -> Result<Tick, EditError> {
    let source = item
        .source_time_at(output)?
        .max(item.source_range.start())
        .min(source_end);
    floor(source, interval)
}

fn sample_positions(range: SourceRange, interval: Tick) -> Result<Vec<Tick>, EditError> {
    let last = range.end().checked_sub(Tick(1)).ok_or(EditError::Control(
        crate::session::wire::ControlError::Limit,
    ))?;
    let first = floor(range.start(), interval)?;
    let last = floor(last, interval)?;
    let count = last
        .0
        .checked_sub(first.0)
        .ok_or(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ))?
        .checked_div(interval.0)
        .ok_or(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ))?
        .checked_add(1)
        .ok_or(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ))?;
    let count = usize::try_from(count)
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    if count == 0 || count > MAX_TILES {
        return Err(EditError::Invalid("thumbnail strip tiles"));
    }
    let mut positions = Vec::with_capacity(count);
    let mut at = first;
    for _ in 0..count {
        positions.push(at);
        at = at.checked_add(interval).ok_or(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ))?;
    }
    Ok(positions)
}

fn tile_count(duration: Tick, interval: Tick) -> Result<usize, EditError> {
    if duration <= Tick::ZERO || interval <= Tick::ZERO {
        return Err(EditError::Invalid("thumbnail strip tiles"));
    }
    let count = duration.0.saturating_sub(1) / interval.0 + 1;
    let count = usize::try_from(count)
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    if count == 0 || count > MAX_TILES {
        return Err(EditError::Invalid("thumbnail strip tiles"));
    }
    Ok(count)
}

/// Nearest sample position at or before `at` on the source-origin grid.
fn floor(at: Tick, interval: Tick) -> Result<Tick, EditError> {
    let step = i128::from(interval.0);
    let value = i128::from(at.0).div_euclid(step);
    let floored = value.checked_mul(step).ok_or(EditError::Control(
        crate::session::wire::ControlError::Limit,
    ))?;
    Ok(Tick(i64::try_from(floored).map_err(|_| {
        EditError::Control(crate::session::wire::ControlError::Limit)
    })?))
}

fn encode(samples: &[(Tick, Thumbnail)]) -> Result<Vec<u8>, EditError> {
    let count = u32::try_from(samples.len())
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&count.to_le_bytes());
    for (at, image) in samples {
        let payload = image.encode();
        let length = u32::try_from(payload.len())
            .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
        let next = bytes
            .len()
            .checked_add(16)
            .and_then(|value| value.checked_add(payload.len()))
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
        if next > MAX_BYTES {
            return Err(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        bytes.extend_from_slice(&at.0.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&payload);
    }
    Ok(bytes)
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, EditError> {
    let end = at
        .checked_add(4)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
    let value = bytes
        .get(at..end)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
    let mut buffer = [0u8; 4];
    buffer.copy_from_slice(value);
    Ok(u32::from_le_bytes(buffer))
}

fn read_i64(bytes: &[u8], at: usize) -> Result<i64, EditError> {
    let end = at
        .checked_add(8)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
    let value = bytes
        .get(at..end)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
    let mut buffer = [0u8; 8];
    buffer.copy_from_slice(value);
    Ok(i64::from_le_bytes(buffer))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn request() -> ThumbnailStripRequest {
        let asset = testkit::source::asset(1, std::env::temp_dir().join("fixture.mp4"))
            .format("mov,mp4")
            .duration(Tick(TICKS_PER_SECOND))
            .hash([7; 32])
            .stream(
                testkit::source::video()
                    .duration(Tick(TICKS_PER_SECOND))
                    .build(),
            )
            .build();
        ThumbnailStripRequest::new(
            asset,
            SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)).unwrap(),
            Tick(TICKS_PER_SECOND / 4),
            THUMBNAIL_MAX_EDGE,
        )
        .unwrap()
    }

    fn sample(at: i64) -> Thumbnail {
        Thumbnail {
            width: 2,
            height: 1,
            frame: Tick(at),
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        }
    }

    fn seconds(count: i64) -> Tick {
        Tick(count * TICKS_PER_SECOND)
    }

    /// Zoom whose one-tile interval is exactly one output second at 39 px.
    const ONE_SECOND_PPS: f64 = 69.333_333_333_333_33;

    fn clip(source: Tick, start: Tick) -> project::Item {
        project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick::ZERO, source).unwrap(),
            start,
            content: project::ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.0,
                muted: false,
                crop: None,
            }),
        }
    }

    #[test]
    fn clip_requests_warm_the_positions_the_display_reads() {
        let asset = testkit::source::asset(1, std::env::temp_dir().join("fixture.mp4"))
            .format("mov,mp4")
            .duration(Tick(3 * TICKS_PER_SECOND))
            .hash([7; 32])
            .stream(
                testkit::source::video()
                    .duration(Tick(3 * TICKS_PER_SECOND))
                    .build(),
            )
            .build();
        let item = clip(Tick(3 * TICKS_PER_SECOND), Tick::ZERO);
        let second = Tick(TICKS_PER_SECOND);
        let visible = |start: Tick, end: Tick| TimeRange::new(start, end).unwrap();
        let request = ThumbnailStripRequest::for_clip(
            asset.clone(),
            &item,
            visible(Tick::ZERO, seconds(2)),
            ONE_SECOND_PPS,
            THUMBNAIL_MAX_EDGE,
        )
        .unwrap();
        assert_eq!(request.positions(), &[Tick::ZERO, second]);
        let trimmed = ThumbnailStripRequest::for_clip(
            asset.clone(),
            &item,
            visible(second, seconds(3)),
            ONE_SECOND_PPS,
            THUMBNAIL_MAX_EDGE,
        )
        .unwrap();
        assert_eq!(trimmed.positions(), &[second, seconds(2)]);
        // Outside the clip, a mismatched asset, a hostile tile count and an
        // invalid grid refuse.
        assert!(
            ThumbnailStripRequest::for_clip(
                asset.clone(),
                &item,
                visible(seconds(5), seconds(6)),
                ONE_SECOND_PPS,
                THUMBNAIL_MAX_EDGE
            )
            .is_err()
        );
        let mut other = item.clone();
        other.source_asset = Some(media::AssetId(2));
        assert!(
            ThumbnailStripRequest::for_clip(
                asset.clone(),
                &other,
                visible(Tick::ZERO, second),
                ONE_SECOND_PPS,
                THUMBNAIL_MAX_EDGE
            )
            .is_err()
        );
        let long = clip(Tick(300 * TICKS_PER_SECOND), Tick::ZERO);
        assert!(
            ThumbnailStripRequest::for_clip(
                asset.clone(),
                &long,
                visible(Tick::ZERO, seconds(300)),
                ONE_SECOND_PPS,
                THUMBNAIL_MAX_EDGE
            )
            .is_err()
        );
        assert!(
            ThumbnailStripRequest::for_clip(
                asset.clone(),
                &item,
                visible(Tick::ZERO, seconds(3)),
                10_000.0,
                THUMBNAIL_MAX_EDGE
            )
            .is_err()
        );
        assert!(
            ThumbnailStripRequest::for_clip(
                asset.clone(),
                &item,
                visible(Tick::ZERO, second),
                0.0,
                THUMBNAIL_MAX_EDGE
            )
            .is_err()
        );
        assert!(
            ThumbnailStripRequest::for_clip(
                asset,
                &item,
                visible(Tick::ZERO, second),
                f64::NAN,
                THUMBNAIL_MAX_EDGE
            )
            .is_err()
        );
    }

    #[test]
    fn interval_is_one_sixteen_by_nine_tile_of_the_strip_height() {
        // 39 px high at 71.5 px per output second: 69.33 px wide, 0.97 s per tile.
        assert_eq!(tile_interval(39, 71.5).unwrap(), Tick(246_318_545_455));
        assert!(tile_interval(0, 71.5).is_err());
        assert!(tile_interval(39, 0.0).is_err());
        assert!(tile_interval(39, f64::NAN).is_err());
    }

    #[test]
    fn manifest_round_trips_and_rejects_hostile_bytes() {
        let request = request();
        let positions = request.positions();
        let samples = vec![(positions[0], sample(0)), (positions[1], sample(1))];
        let bytes = encode(&samples).unwrap();
        let decoded = request.decode(&bytes).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, positions[0]);
        assert_eq!(decoded[1].1, sample(1));

        assert!(matches!(
            request.decode(b"short"),
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
        ));
        let mut wrong_magic = bytes.clone();
        if let Some(byte) = wrong_magic.get_mut(0) {
            *byte ^= 0xff;
        }
        assert!(matches!(
            request.decode(&wrong_magic),
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
        ));
        let mut oversized = bytes.clone();
        if let Some(slot) = oversized.get_mut(8..12) {
            slot.copy_from_slice(&u32::MAX.to_le_bytes());
        }
        assert!(matches!(
            request.decode(&oversized),
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
        ));
        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() - 1);
        assert!(matches!(
            request.decode(&truncated),
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
        ));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            request.decode(&trailing),
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
        ));
        let unknown = vec![(Tick(1), sample(1))];
        let unknown = encode(&unknown).unwrap();
        assert!(matches!(
            request.decode(&unknown),
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
        ));
    }
}
