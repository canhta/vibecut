//! Progressive MP4/MOV muxing: H.264 (`avc1` + `avcC`) and AAC (`mp4a` + `esds`).
//!
//! Layout: `ftyp`, one `mdat` with the samples in write order, then a `moov` at the
//! end. Sample data is streamed to the writer, so memory use does not grow with
//! file length. An AAC edit list hides encoder priming.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use super::boxes::BoxBuf;
use super::codec::{write_audio_entry, write_video_entry};
use crate::{AacConfig, AvcConfig, MediaError, Result};

/// Output flavor: brands and the layout of some top-level boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brand {
    /// ISO MP4: major brand `isom`.
    Mp4,
    /// QuickTime MOV: major brand `qt  `.
    Mov,
}

/// Movie-level muxer options.
#[derive(Clone, Debug)]
pub struct MuxOptions {
    pub brand: Brand,
    /// Movie timescale (`mvhd`); 1000 keeps millisecond rounding small.
    pub movie_timescale: u32,
}

impl Default for MuxOptions {
    fn default() -> Self {
        Self {
            brand: Brand::Mp4,
            movie_timescale: 1000,
        }
    }
}

/// Configuration for an H.264 video track.
#[derive(Clone, Debug)]
pub struct VideoTrackConfig {
    pub width: u16,
    pub height: u16,
    /// Media timescale for this track's timestamps and durations.
    pub timescale: u32,
    pub avc: AvcConfig,
}

/// Configuration for an AAC audio track.
#[derive(Clone, Debug)]
pub struct AudioTrackConfig {
    pub aac: AacConfig,
    pub channels: u16,
    pub sample_rate: u32,
    /// Media timescale for this track's timestamps and durations.
    pub timescale: u32,
    /// Encoder delay/priming in media units, hidden by the edit list; 0 for none.
    pub priming: u32,
}

/// One sample handed to [`Muxer::write_packet`], in the track's media timescale.
#[derive(Clone, Copy, Debug)]
pub struct WritePacket<'a> {
    pub dts: i64,
    pub pts: i64,
    /// Duration in media units.
    pub duration: u32,
    pub keyframe: bool,
    /// Sample bytes as stored: AVCC length-prefixed NAL units for H.264, one raw
    /// frame for AAC.
    pub data: &'a [u8],
}

#[derive(Clone, Debug)]
enum TrackKind {
    Video {
        width: u16,
        height: u16,
        avc: AvcConfig,
    },
    Audio {
        sample_rate: u32,
        channels: u16,
        aac: AacConfig,
        priming: u32,
    },
}

#[derive(Clone, Debug)]
struct WriteTrack {
    id: u32,
    kind: TrackKind,
    timescale: u32,
    tables: Tables,
}

/// Accumulated sample tables for one track.
#[derive(Clone, Debug, Default)]
struct Tables {
    stts: Vec<(u32, u32)>,
    ctts: Vec<(u32, i32)>,
    sizes: Vec<u32>,
    /// 1-based sample numbers of sync samples.
    sync: Vec<u32>,
    all_sync: bool,
    /// `(offset, sample count)` per chunk, in write order.
    chunks: Vec<(u64, u32)>,
    sample_count: u64,
    media_duration: u64,
}

impl Tables {
    fn new() -> Self {
        Self {
            all_sync: true,
            ..Self::default()
        }
    }

    fn push_run<T: PartialEq + Copy>(runs: &mut Vec<(u32, T)>, mut count: u64, value: T) {
        if let Some(last) = runs.last_mut()
            && last.1 == value
        {
            let take = count.min(u64::from(u32::MAX - last.0));
            last.0 += take as u32;
            count -= take;
        }
        while count > 0 {
            let take = count.min(u64::from(u32::MAX));
            runs.push((take as u32, value));
            count -= take;
        }
    }

    fn push(
        &mut self,
        offset: u64,
        size: u32,
        duration: u32,
        composition_offset: i32,
        keyframe: bool,
        contiguous: bool,
    ) {
        Self::push_run(&mut self.stts, 1, duration);
        Self::push_run(&mut self.ctts, 1, composition_offset);
        self.sizes.push(size);
        self.sample_count += 1;
        if keyframe {
            self.sync
                .push(u32::try_from(self.sample_count).unwrap_or(u32::MAX));
        } else {
            self.all_sync = false;
        }
        self.media_duration = self.media_duration.saturating_add(u64::from(duration));
        if contiguous && let Some(last) = self.chunks.last_mut() {
            last.1 = last.1.saturating_add(1);
        } else {
            self.chunks.push((offset, 1));
        }
    }
}

fn rescale(value: u64, from: u32, to: u32) -> u64 {
    if from == 0 {
        return 0;
    }
    (u128::from(value) * u128::from(to) / u128::from(from)).min(u128::from(u64::MAX)) as u64
}

const MATRIX: [i32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

fn write_ftyp(b: &mut BoxBuf, brand: Brand) {
    let marker = b.start(b"ftyp");
    match brand {
        Brand::Mp4 => {
            b.bytes(b"isom");
            b.u32(0x200);
            for compatible in [b"isom", b"iso2", b"avc1", b"mp41", b"mp42"] {
                b.bytes(compatible);
            }
        }
        Brand::Mov => {
            b.bytes(b"qt  ");
            b.u32(0x200);
            b.bytes(b"qt  ");
        }
    }
    b.end(marker);
}

fn write_mvhd(b: &mut BoxBuf, timescale: u32, duration: u64, next_track_id: u32) {
    let v1 = duration > u64::from(u32::MAX);
    let marker = b.start_full(b"mvhd", u8::from(v1), 0);
    b.u32(0);
    b.u32(0);
    b.u32(timescale);
    if v1 {
        b.u64(duration);
    } else {
        b.u32(duration as u32);
    }
    b.u32(0x0001_0000);
    b.u16(0x0100);
    b.zeros(10);
    for value in MATRIX {
        b.i32(value);
    }
    b.zeros(24);
    b.u32(next_track_id);
    b.end(marker);
}

fn media_duration(track: &WriteTrack) -> u64 {
    track.tables.media_duration
}

fn priming(track: &WriteTrack) -> u32 {
    match track.kind {
        TrackKind::Audio { priming, .. } => priming,
        TrackKind::Video { .. } => 0,
    }
}

fn presentation_duration(track: &WriteTrack, movie_timescale: u32) -> u64 {
    let media = media_duration(track);
    let start = u64::from(priming(track)).min(media);
    rescale(media - start, track.timescale, movie_timescale)
}

fn write_trak(b: &mut BoxBuf, track: &WriteTrack, movie_timescale: u32, qt: bool) -> Result<()> {
    let duration = presentation_duration(track, movie_timescale);
    let trak = b.start(b"trak");
    let v1 = duration > u64::from(u32::MAX);
    let marker = b.start_full(b"tkhd", u8::from(v1), 3);
    b.u32(0);
    b.u32(0);
    b.u32(track.id);
    b.u32(0);
    if v1 {
        b.u64(duration);
    } else {
        b.u32(duration as u32);
    }
    b.zeros(8);
    b.u16(0);
    b.u16(0);
    b.u16(if matches!(track.kind, TrackKind::Audio { .. }) {
        0x0100
    } else {
        0
    });
    b.u16(0);
    for value in MATRIX {
        b.i32(value);
    }
    let (width, height) = match &track.kind {
        TrackKind::Video { width, height, .. } => (*width, *height),
        TrackKind::Audio { .. } => (0, 0),
    };
    b.u32(u32::from(width) << 16);
    b.u32(u32::from(height) << 16);
    b.end(marker);

    let start = u64::from(priming(track));
    if start > 0 {
        let edts = b.start(b"edts");
        let v1 = duration > u64::from(u32::MAX) || start > i32::MAX as u64;
        let elst = b.start_full(b"elst", u8::from(v1), 0);
        b.u32(1);
        if v1 {
            b.u64(duration);
            b.u64(start);
        } else {
            b.u32(duration as u32);
            b.i32(start as i32);
        }
        b.i32(0x0001_0000);
        b.end(elst);
        b.end(edts);
    }

    let mdia = b.start(b"mdia");
    let media = media_duration(track);
    let mv1 = media > u64::from(u32::MAX);
    let marker = b.start_full(b"mdhd", u8::from(mv1), 0);
    b.u32(0);
    b.u32(0);
    b.u32(track.timescale);
    if mv1 {
        b.u64(media);
    } else {
        b.u32(media as u32);
    }
    b.u16(0x55C4);
    b.u16(0);
    b.end(marker);

    let (handler, name) = match &track.kind {
        TrackKind::Video { .. } => (b"vide", "VideoHandler"),
        TrackKind::Audio { .. } => (b"soun", "SoundHandler"),
    };
    let marker = b.start_full(b"hdlr", 0, 0);
    b.bytes(if qt { b"mhlr" } else { &[0, 0, 0, 0] });
    b.bytes(handler);
    b.zeros(12);
    if qt {
        let bytes = name.as_bytes();
        b.u8(bytes.len() as u8);
        b.bytes(bytes);
    } else {
        b.bytes(name.as_bytes());
        b.u8(0);
    }
    b.end(marker);

    let minf = b.start(b"minf");
    match &track.kind {
        TrackKind::Video { .. } => {
            let marker = b.start_full(b"vmhd", 0, 1);
            b.zeros(8);
            b.end(marker);
        }
        TrackKind::Audio { .. } => {
            let marker = b.start_full(b"smhd", 0, 0);
            b.zeros(4);
            b.end(marker);
        }
    }
    let dinf = b.start(b"dinf");
    let dref = b.start_full(b"dref", 0, 0);
    b.u32(1);
    let url = b.start_full(b"url ", 0, 1);
    b.end(url);
    b.end(dref);
    b.end(dinf);
    write_stbl(b, track)?;
    b.end(minf);
    b.end(mdia);
    b.end(trak);
    Ok(())
}

fn write_stbl(b: &mut BoxBuf, track: &WriteTrack) -> Result<()> {
    let tables = &track.tables;
    if tables.sample_count > u64::from(u32::MAX) {
        return Err(MediaError::Mux(
            "more than 2^32 - 1 samples in one track".into(),
        ));
    }
    let sample_count = tables.sample_count as u32;
    let stbl = b.start(b"stbl");
    let stsd = b.start_full(b"stsd", 0, 0);
    b.u32(1);
    match &track.kind {
        TrackKind::Video { width, height, avc } => write_video_entry(b, *width, *height, avc),
        TrackKind::Audio {
            sample_rate,
            channels,
            aac,
            ..
        } => {
            write_audio_entry(b, *sample_rate, *channels, aac);
        }
    }
    b.end(stsd);

    let stts = b.start_full(b"stts", 0, 0);
    b.u32(tables.stts.len() as u32);
    for &(count, duration) in &tables.stts {
        b.u32(count);
        b.u32(duration);
    }
    b.end(stts);

    if tables.ctts.iter().any(|&(_, offset)| offset != 0) {
        let version = u8::from(tables.ctts.iter().any(|&(_, offset)| offset < 0));
        let ctts = b.start_full(b"ctts", version, 0);
        b.u32(tables.ctts.len() as u32);
        for &(count, offset) in &tables.ctts {
            b.u32(count);
            b.i32(offset);
        }
        b.end(ctts);
    }

    if !tables.all_sync {
        let stss = b.start_full(b"stss", 0, 0);
        b.u32(tables.sync.len() as u32);
        for &sample in &tables.sync {
            b.u32(sample);
        }
        b.end(stss);
    }

    let mut runs: Vec<(u32, u32)> = Vec::new();
    for (index, &(_, count)) in tables.chunks.iter().enumerate() {
        if runs.last().is_none_or(|run| run.1 != count) {
            runs.push((index as u32 + 1, count));
        }
    }
    let stsc = b.start_full(b"stsc", 0, 0);
    b.u32(runs.len() as u32);
    for (first_chunk, per_chunk) in runs {
        b.u32(first_chunk);
        b.u32(per_chunk);
        b.u32(1);
    }
    b.end(stsc);

    let stsz = b.start_full(b"stsz", 0, 0);
    let first_size = tables.sizes.first().copied().unwrap_or(0);
    if first_size != 0 && tables.sizes.iter().all(|&size| size == first_size) {
        b.u32(first_size);
        b.u32(sample_count);
    } else {
        b.u32(0);
        b.u32(sample_count);
        for &size in &tables.sizes {
            b.u32(size);
        }
    }
    b.end(stsz);

    let large = tables
        .chunks
        .iter()
        .any(|&(offset, _)| offset > u64::from(u32::MAX));
    let stco = b.start_full(if large { b"co64" } else { b"stco" }, 0, 0);
    b.u32(tables.chunks.len() as u32);
    for &(offset, _) in &tables.chunks {
        if large {
            b.u64(offset);
        } else {
            b.u32(offset as u32);
        }
    }
    b.end(stco);
    b.end(stbl);
    Ok(())
}

/// Progressive MP4/MOV writer over a seekable sink.
pub struct Muxer<W: Write + Seek> {
    writer: W,
    options: MuxOptions,
    tracks: Vec<WriteTrack>,
    /// Offset of the `mdat` box header.
    mdat_offset: u64,
    /// Offset where the next sample byte goes.
    position: u64,
    last_track: Option<usize>,
}

impl Muxer<File> {
    /// Create or truncate a file and start writing.
    pub fn create(path: &Path, options: MuxOptions) -> Result<Self> {
        Self::new(File::create(path)?, options)
    }
}

impl<W: Write + Seek> Muxer<W> {
    /// Start a file: writes `ftyp` and a large-size `mdat` header.
    pub fn new(mut writer: W, options: MuxOptions) -> Result<Self> {
        let start = writer.stream_position()?;
        let mut b = BoxBuf::new();
        write_ftyp(&mut b, options.brand);
        let mdat_offset = start.saturating_add(b.len() as u64);
        b.u32(1);
        b.bytes(b"mdat");
        b.u64(0);
        writer.write_all(&b.buf)?;
        let position = start.saturating_add(b.len() as u64);
        Ok(Self {
            writer,
            options,
            tracks: Vec::new(),
            mdat_offset,
            position,
            last_track: None,
        })
    }

    /// Add an H.264 track; returns its index (track IDs are index + 1).
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Mux`] for a zero timescale, zero size or missing SPS.
    pub fn add_video(&mut self, config: VideoTrackConfig) -> Result<usize> {
        if config.timescale == 0 {
            return Err(MediaError::Mux("timescale must be non-zero".into()));
        }
        if config.width == 0 || config.height == 0 {
            return Err(MediaError::Mux("video size must be non-zero".into()));
        }
        if config.avc.sps.is_empty() {
            return Err(MediaError::Mux("H.264 track needs at least one SPS".into()));
        }
        self.tracks.push(WriteTrack {
            id: self.tracks.len() as u32 + 1,
            kind: TrackKind::Video {
                width: config.width,
                height: config.height,
                avc: config.avc,
            },
            timescale: config.timescale,
            tables: Tables::new(),
        });
        Ok(self.tracks.len() - 1)
    }

    /// Add an AAC track; returns its index (track IDs are index + 1).
    ///
    /// `priming` is the encoder delay in media units, written as an edit-list
    /// `media_time` so players skip it.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Mux`] for a zero timescale, rate or channel count, or
    /// an empty AudioSpecificConfig.
    pub fn add_audio(&mut self, config: AudioTrackConfig) -> Result<usize> {
        if config.timescale == 0 {
            return Err(MediaError::Mux("timescale must be non-zero".into()));
        }
        if config.sample_rate == 0 || config.channels == 0 {
            return Err(MediaError::Mux(
                "audio rate and channels must be non-zero".into(),
            ));
        }
        if config.aac.asc.is_empty() {
            return Err(MediaError::Mux(
                "AAC track needs an AudioSpecificConfig".into(),
            ));
        }
        self.tracks.push(WriteTrack {
            id: self.tracks.len() as u32 + 1,
            kind: TrackKind::Audio {
                sample_rate: config.sample_rate,
                channels: config.channels,
                aac: config.aac,
                priming: config.priming,
            },
            timescale: config.timescale,
            tables: Tables::new(),
        });
        Ok(self.tracks.len() - 1)
    }

    /// Append one sample to `track`. Samples are stored in call order; interleave
    /// tracks by decode timestamp to keep chunks small.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::NoSuchTrack`] for an unknown index and
    /// [`MediaError::Mux`] for a sample or composition offset out of range.
    pub fn write_packet(&mut self, track: usize, packet: WritePacket<'_>) -> Result<()> {
        let existing = self
            .tracks
            .get(track)
            .ok_or(MediaError::NoSuchTrack(track))?;
        if existing.tables.sample_count >= u64::from(u32::MAX) {
            return Err(MediaError::Mux(
                "more than 2^32 - 1 samples in one track".into(),
            ));
        }
        if u64::try_from(packet.data.len()).map_or(true, |len| len > u64::from(u32::MAX)) {
            return Err(MediaError::Mux("sample larger than 4 GiB".into()));
        }
        let composition_offset = i32::try_from(packet.pts.saturating_sub(packet.dts))
            .map_err(|_| MediaError::Mux("presentation offset does not fit 32 bits".into()))?;
        let contiguous = self.last_track == Some(track);
        let offset = self.position;
        self.writer.write_all(packet.data)?;
        self.position = offset
            .checked_add(packet.data.len() as u64)
            .ok_or(MediaError::Mux("file offset overflowed".into()))?;
        self.last_track = Some(track);
        let track = self
            .tracks
            .get_mut(track)
            .ok_or(MediaError::NoSuchTrack(track))?;
        track.tables.push(
            offset,
            packet.data.len() as u32,
            packet.duration,
            composition_offset,
            packet.keyframe,
            contiguous,
        );
        Ok(())
    }

    fn finish_mdat(&mut self) -> Result<()> {
        let size = self.position.saturating_sub(self.mdat_offset);
        self.writer.seek(SeekFrom::Start(self.mdat_offset + 8))?;
        self.writer.write_all(&size.to_be_bytes())?;
        self.writer.seek(SeekFrom::Start(self.position))?;
        Ok(())
    }

    fn build_moov(&self) -> Result<Vec<u8>> {
        let timescale = self.options.movie_timescale.max(1);
        let duration = self
            .tracks
            .iter()
            .map(|track| presentation_duration(track, timescale))
            .max()
            .unwrap_or(0);
        let mut b = BoxBuf::new();
        let moov = b.start(b"moov");
        write_mvhd(&mut b, timescale, duration, self.tracks.len() as u32 + 1);
        for track in &self.tracks {
            write_trak(&mut b, track, timescale, self.options.brand == Brand::Mov)?;
        }
        b.end(moov);
        Ok(b.buf)
    }

    /// Finish the file: patches `mdat`, writes `moov` and returns the sink.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Mux`] when priming consumes the whole audio track.
    pub fn finish(mut self) -> Result<W> {
        for track in &self.tracks {
            let priming = u64::from(priming(track));
            if priming > 0 && priming >= media_duration(track).max(1) {
                return Err(MediaError::Mux(
                    "audio priming consumes the whole track".into(),
                ));
            }
        }
        self.finish_mdat()?;
        let moov = self.build_moov()?;
        self.writer.seek(SeekFrom::Start(self.position))?;
        self.writer.write_all(&moov)?;
        self.writer.flush()?;
        Ok(self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::demux::probe;
    use crate::mp4::test_support::sample_file;

    #[test]
    fn options_default_to_mp4() {
        let options = MuxOptions::default();
        assert_eq!(options.brand, Brand::Mp4);
        assert_eq!(options.movie_timescale, 1000);
    }

    #[test]
    fn rejects_invalid_track_configs() {
        let mut muxer =
            Muxer::new(std::io::Cursor::new(Vec::new()), MuxOptions::default()).unwrap();
        let avc = AvcConfig::new(vec![vec![0x67]], vec![vec![0x68]], 4);
        assert!(matches!(
            muxer.add_video(VideoTrackConfig {
                width: 0,
                height: 1,
                timescale: 1,
                avc: avc.clone()
            }),
            Err(MediaError::Mux(_))
        ));
        assert!(matches!(
            muxer.add_video(VideoTrackConfig {
                width: 1,
                height: 1,
                timescale: 0,
                avc: avc.clone()
            }),
            Err(MediaError::Mux(_))
        ));
        assert!(matches!(
            muxer.add_audio(AudioTrackConfig {
                aac: AacConfig::default(),
                channels: 2,
                sample_rate: 48_000,
                timescale: 48_000,
                priming: 0,
            }),
            Err(MediaError::Mux(_))
        ));
        assert!(
            muxer
                .add_video(VideoTrackConfig {
                    width: 2,
                    height: 2,
                    timescale: 1,
                    avc
                })
                .is_ok()
        );
        assert!(matches!(
            muxer.write_packet(
                3,
                WritePacket {
                    dts: 0,
                    pts: 0,
                    duration: 1,
                    keyframe: true,
                    data: &[]
                }
            ),
            Err(MediaError::NoSuchTrack(3))
        ));
    }

    #[test]
    fn writes_mov_and_mp4_brands_that_probe_reads() {
        for brand in [Brand::Mp4, Brand::Mov] {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/fixtures/media");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(match brand {
                Brand::Mp4 => "unit-brand.mp4",
                Brand::Mov => "unit-brand.mov",
            });
            let mut muxer = Muxer::create(
                &path,
                MuxOptions {
                    brand,
                    movie_timescale: 600,
                },
            )
            .unwrap();
            let avc = AvcConfig::new(vec![vec![0x67, 0x42, 0, 0x1E]], vec![vec![0x68, 0xCE]], 4);
            let video = muxer
                .add_video(VideoTrackConfig {
                    width: 32,
                    height: 24,
                    timescale: 24,
                    avc,
                })
                .unwrap();
            muxer
                .write_packet(
                    video,
                    WritePacket {
                        dts: 0,
                        pts: 0,
                        duration: 1,
                        keyframe: true,
                        data: &[1, 2, 3],
                    },
                )
                .unwrap();
            muxer.finish().unwrap();
            let probed = probe(&path).unwrap();
            assert_eq!(probed.streams.len(), 1);
            assert_eq!(
                probed.streams.first().map(|stream| stream.sample_count),
                Some(1)
            );
            assert_eq!(
                probed.brand,
                if brand == Brand::Mp4 {
                    *b"isom"
                } else {
                    *b"qt  "
                }
            );
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn whole_track_priming_is_rejected() {
        let mut muxer =
            Muxer::new(std::io::Cursor::new(Vec::new()), MuxOptions::default()).unwrap();
        let audio = muxer
            .add_audio(AudioTrackConfig {
                aac: AacConfig::from_asc(vec![0x11, 0x90]),
                channels: 2,
                sample_rate: 48_000,
                timescale: 48_000,
                priming: 1024,
            })
            .unwrap();
        muxer
            .write_packet(
                audio,
                WritePacket {
                    dts: 0,
                    pts: 0,
                    duration: 1024,
                    keyframe: true,
                    data: &[0; 4],
                },
            )
            .unwrap();
        assert!(matches!(muxer.finish(), Err(MediaError::Mux(_))));
    }

    #[test]
    fn roundtrips_priming_through_the_demuxer() {
        let data = sample_file();
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/media");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unit-remux.mp4");
        std::fs::write(&path, &data).unwrap();
        let probed = probe(&path).unwrap();
        assert_eq!(probed.streams.len(), 2);
        assert_eq!(
            probed
                .streams
                .get(1)
                .and_then(|stream| stream.edits.first())
                .map(|edit| edit.media_time),
            Some(1024)
        );
        std::fs::remove_file(path).unwrap();
    }
}
