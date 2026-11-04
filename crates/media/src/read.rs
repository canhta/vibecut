use crate::{
    CodecParameters, MediaError, Packet, Probe, Result, StreamInfo, StreamKind, TimeBase, error,
    ffi, init,
};
use ffmpeg_next as ff;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use time::{FrameRate, Rounding, Tick};

const MAX_PACKET: usize = 64 << 20;
struct Input {
    file: File,
    bytes: Arc<AtomicU64>,
    budget: Arc<AtomicU64>,
}
impl Read for Input {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = self.budget.load(Ordering::Relaxed);
        if remaining == 0 {
            return Err(io::Error::other("media probe read budget exhausted"));
        }
        let take = bytes.len().min(remaining.min(usize::MAX as u64) as usize);
        let n = self.file.read(
            bytes
                .get_mut(..take)
                .ok_or_else(|| io::Error::other("read bounds"))?,
        )?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        self.budget.fetch_sub(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}
impl Seek for Input {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

pub struct PacketReader {
    pub(crate) input: ff::format::context::Input,
    probe: Probe,
    cancel: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
}
impl PacketReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// Opens native paths through Rust File + seekable AVIO (32 KiB buffer), with
    /// at most 32 MiB read while probing. The token also interrupts FFmpeg probing.
    pub fn open_with_cancel(path: impl AsRef<Path>, cancel: Arc<AtomicBool>) -> Result<Self> {
        init()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let bytes = Arc::new(AtomicU64::new(0));
        let budget = Arc::new(AtomicU64::new(32 << 20));
        let io = ff::format::context::StreamIo::from_read_seek(Input {
            file: File::open(path)?,
            bytes: bytes.clone(),
            budget: budget.clone(),
        })
        .map_err(error("allocate input I/O"))?;
        let token = cancel.clone();
        let mut options = ff::Dictionary::new();
        options.set("max_streams", "256");
        options.set("probesize", "33554432");
        options.set("analyzeduration", "5000000");
        options.set("max_index_size", "8388608");
        let input =
            ff::format::input_from_stream_with_interrupt(io, None, Some(options), move || {
                token.load(Ordering::Relaxed)
            })
            .map_err(|source| {
                if cancel.load(Ordering::Relaxed) {
                    MediaError::Cancelled
                } else {
                    error("probe input")(source)
                }
            })?;
        budget.store(u64::MAX, Ordering::Relaxed);
        if input.nb_streams() > 256 {
            return Err(MediaError::TooLarge("stream count"));
        }
        let mut streams = Vec::new();
        for stream in input.streams() {
            let parameters = stream.parameters();
            let time_base = TimeBase::from_ff(stream.time_base())?;
            let (video, audio) = ffi::parameter_info(&parameters)?;
            let rate = stream.avg_frame_rate();
            streams.push(StreamInfo {
                index: stream.index(),
                kind: match parameters.medium() {
                    ff::media::Type::Video => StreamKind::Video,
                    ff::media::Type::Audio => StreamKind::Audio,
                    _ => StreamKind::Other,
                },
                codec: parameters.id().name().to_owned(),
                time_base,
                duration: if stream.duration() > 0 {
                    Some(time_base.ticks(stream.duration())?)
                } else {
                    None
                },
                start: if stream.start_time() == i64::MIN {
                    None
                } else {
                    Some(time_base.ticks(stream.start_time())?)
                },
                frame_rate: FrameRate::new(
                    i64::from(rate.numerator()),
                    i64::from(rate.denominator()),
                )
                .ok(),
                frame_count: u64::try_from(stream.frames()).ok().filter(|n| *n > 0),
                variable_frame_rate: None,
                video,
                audio,
                parameters: CodecParameters(parameters.clone()),
            });
        }
        let duration = if input.duration() > 0 {
            Some(Tick::from_units(
                input.duration(),
                1_000_000,
                Rounding::Nearest,
            )?)
        } else {
            None
        };
        let probe = Probe {
            format: input.format().name().to_owned(),
            duration,
            streams,
            bytes_read: bytes.load(Ordering::Relaxed),
        };
        Ok(Self {
            input,
            probe,
            cancel,
            bytes,
        })
    }
    pub fn probe(&self) -> &Probe {
        &self.probe
    }
    pub fn bytes_read(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let mut inner = ff::Packet::empty();
        match inner.read(&mut self.input) {
            Ok(()) => {}
            Err(ff::Error::Eof) => return Ok(None),
            Err(source) => {
                return Err(if self.cancel.load(Ordering::Relaxed) {
                    MediaError::Cancelled
                } else {
                    error("read packet")(source)
                });
            }
        }
        if inner.size() > MAX_PACKET {
            return Err(MediaError::TooLarge("packet bytes"));
        }
        let stream = self
            .probe
            .streams
            .get(inner.stream())
            .ok_or(MediaError::Invalid("packet stream index"))?;
        Ok(Some(Packet {
            inner,
            time_base: stream.time_base,
        }))
    }
    /// Seek backward to a keyframe at or before this presentation time. Reset any
    /// decoder and decode forward, comparing frame tick PTS for an exact cut.
    pub fn seek(&mut self, time: Tick) -> Result<()> {
        let micros = time.to_units(1_000_000, Rounding::Floor)?;
        self.input
            .seek(micros, ..micros)
            .map_err(error("seek to preceding keyframe"))
    }
}
pub fn probe(path: impl AsRef<Path>) -> Result<Probe> {
    Ok(PacketReader::open(path)?.probe)
}

/// Sequential video access with automatic hardware-to-software recovery. On a
/// hardware decode failure, seeks to the last emitted frame's preceding keyframe
/// and decodes forward in software, suppressing already emitted presentation PTS.
/// Owns one bounded frame queue; it does not own an engine job or retry policy.
pub struct VideoReader {
    packets: PacketReader,
    decoder: crate::VideoDecoder,
    stream: StreamInfo,
    last_pts: Option<Tick>,
    minimum: Option<Tick>,
    recovering: bool,
    software: bool,
    finished: bool,
}
impl VideoReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let packets = PacketReader::open(path)?;
        let stream = packets
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(MediaError::Unsupported("no video stream"))?
            .clone();
        let decoder = crate::VideoDecoder::new(&stream, crate::DecodePreference::Hardware)?;
        Ok(Self {
            packets,
            decoder,
            stream,
            last_pts: None,
            minimum: None,
            recovering: false,
            software: false,
            finished: false,
        })
    }
    pub fn stream(&self) -> &StreamInfo {
        &self.stream
    }
    pub fn seek(&mut self, target: Tick) -> Result<()> {
        self.packets.seek(target)?;
        self.decoder.reset();
        self.minimum = Some(target);
        self.last_pts = None;
        self.recovering = false;
        self.finished = false;
        Ok(())
    }
    /// Decode from the preceding keyframe without suppressing earlier PTS. The
    /// compositor selects the frame whose presentation interval contains target.
    pub fn seek_keyframe(&mut self, target: Tick) -> Result<()> {
        self.seek(target)?;
        self.minimum = None;
        Ok(())
    }
    pub fn next_frame(&mut self) -> Result<Option<crate::VideoFrame>> {
        loop {
            let result = (|| {
                if let Some(frame) = self.decoder.pull_frame()? {
                    let pts = frame.pts()?.ok_or(MediaError::Unsupported(
                        "video presentation timestamp unavailable",
                    ))?;
                    if self.minimum.is_some_and(|min| pts < min)
                        || (self.recovering && self.last_pts.is_some_and(|last| pts <= last))
                    {
                        return Ok(None);
                    }
                    self.last_pts = Some(pts);
                    self.minimum = None;
                    self.recovering = false;
                    return Ok(Some(Some(frame)));
                }
                if self.finished {
                    return Ok(Some(None));
                }
                match self.packets.next_packet()? {
                    Some(packet) if packet.stream() == self.stream.index => {
                        self.decoder.push_packet(&packet)?
                    }
                    Some(_) => {}
                    None => {
                        self.finished = true;
                        self.decoder.finish_input()?;
                    }
                }
                Ok(None)
            })();
            match result {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {}
                Err(_) if !self.software => {
                    let target = self.last_pts.or(self.minimum).unwrap_or(Tick::ZERO);
                    self.packets.seek(target)?;
                    self.decoder =
                        crate::VideoDecoder::new(&self.stream, crate::DecodePreference::Software)?;
                    self.software = true;
                    self.finished = false;
                    self.recovering = true;
                }
                Err(error) => return Err(error),
            }
        }
    }
}
