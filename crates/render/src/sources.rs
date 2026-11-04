use crate::{
    AudioBlock, Error, ExecutionHooks, Image, MemoryKind, Reservation, Result, UnavailableReason,
    check,
};
use media::{
    AssetId, AudioDecoder, AudioFrame, AudioResampler, AudioSampleFormat, PacketReader,
    RgbaConverter, SourceAsset, StreamKind, VideoFrame, VideoReader,
};
use project::ItemId;
use std::collections::{BTreeMap, VecDeque};
use time::{Rounding, Tick};
/// Domain-owned injection seam, not another job/cache runtime. Audio requests use
/// absolute source sample coordinates at the requested output rate.
pub trait SourceProvider {
    fn status(
        &mut self,
        asset: &SourceAsset,
        hash: bool,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>>;
    fn begin_video(&mut self, _active: &[ItemId]) {}
    fn begin_audio(&mut self, _active: &[ItemId]) {}
    fn finish_audio(&mut self, _item: ItemId) {}
    fn video(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        at: Tick,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>>;
    /// A full-coordinate image whose pixels outside the requested horizontal
    /// window may be transparent. Implementations may return the full image.
    /// Consumers must sample only the requested columns; reduction and blur use
    /// the ordinary full-frame entry point.
    fn video_window(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        at: Tick,
        _window: Option<std::ops::Range<u32>>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        self.video(asset, item, at, hooks)
    }
    fn audio(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock>;
}
struct Signature {
    asset: SourceAsset,
    file: (u64, Option<std::time::SystemTime>),
}
#[derive(Default)]
pub struct FileSources {
    signatures: BTreeMap<AssetId, Signature>,
    videos: BTreeMap<ItemId, VideoCursor>,
    audios: BTreeMap<ItemId, AudioCursor>,
}
impl SourceProvider for FileSources {
    fn status(
        &mut self,
        asset: &SourceAsset,
        hash: bool,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        check(hooks)?;
        let meta = match std::fs::metadata(asset.path()) {
            Ok(m) if m.is_file() => m,
            Ok(_) => return Ok(Some(UnavailableReason::Missing)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some(UnavailableReason::Missing));
            }
            Err(e) => return Err(e.into()),
        };
        let signature = (meta.len(), meta.modified().ok());
        if hash
            || self
                .signatures
                .get(&asset.id())
                .is_none_or(|s| s.asset != *asset || s.file != signature)
        {
            if media::ContentHash::read_with_cancel(asset.path(), || hooks.cancelled())?
                != asset.content_hash()
            {
                return Ok(Some(UnavailableReason::Changed));
            }
            let _probe = hooks.reserve(MemoryKind::Decoded, 256 << 20)?;
            let live = media::probe(asset.path())?;
            if asset.probe().duration.is_some() && live.duration != asset.probe().duration {
                return Ok(Some(UnavailableReason::Metadata));
            }
            for kind in [StreamKind::Video, StreamKind::Audio] {
                if let Some(saved) = asset.probe().streams.iter().find(|s| s.kind == kind) {
                    let actual = live.streams.iter().find(|s| s.kind == kind);
                    let Some(actual) = actual else {
                        return Ok(Some(if kind == StreamKind::Video {
                            UnavailableReason::Video
                        } else {
                            UnavailableReason::Audio
                        }));
                    };
                    if actual.index != saved.index
                        || actual.codec != saved.codec
                        || actual.video != saved.video
                        || actual.audio != saved.audio
                        || saved
                            .frame_rate
                            .is_some_and(|rate| actual.frame_rate != Some(rate))
                    {
                        return Ok(Some(UnavailableReason::Metadata));
                    }
                }
            }
            if self.signatures.len() >= 10000 && !self.signatures.contains_key(&asset.id()) {
                self.signatures.clear();
            }
            self.signatures.insert(
                asset.id(),
                Signature {
                    asset: asset.clone(),
                    file: signature,
                },
            );
        }
        check(hooks)?;
        Ok(None)
    }
    fn begin_video(&mut self, active: &[ItemId]) {
        self.videos.retain(|id, _| active.contains(id));
    }
    fn begin_audio(&mut self, active: &[ItemId]) {
        self.audios.retain(|id, _| active.contains(id));
    }
    fn finish_audio(&mut self, item: ItemId) {
        self.audios.remove(&item);
    }
    fn video(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        at: Tick,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        self.video_window(asset, item, at, None, hooks)
    }
    fn video_window(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        at: Tick,
        window: Option<std::ops::Range<u32>>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        if self.videos.get(&item).is_none_or(|c| c.asset != *asset) {
            self.videos.remove(&item);
            self.videos
                .insert(item, VideoCursor::new(asset, item, hooks)?);
        }
        self.videos
            .get_mut(&item)
            .ok_or(Error::Invalid("video cursor"))?
            .image(at, window, hooks)
    }
    fn audio(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let reset = self
            .audios
            .get(&item)
            .is_none_or(|a| a.rate != rate || start < a.next_request || a.asset != *asset);
        if reset {
            self.audios.remove(&item);
            self.audios
                .insert(item, AudioCursor::new(asset, rate, hooks)?);
        }
        self.audios
            .get_mut(&item)
            .ok_or(Error::Invalid("audio cursor"))?
            .read(start, frames, hooks)
    }
}
struct VideoCursor {
    asset: SourceAsset,
    item: ItemId,
    eof: bool,
    end: Option<Tick>,
    step: Tick,
    reader: VideoReader,
    converter: RgbaConverter,
    current: Option<VideoFrame>,
    next: Option<VideoFrame>,
    last: Option<Tick>,
    _lease: Box<dyn Reservation>,
}
impl VideoCursor {
    fn new(asset: &SourceAsset, item: ItemId, hooks: &dyn ExecutionHooks) -> Result<Self> {
        let probe_guard = hooks.reserve(MemoryKind::Decoded, 256 << 20)?;
        let probe = media::probe(asset.path())?;
        let video = probe
            .streams
            .iter()
            .find_map(|s| s.video.as_ref())
            .ok_or(Error::Unsupported("video stream missing"))?;
        if let Some(stream) = probe.streams.iter().find(|s| s.kind == StreamKind::Video)
            && stream.parameters.display_transform_required()?
        {
            return Err(Error::Unsupported(
                "non-square pixels or display matrix require explicit conversion",
            ));
        }
        let color = video.color.resolved_sdr();
        if color.is_hdr()
            || color.primaries != 1
            || !matches!(color.transfer, 1 | 8 | 13)
            || video.bit_depth.is_some_and(|n| n > 8)
        {
            return Err(Error::Unsupported(
                "video requires HDR/primary/depth conversion or passthrough",
            ));
        }
        let stream = probe
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(Error::Invalid("video stream"))?;
        let step = stream
            .frame_rate
            .and_then(|r| r.frame_duration().ok())
            .filter(|d| *d > Tick::ZERO)
            .unwrap_or(Tick(time::TICKS_PER_SECOND / 30));
        let end = stream
            .duration
            .and_then(|d| stream.start.unwrap_or(Tick::ZERO).checked_add(d))
            .or(probe.duration);
        Image::bytes(video.width, video.height)?;
        let bytes = u64::from(video.width).div_ceil(512)
            * 512
            * u64::from(video.height).div_ceil(64)
            * 64
            * 4
            * 4
            + (96 << 20);
        drop(probe_guard);
        let (threads, lease) = crate::conversion_reservation(hooks, MemoryKind::Decoded, bytes)?;
        Ok(Self {
            asset: asset.clone(),
            item,
            eof: false,
            end,
            step,
            reader: VideoReader::open(asset.path())?,
            converter: RgbaConverter::with_threads(threads)?,
            current: None,
            next: None,
            last: None,
            _lease: lease,
        })
    }
    fn image(
        &mut self,
        at: Tick,
        window: Option<std::ops::Range<u32>>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        check(hooks)?;
        if self.last.is_none_or(|last| at < last) {
            self.reader.seek_keyframe(at)?;
            self.eof = false;
            self.current = None;
            self.next = None;
        }
        self.last = Some(at);
        loop {
            if self.next.is_none() {
                self.next = self.reader.next_frame()?;
                self.eof = self.next.is_none();
            }
            let Some(next) = self.next.as_ref() else {
                break;
            };
            let pts = next
                .pts()?
                .ok_or(Error::Unsupported("video PTS unavailable"))?;
            if pts > at {
                break;
            }
            self.current = self.next.take();
            check(hooks)?;
        }
        let boundary = self.end.or(self
            .current
            .as_ref()
            .and_then(|f| f.pts().ok().flatten())
            .and_then(|t| t.checked_add(self.step)));
        if self.eof && (self.current.is_none() || boundary.is_some_and(|end| at > end)) {
            let end = at
                .checked_add(Tick(1))
                .ok_or(Error::Invalid("unavailable source range"))?;
            return Err(Error::Unavailable(vec![crate::Unavailable {
                asset: self.asset.id(),
                item: self.item,
                range: time::TimeRange::new(at, end)?,
                reason: UnavailableReason::Video,
            }]));
        }
        let Some(frame) = self.current.as_ref() else {
            return Ok(None);
        };
        let actual = frame.color().resolved_sdr();
        if actual.primaries != 1 || actual.is_hdr() || frame.bit_depth().is_some_and(|n| n > 8) {
            return Err(Error::Unsupported(
                "runtime video color/depth change requires conversion",
            ));
        }
        let shape_matches = self
            .asset
            .probe()
            .streams
            .iter()
            .find_map(|s| s.video.as_ref())
            .is_some_and(|v| v.width == frame.width() && v.height == frame.height());
        let (rgba, origin) = if let Some(window) = window.filter(|_| shape_matches) {
            self.converter.convert_horizontal(frame, window)?
        } else {
            (self.converter.convert(frame)?, 0)
        };
        let color = rgba.color().resolved_sdr();
        let stride = rgba.stride(0)?;
        Ok(Some(Image::from_rgba8_strided_at(
            rgba.width(),
            rgba.height(),
            rgba.plane(0)?,
            stride,
            rgba.premultiplied_alpha(),
            color.transfer,
            Some((frame.width(), origin)),
            hooks,
        )?))
    }
}
struct AudioCursor {
    asset: SourceAsset,
    reader: PacketReader,
    decoder: AudioDecoder,
    index: usize,
    rate: u32,
    channels: u16,
    source_rate: u32,
    resampler: Option<AudioResampler>,
    converted: VecDeque<AudioFrame>,
    current: Option<(i64, Vec<Vec<f32>>)>,
    eof: bool,
    next_request: i64,
    _lease: Box<dyn Reservation>,
}
impl AudioCursor {
    fn new(asset: &SourceAsset, rate: u32, hooks: &dyn ExecutionHooks) -> Result<Self> {
        let probe_guard = hooks.reserve(MemoryKind::Decoded, 256 << 20)?;
        let reader = PacketReader::open(asset.path())?;
        let stream = reader
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Audio)
            .ok_or(Error::Unsupported("audio stream missing"))?;
        let info = stream
            .audio
            .as_ref()
            .ok_or(Error::Unsupported("audio metadata"))?;
        if !matches!(info.channels, 1 | 2) || !(8000..=192000).contains(&info.sample_rate) {
            return Err(Error::Unsupported(
                "source audio format; only mono/stereo 8-192 kHz admitted",
            ));
        }
        let expansion = u64::from(rate).div_ceil(u64::from(info.sample_rate));
        let bytes = 65536 * u64::from(info.channels) * 8 * (1 + expansion * 2) + (96 << 20);
        let source_rate = info.sample_rate;
        let channels = info.channels;
        drop(probe_guard);
        let lease = hooks.reserve(MemoryKind::Decoded, bytes)?;
        let decoder = AudioDecoder::new(stream)?;
        let index = stream.index;
        Ok(Self {
            asset: asset.clone(),
            reader,
            decoder,
            index,
            rate,
            channels,
            source_rate,
            resampler: None,
            converted: VecDeque::new(),
            current: None,
            eof: false,
            next_request: 0,
            _lease: lease,
        })
    }
    fn advance(&mut self, hooks: &dyn ExecutionHooks) -> Result<bool> {
        loop {
            check(hooks)?;
            if let Some(frame) = self.converted.pop_front() {
                let start = frame
                    .pts()?
                    .ok_or(Error::Unsupported("audio PTS"))?
                    .to_units(i64::from(self.rate), Rounding::Nearest)?;
                self.current = Some((start, frame.planar_f32()?));
                return Ok(true);
            }
            if let Some(frame) = self.decoder.pull_frame()? {
                if frame.channels() != self.channels || frame.sample_rate() != self.source_rate {
                    return Err(Error::Unsupported("runtime audio rate/layout changed"));
                }
                if self.resampler.is_none() {
                    self.resampler = Some(AudioResampler::new(
                        &frame,
                        self.rate,
                        self.channels,
                        AudioSampleFormat::PlanarF32,
                    )?);
                }
                self.converted.extend(
                    self.resampler
                        .as_mut()
                        .ok_or(Error::Invalid("resampler"))?
                        .convert(&frame)?,
                );
                continue;
            }
            if self.eof {
                if let Some(resampler) = self.resampler.as_mut()
                    && let Some(frame) = resampler.flush()?
                {
                    self.converted.push_back(frame);
                    continue;
                }
                self.current = None;
                return Ok(false);
            }
            match self.reader.next_packet()? {
                Some(packet) if packet.stream() == self.index => {
                    self.decoder.push_packet(&packet)?
                }
                Some(_) => {}
                None => {
                    self.eof = true;
                    self.decoder.finish_input()?;
                }
            }
        }
    }
    fn read(
        &mut self,
        start: i64,
        frames: usize,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let end = start
            .checked_add(frames as i64)
            .ok_or(Error::Invalid("audio read end"))?;
        let mut out =
            AudioBlock::new(self.rate, self.channels, frames, MemoryKind::Decoded, hooks)?;
        self.next_request = end;
        let mut position = start;
        while position < end {
            check(hooks)?;
            if self.current.is_none() && !self.advance(hooks)? {
                break;
            }
            let Some((origin, planes)) = self.current.as_ref() else {
                break;
            };
            let count = planes.first().map_or(0, Vec::len);
            let limit = origin
                .checked_add(count as i64)
                .ok_or(Error::Invalid("audio source end"))?;
            if position >= limit {
                self.current = None;
                continue;
            }
            if position < *origin {
                position = end.min(*origin);
                continue;
            }
            let take = (end.min(limit) - position) as usize;
            let from = (position - origin) as usize;
            let offset = (position - start) as usize;
            for (dst, src) in out.planes.iter_mut().zip(planes) {
                dst.get_mut(offset..offset + take)
                    .ok_or(Error::Invalid("audio output slice"))?
                    .copy_from_slice(
                        src.get(from..from + take)
                            .ok_or(Error::Invalid("audio source slice"))?,
                    );
            }
            position += take as i64;
        }
        Ok(out)
    }
}
