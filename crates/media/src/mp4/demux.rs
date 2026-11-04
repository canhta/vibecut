//! MP4/MOV demuxing: top-level box scan, `moov` sample tables and streamed packets.
//!
//! Only box headers and the `moov` box are read up front. Sample payloads are read
//! one at a time through [`ByteSource::read_at`], so a file larger than memory is
//! fine as long as its sample table fits the configured limits.

use std::fs::File;
use std::io;
use std::path::Path;

use time::{Rounding, Tick};

use super::boxes::{BoxHeader, Reader, boxes, find};
use super::codec::{SampleEntry, parse_sample_entry};
use super::{Edit, Probe, StreamInfo, StreamKind, VideoInfo};
use crate::{MediaError, Result};

/// Largest packet [`PacketReader`] will allocate.
pub const MAX_PACKET_BYTES: usize = 64 << 20;

/// Hard cap on the sample count of the whole file.
const MAX_SAMPLES: usize = 1 << 26;
/// Samples any file may hold, however short.
const MIN_SAMPLE_ALLOWANCE: usize = 1 << 20;
/// Bytes of file each sample needs beyond the minimum allowance.
const FILE_BYTES_PER_SAMPLE: u64 = 4;
/// Hard cap on an in-memory `moov` box.
const MAX_HEADER_BOX: u64 = 1 << 30;
/// Cap on the `ftyp` box.
const MAX_FTYP: u64 = 1 << 16;

/// Random-access read-only bytes: a file or an in-memory buffer.
pub(crate) trait ByteSource {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// Fill `buf` entirely starting at `offset`.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl ByteSource for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset)
            .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "offset past end"))?;
        let end = start
            .checked_add(buf.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "range past end"))?;
        let slice = self
            .get(start..end)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "range past end"))?;
        buf.copy_from_slice(slice);
        Ok(())
    }
}

impl ByteSource for File {
    fn len(&self) -> u64 {
        self.metadata().map(|meta| meta.len()).unwrap_or(0)
    }

    #[cfg(unix)]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = self.seek_read(buf, offset)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "range past end",
                ));
            }
            offset += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = self;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}

/// One sample (access unit) of a track, in the track's media timescale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Sample {
    offset: u64,
    size: u32,
    dts: i64,
    pts: i64,
    duration: u32,
    sync: bool,
}

/// A parsed track with its sample table.
#[derive(Clone, Debug, PartialEq)]
struct Track {
    id: u32,
    kind: StreamKind,
    timescale: u32,
    duration: u64,
    entry: SampleEntry,
    samples: Vec<Sample>,
    edits: Vec<Edit>,
}

/// A parsed file: headers and sample tables only; media stays in the source.
#[derive(Clone, Debug, PartialEq)]
struct Mp4File {
    brand: super::FourCc,
    is_quicktime: bool,
    timescale: u32,
    duration: u64,
    tracks: Vec<Track>,
}

impl Mp4File {
    fn probe(&self) -> Probe {
        Probe {
            brand: self.brand,
            timescale: self.timescale,
            duration: ticks_saturating(self.duration, self.timescale),
            streams: self.tracks.iter().map(stream_info).collect(),
        }
    }
}

fn stream_info(track: &Track) -> StreamInfo {
    let vfr = track.kind == StreamKind::Video
        && track
            .samples
            .iter()
            .map(|sample| sample.duration)
            .next()
            .is_some_and(|first| track.samples.iter().any(|sample| sample.duration != first));
    StreamInfo {
        id: track.id,
        kind: track.kind,
        codec: track.entry.format,
        timescale: track.timescale,
        duration: ticks_saturating(track.duration, track.timescale),
        sample_count: track.samples.len(),
        keyframe_count: track.samples.iter().filter(|sample| sample.sync).count(),
        vfr,
        video: track.entry.video,
        audio: track.entry.audio,
        avc: track.entry.avc.clone(),
        aac: track.entry.aac.clone(),
        edits: track.edits.clone(),
    }
}

/// Convert container units to ticks, saturating on a hostile timescale.
fn ticks_saturating(units: u64, timescale: u32) -> Tick {
    if timescale == 0 {
        return Tick::ZERO;
    }
    let units = i64::try_from(units).unwrap_or(i64::MAX);
    Tick::from_rational(units, 1, i64::from(timescale), Rounding::Nearest).unwrap_or(Tick(i64::MAX))
}

/// Convert a signed container timestamp to ticks.
fn ticks(units: i64, timescale: u32) -> Result<Tick> {
    if timescale == 0 {
        return Err(MediaError::Invalid("track timescale is zero".into()));
    }
    Ok(Tick::from_rational(
        units,
        1,
        i64::from(timescale),
        Rounding::Nearest,
    )?)
}

/// What is left of the file-wide sample limit while tracks are parsed.
struct SampleBudget {
    remaining: usize,
}

impl SampleBudget {
    fn new(file_len: u64) -> Self {
        let by_len = usize::try_from(file_len / FILE_BYTES_PER_SAMPLE).unwrap_or(usize::MAX);
        Self {
            remaining: by_len.clamp(MIN_SAMPLE_ALLOWANCE, MAX_SAMPLES),
        }
    }

    fn take(&mut self, n: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(n)
            .ok_or(MediaError::TooLarge("sample count"))?;
        Ok(())
    }
}

fn read_range<S: ByteSource + ?Sized>(source: &S, offset: u64, size: u64) -> Result<Vec<u8>> {
    let size = usize::try_from(size).map_err(|_| MediaError::TooLarge("box payload"))?;
    let mut data = vec![0u8; size];
    read_into(source, offset, &mut data)?;
    Ok(data)
}

fn read_into<S: ByteSource + ?Sized>(source: &S, offset: u64, buf: &mut [u8]) -> Result<()> {
    let size = u64::try_from(buf.len()).map_err(|_| MediaError::TooLarge("buffer"))?;
    let end = offset
        .checked_add(size)
        .ok_or(MediaError::Truncated("sample data"))?;
    if end > source.len() {
        return Err(MediaError::Truncated("sample data"));
    }
    source.read_at(offset, buf)?;
    Ok(())
}

/// Parse an MP4/MOV file from a byte source.
fn open<S: ByteSource + ?Sized>(source: &S) -> Result<Mp4File> {
    let len = source.len();
    let mut offset = 0u64;
    let mut ftyp: Option<Vec<u8>> = None;
    let mut moov: Option<Vec<u8>> = None;
    while offset
        .checked_add(8)
        .is_some_and(|header_end| header_end <= len)
    {
        let available = (len - offset).min(BoxHeader::MAX_LEN as u64) as usize;
        let mut header = [0u8; BoxHeader::MAX_LEN];
        let slice: &mut [u8] = header.get_mut(..available).unwrap_or(&mut []);
        source.read_at(offset, slice)?;
        let header = BoxHeader::parse(slice)?;
        let size = header.size.unwrap_or(len - offset);
        let end = offset.saturating_add(size).min(len);
        let body = end.saturating_sub(offset.saturating_add(header.header_len));
        let body_bytes = || -> Result<Vec<u8>> {
            read_range(source, offset.saturating_add(header.header_len), body)
        };
        match &header.kind.0 {
            b"ftyp" if ftyp.is_none() => {
                if body > MAX_FTYP {
                    return Err(MediaError::TooLarge("ftyp"));
                }
                ftyp = Some(body_bytes()?);
            }
            b"moov" if moov.is_none() => {
                if body > MAX_HEADER_BOX {
                    return Err(MediaError::TooLarge("header box"));
                }
                moov = Some(body_bytes()?);
            }
            _ => {}
        }
        if header.size.is_none() || end <= offset {
            break;
        }
        offset = end;
    }
    let moov = moov.ok_or(MediaError::NoMoov)?;
    parse_moov(&moov, ftyp.as_deref(), len)
}

fn parse_moov(moov: &[u8], ftyp: Option<&[u8]>, file_len: u64) -> Result<Mp4File> {
    let mut file = Mp4File {
        brand: super::FourCc::default(),
        is_quicktime: true,
        timescale: 0,
        duration: 0,
        tracks: Vec::new(),
    };
    if let Some(ftyp) = ftyp {
        let mut c = Reader::labeled(ftyp, "ftyp");
        file.brand = c.fourcc()?;
        let _minor_version = c.u32().unwrap_or(0);
        file.is_quicktime = file.brand == b"qt  ";
    }
    let mut tracks = Vec::new();
    for b in boxes(moov) {
        let b = b?;
        match &b.kind.0 {
            b"cmov" => return Err(MediaError::Unsupported("compressed movie header".into())),
            b"mvex" => return Err(MediaError::Unsupported("fragmented MP4".into())),
            b"mvhd" => {
                let mut c = Reader::labeled(b.payload, "mvhd");
                let (version, _) = c.full_header()?;
                if version == 1 {
                    c.skip(16)?;
                    file.timescale = c.u32()?;
                    file.duration = c.u64()?;
                } else {
                    c.skip(8)?;
                    file.timescale = c.u32()?;
                    file.duration = u64::from(c.u32()?);
                }
            }
            b"trak" => tracks.push(b.payload),
            _ => {}
        }
    }
    let mut budget = SampleBudget::new(file_len);
    for payload in tracks {
        file.tracks.push(parse_trak(
            payload,
            file.is_quicktime,
            file_len,
            &mut budget,
        )?);
    }
    Ok(file)
}

fn parse_trak(
    payload: &[u8],
    is_quicktime: bool,
    file_len: u64,
    budget: &mut SampleBudget,
) -> Result<Track> {
    let mut track = Track {
        id: 0,
        kind: StreamKind::Other,
        timescale: 0,
        duration: 0,
        entry: SampleEntry::default(),
        samples: Vec::new(),
        edits: Vec::new(),
    };
    let mut handler = super::FourCc::default();
    let mut width = 0u16;
    let mut height = 0u16;
    let mut mdia = None;
    for b in boxes(payload) {
        let b = b?;
        match &b.kind.0 {
            b"tkhd" => {
                let mut c = Reader::labeled(b.payload, "tkhd");
                let (version, _) = c.full_header()?;
                if version == 1 {
                    c.skip(16)?;
                    track.id = c.u32()?;
                    c.skip(4)?;
                    c.skip(8)?;
                } else {
                    c.skip(8)?;
                    track.id = c.u32()?;
                    c.skip(4)?;
                    c.skip(4)?;
                }
                c.skip(8)?;
                c.skip(2)?;
                c.skip(2)?;
                c.skip(2)?;
                c.skip(2)?;
                c.skip(36)?;
                let coded_width = c.u32()? >> 16;
                let coded_height = c.u32()? >> 16;
                width = u16::try_from(coded_width).unwrap_or(u16::MAX);
                height = u16::try_from(coded_height).unwrap_or(u16::MAX);
            }
            b"edts" => {
                if let Some(elst) = find(b.payload, b"elst") {
                    let mut c = Reader::labeled(elst, "elst");
                    let (version, _) = c.full_header()?;
                    let entry_size = if version == 1 { 20 } else { 12 };
                    let count = c.count(entry_size)?;
                    for _ in 0..count {
                        let (segment_duration, media_time) = if version == 1 {
                            (c.u64()?, c.i64()?)
                        } else {
                            (u64::from(c.u32()?), i64::from(c.i32()?))
                        };
                        let media_rate = c.i32()?;
                        track.edits.push(Edit {
                            segment_duration,
                            media_time,
                            media_rate,
                        });
                    }
                }
            }
            b"mdia" => mdia = Some(b.payload),
            _ => {}
        }
    }
    let mdia = mdia.ok_or_else(|| MediaError::Invalid("trak without mdia".into()))?;
    let mut minf = None;
    for b in boxes(mdia) {
        let b = b?;
        match &b.kind.0 {
            b"mdhd" => {
                let mut c = Reader::labeled(b.payload, "mdhd");
                let (version, _) = c.full_header()?;
                if version == 1 {
                    c.skip(16)?;
                    track.timescale = c.u32()?;
                    track.duration = c.u64()?;
                } else {
                    c.skip(8)?;
                    track.timescale = c.u32()?;
                    track.duration = u64::from(c.u32()?);
                }
            }
            b"hdlr" => {
                let mut c = Reader::labeled(b.payload, "hdlr");
                c.full_header()?;
                c.skip(4)?;
                handler = c.fourcc()?;
            }
            b"minf" => minf = Some(b.payload),
            _ => {}
        }
    }
    track.kind = match &handler.0 {
        b"vide" => StreamKind::Video,
        b"soun" => StreamKind::Audio,
        _ => StreamKind::Other,
    };
    if track.kind == StreamKind::Video {
        track.entry.video = Some(VideoInfo { width, height });
    }
    if let Some(stbl) = minf.and_then(|minf| find(minf, b"stbl")) {
        parse_stbl(stbl, &mut track, handler, is_quicktime, file_len, budget)?;
    }
    if !track.samples.is_empty() && track.timescale == 0 {
        return Err(MediaError::Invalid(
            "track with samples has a zero timescale".into(),
        ));
    }
    Ok(track)
}

struct SttsCursor<'a> {
    runs: &'a [(u32, u32)],
    run: usize,
    used: u32,
}

impl SttsCursor<'_> {
    fn take(&mut self, mut n: u64) -> u64 {
        let mut total = 0u64;
        while n > 0 {
            let Some(&(count, delta)) = self.runs.get(self.run) else {
                break;
            };
            if count == 0 {
                self.run += 1;
                self.used = 0;
                continue;
            }
            let available = u64::from(count.saturating_sub(self.used));
            let take = available.min(n);
            total = total.saturating_add(take.saturating_mul(u64::from(delta)));
            n -= take;
            self.used = self.used.saturating_add(take as u32);
            if self.used >= count {
                self.run += 1;
                self.used = 0;
            }
        }
        total
    }

    fn next_value(&mut self) -> u32 {
        while self.runs.get(self.run).is_some_and(|run| run.0 == 0) {
            self.run += 1;
        }
        let Some(&(_, delta)) = self.runs.get(self.run) else {
            return 0;
        };
        self.take(1);
        delta
    }
}

fn parse_stbl(
    stbl: &[u8],
    track: &mut Track,
    handler: super::FourCc,
    is_quicktime: bool,
    file_len: u64,
    budget: &mut SampleBudget,
) -> Result<()> {
    let mut stts: Vec<(u32, u32)> = Vec::new();
    let mut ctts: Vec<(u32, u32)> = Vec::new();
    let mut stss: Option<Vec<u32>> = None;
    let mut stsc: Vec<(u32, u32, u32)> = Vec::new();
    let mut const_size = 0u32;
    let mut sample_count = 0usize;
    let mut sizes: Vec<u32> = Vec::new();
    let mut chunks: Vec<u64> = Vec::new();
    for b in boxes(stbl) {
        let b = b?;
        let mut c = Reader::labeled(b.payload, "stbl child");
        match &b.kind.0 {
            b"stsd" => {
                c.full_header()?;
                let count = c.count(8)?;
                for entry in boxes(c.rest()).take(count) {
                    let entry = entry?;
                    if track.entry.format == super::FourCc::default() {
                        track.entry =
                            parse_sample_entry(entry.kind, entry.payload, handler, is_quicktime)?;
                    }
                }
            }
            b"stts" => {
                c.full_header()?;
                let count = c.count(8)?;
                stts = (0..count)
                    .map(|_| Ok((c.u32()?, c.u32()?)))
                    .collect::<Result<_>>()?;
            }
            b"ctts" => {
                c.full_header()?;
                let count = c.count(8)?;
                ctts = (0..count)
                    .map(|_| Ok((c.u32()?, c.u32()?)))
                    .collect::<Result<_>>()?;
            }
            b"stss" => {
                c.full_header()?;
                let count = c.count(4)?;
                let mut values: Vec<u32> = (0..count).map(|_| c.u32()).collect::<Result<_>>()?;
                values.sort_unstable();
                stss = Some(values);
            }
            b"stsc" => {
                c.full_header()?;
                let count = c.count(12)?;
                stsc = (0..count)
                    .map(|_| Ok((c.u32()?, c.u32()?, c.u32()?)))
                    .collect::<Result<_>>()?;
            }
            b"stsz" => {
                c.full_header()?;
                const_size = c.u32()?;
                let count = c.u32()? as usize;
                if const_size == 0 {
                    c.check_count(count, 4)?;
                    sizes = (0..count).map(|_| c.u32()).collect::<Result<_>>()?;
                    sample_count = count;
                } else {
                    sample_count = (count as u64).min(file_len / u64::from(const_size)) as usize;
                }
            }
            b"stz2" => {
                c.full_header()?;
                let field = c.u32()? & 0xFF;
                let count = c.u32()? as usize;
                let bits = match field {
                    4 | 8 | 16 => field as usize,
                    _ => return Err(MediaError::Invalid(format!("stz2 field size {field}"))),
                };
                if count
                    .checked_mul(bits)
                    .is_none_or(|total| total.div_ceil(8) > c.remaining())
                {
                    return Err(MediaError::Truncated("stz2"));
                }
                let data = c.rest();
                sizes = (0..count)
                    .map(|i| match bits {
                        4 => {
                            let byte = data.get(i / 2).copied().unwrap_or(0);
                            u32::from(if i % 2 == 0 { byte >> 4 } else { byte & 0xF })
                        }
                        8 => u32::from(data.get(i).copied().unwrap_or(0)),
                        _ => {
                            let high = data.get(2 * i).copied().unwrap_or(0);
                            let low = data.get(2 * i + 1).copied().unwrap_or(0);
                            u32::from(u16::from_be_bytes([high, low]))
                        }
                    })
                    .collect();
                sample_count = count;
            }
            b"stco" => {
                c.full_header()?;
                let count = c.count(4)?;
                chunks = (0..count)
                    .map(|_| c.u32().map(u64::from))
                    .collect::<Result<_>>()?;
            }
            b"co64" => {
                c.full_header()?;
                let count = c.count(8)?;
                chunks = (0..count).map(|_| c.u64()).collect::<Result<_>>()?;
            }
            _ => {}
        }
    }
    if sample_count == 0 || chunks.is_empty() || stsc.is_empty() {
        return Ok(());
    }
    if stsc.first().is_none_or(|first| first.0 == 0)
        || stsc
            .windows(2)
            .any(|pair| match (pair.first(), pair.get(1)) {
                (Some(first), Some(second)) => second.0 <= first.0,
                _ => false,
            })
    {
        return Err(MediaError::Invalid(
            "stsc first_chunk values are not increasing from 1".into(),
        ));
    }
    if const_size != 0 {
        sample_count = sample_count.min((file_len / u64::from(const_size)) as usize);
    }
    if sample_count > budget.remaining {
        return Err(MediaError::TooLarge("sample count"));
    }
    let mut mapped = 0usize;
    let mut map_index = 0usize;
    for chunk_index in 0..chunks.len() {
        while map_index + 1 < stsc.len()
            && stsc
                .get(map_index + 1)
                .is_some_and(|entry| entry.0 as usize <= chunk_index + 1)
        {
            map_index += 1;
        }
        mapped = mapped.saturating_add(stsc.get(map_index).map_or(0, |entry| entry.1 as usize));
        if mapped >= sample_count {
            break;
        }
    }
    let mut samples = Vec::new();
    samples
        .try_reserve(sample_count.min(mapped))
        .map_err(|_| MediaError::TooLarge("sample count"))?;
    let mut stts_cursor = SttsCursor {
        runs: &stts,
        run: 0,
        used: 0,
    };
    let mut ctts_cursor = SttsCursor {
        runs: &ctts,
        run: 0,
        used: 0,
    };
    let mut stss_index = 0usize;
    let mut sample_index = 0usize;
    let mut dts = 0i64;
    let mut stsc_index = 0usize;
    'chunks: for (chunk_index, &chunk_offset) in chunks.iter().enumerate() {
        while stsc_index + 1 < stsc.len()
            && stsc
                .get(stsc_index + 1)
                .is_some_and(|entry| entry.0 as usize <= chunk_index + 1)
        {
            stsc_index += 1;
        }
        let Some(&(_, per_chunk, _)) = stsc.get(stsc_index) else {
            break;
        };
        let mut offset = chunk_offset;
        for _ in 0..per_chunk {
            if sample_index >= sample_count {
                break 'chunks;
            }
            let size = if const_size != 0 {
                const_size
            } else {
                sizes.get(sample_index).copied().unwrap_or(0)
            };
            let duration = stts_cursor.next_value();
            let composition_offset = ctts_cursor.next_value() as i32 as i64;
            let sync = match &stss {
                None => true,
                Some(values) => {
                    while values
                        .get(stss_index)
                        .is_some_and(|&value| (value as usize) < sample_index + 1)
                    {
                        stss_index += 1;
                    }
                    values
                        .get(stss_index)
                        .is_some_and(|&value| value as usize == sample_index + 1)
                }
            };
            samples.push(Sample {
                offset,
                size,
                dts,
                pts: dts.saturating_add(composition_offset),
                duration,
                sync,
            });
            offset = offset.saturating_add(u64::from(size));
            dts = dts.saturating_add(i64::from(duration));
            sample_index += 1;
        }
    }
    budget.take(samples.len())?;
    track.samples = samples;
    Ok(())
}

/// Probe a file without reading its media data.
pub fn probe(path: &Path) -> Result<Probe> {
    let source = File::open(path)?;
    Ok(open(&source)?.probe())
}

/// One demuxed packet in decode order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// Index into [`Probe::streams`].
    pub stream: usize,
    /// Presentation timestamp in source-media ticks (before edit-list mapping).
    pub pts: Tick,
    /// Decode timestamp in source-media ticks.
    pub dts: Tick,
    /// Sample duration in source-media ticks.
    pub duration: Tick,
    /// True for a sync sample (keyframe).
    pub keyframe: bool,
    /// Sample bytes in the container's stored form (AVCC length prefixes for H.264).
    pub data: Vec<u8>,
}

/// Streams one sample at a time from a file, interleaved by decode timestamp.
pub struct PacketReader {
    source: File,
    /// File length, read once at open.
    len: u64,
    file: Mp4File,
    probe: Probe,
    cursors: Vec<usize>,
}

impl PacketReader {
    /// Open a file and read its sample tables; no media data is read here.
    pub fn open(path: &Path) -> Result<Self> {
        let source = File::open(path)?;
        let len = source.len();
        let file = open(&source)?;
        let probe = file.probe();
        let cursors = vec![0; file.tracks.len()];
        Ok(Self {
            source,
            len,
            file,
            probe,
            cursors,
        })
    }

    /// The probe read at open time.
    pub fn probe(&self) -> &Probe {
        &self.probe
    }

    /// Next packet by decode timestamp across all streams, or `None` at the end.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::TooLarge`] for a sample above [`MAX_PACKET_BYTES`] and
    /// [`MediaError::Truncated`] for sample data past the end of the file.
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        let mut best: Option<(Tick, usize)> = None;
        for (index, track) in self.file.tracks.iter().enumerate() {
            let cursor = self.cursors.get(index).copied().unwrap_or(0);
            if let Some(sample) = track.samples.get(cursor) {
                let dts = ticks(sample.dts, track.timescale)?;
                if best.is_none_or(|(best_dts, best_index)| (dts, index) < (best_dts, best_index)) {
                    best = Some((dts, index));
                }
            }
        }
        let Some((dts, index)) = best else {
            return Ok(None);
        };
        let track = self
            .file
            .tracks
            .get(index)
            .ok_or(MediaError::NoSuchTrack(index))?;
        let cursor = self.cursors.get(index).copied().unwrap_or(0);
        let sample = *track
            .samples
            .get(cursor)
            .ok_or(MediaError::NoSuchSample(cursor))?;
        let pts = ticks(sample.pts, track.timescale)?;
        let duration = ticks(i64::from(sample.duration), track.timescale)?;
        let size = usize::try_from(sample.size).map_err(|_| MediaError::TooLarge("packet"))?;
        if size > MAX_PACKET_BYTES {
            return Err(MediaError::TooLarge("packet"));
        }
        let mut data = vec![0u8; size];
        let end = sample
            .offset
            .checked_add(u64::from(sample.size))
            .ok_or(MediaError::Truncated("sample data"))?;
        if end > self.len {
            return Err(MediaError::Truncated("sample data"));
        }
        self.source.read_at(sample.offset, &mut data)?;
        if let Some(cursor) = self.cursors.get_mut(index) {
            *cursor = cursor.saturating_add(1);
        }
        Ok(Some(Packet {
            stream: index,
            pts,
            dts,
            duration,
            keyframe: sample.sync,
            data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::test_support::sample_file;
    use crate::{AudioInfo, AvcConfig, Brand, MuxOptions, Muxer, VideoTrackConfig, WritePacket};
    use std::cell::Cell;
    use std::io::Write;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/media")
    }

    fn scratch_path(name: &str) -> PathBuf {
        let dir = fixtures();
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn probes_and_streams_a_muxed_file() {
        let data = sample_file();
        let file = open(data.as_slice()).unwrap();
        let probe = file.probe();
        assert_eq!(probe.streams.len(), 2);
        let video = &probe.streams[0];
        assert_eq!(video.kind, StreamKind::Video);
        assert_eq!(video.codec, b"avc1");
        assert_eq!(video.timescale, 30_000);
        assert_eq!(video.sample_count, 30);
        assert_eq!(video.keyframe_count, 3);
        assert!(!video.vfr);
        assert_eq!(
            video.video,
            Some(VideoInfo {
                width: 640,
                height: 360
            })
        );
        let audio = &probe.streams[1];
        assert_eq!(audio.kind, StreamKind::Audio);
        assert_eq!(audio.codec, b"mp4a");
        assert_eq!(audio.edits.len(), 1);
        assert_eq!(audio.edits.first().map(|edit| edit.media_time), Some(1024));
        assert_eq!(
            audio.audio,
            Some(AudioInfo {
                sample_rate: 48_000,
                channels: 2
            })
        );

        let path = scratch_path("unit-roundtrip.mp4");
        let mut out = File::create(&path).unwrap();
        out.write_all(&data).unwrap();
        out.flush().unwrap();
        let mut reader = PacketReader::open(&path).unwrap();
        assert_eq!(reader.probe(), &probe);
        let mut video_packets = 0;
        let mut audio_packets = 0;
        let mut last_video_dts = Tick(-1);
        while let Some(packet) = reader.next_packet().unwrap() {
            match packet.stream {
                0 => {
                    assert!(packet.dts >= last_video_dts);
                    last_video_dts = packet.dts;
                    assert_eq!(packet.data.len(), 12);
                    video_packets += 1;
                }
                _ => {
                    assert_eq!(packet.data.len(), 20);
                    audio_packets += 1;
                }
            }
        }
        assert_eq!((video_packets, audio_packets), (30, 30));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn vfr_flag_follows_sample_durations() {
        let avc = AvcConfig::new(vec![vec![0x67, 0x42, 0, 0x1E]], vec![vec![0x68, 0xCE]], 4);
        let mut muxer = Muxer::new(
            std::io::Cursor::new(Vec::new()),
            MuxOptions {
                brand: Brand::Mp4,
                movie_timescale: 1000,
            },
        )
        .unwrap();
        let video = muxer
            .add_video(VideoTrackConfig {
                width: 64,
                height: 48,
                timescale: 15_360,
                avc,
            })
            .unwrap();
        for (dts, duration) in [(0i64, 512u32), (512, 512), (1024, 1024)] {
            muxer
                .write_packet(
                    video,
                    WritePacket {
                        dts,
                        pts: dts,
                        duration,
                        keyframe: true,
                        data: &[0u8; 4],
                    },
                )
                .unwrap();
        }
        let out = muxer.finish().unwrap().into_inner();
        let probe = open(out.as_slice()).unwrap().probe();
        assert!(probe.streams.first().is_some_and(|stream| stream.vfr));
    }

    /// A source that counts how many bytes were requested.
    struct Counting<'a> {
        file: &'a File,
        bytes: Cell<u64>,
    }

    impl ByteSource for Counting<'_> {
        fn len(&self) -> u64 {
            self.file.len()
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            self.bytes.set(self.bytes.get() + buf.len() as u64);
            self.file.read_at(offset, buf)
        }
    }

    #[test]
    fn reads_a_sparse_file_larger_than_two_gibibytes_without_loading_it() {
        let data = sample_file();
        let path = scratch_path("sparse-2g.mp4");
        {
            let mut file = File::create(&path).unwrap();
            file.write_all(&data).unwrap();
            file.set_len(2_500_000_000).unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() > 2_000_000_000);
        let file = File::open(&path).unwrap();
        let counting = Counting {
            file: &file,
            bytes: Cell::new(0),
        };
        let open_read = open(&counting).unwrap();
        let probe = open_read.probe();
        assert_eq!(probe.streams.len(), 2);
        let read = counting.bytes.get();
        assert!(read < 1 << 20, "open read {read} bytes");
        assert_eq!(open_read.tracks.first().map(|t| t.samples.len()), Some(30));
        let counting = Counting {
            file: &file,
            bytes: Cell::new(0),
        };
        let held = open(&counting).unwrap();
        let sample = held
            .tracks
            .first()
            .and_then(|t| t.samples.first())
            .copied()
            .unwrap();
        let mut bytes = vec![0u8; sample.size as usize];
        read_into(&counting, sample.offset, &mut bytes).unwrap();
        assert_eq!(bytes.len(), 12);
        std::fs::remove_file(path).unwrap();
    }
}
