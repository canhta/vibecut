use crate::{CodecParameters, MediaError, Packet, Result, StreamInfo, TimeBase, error, init};
use ffmpeg_next as ff;
use std::fs::File;
use std::io::{Seek, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum Container {
    Mp4,
    Mov,
    WebM,
}
impl Container {
    /// Resolve the admitted video containers from an ASCII extension.
    pub fn from_extension(extension: &str) -> Option<Self> {
        [Self::Mp4, Self::Mov, Self::WebM]
            .into_iter()
            .find(|container| container.name().eq_ignore_ascii_case(extension))
    }
    fn name(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::WebM => "webm",
        }
    }
}
/// Streaming container output. Callers write to a temporary file and promote it
/// only after finish succeeds; dropping never hides a failed trailer as success.
pub struct Muxer {
    output: ff::format::context::Output,
    started: bool,
    time_bases: Vec<TimeBase>,
    sink: File,
}
impl Muxer {
    pub fn create(path: impl AsRef<Path>, container: Container) -> Result<Self> {
        Self::new(File::create(path)?, container)
    }
    pub fn new(writer: impl MuxSink + 'static, container: Container) -> Result<Self> {
        init()?;
        let sink = writer.sync_file()?;
        let io = ff::format::context::StreamIo::from_write_seek(writer)
            .map_err(error("allocate output I/O"))?;
        let output = ff::format::output_to_stream(io, None, Some(container.name()))
            .map_err(error("create muxer"))?;
        Ok(Self {
            output,
            started: false,
            time_bases: Vec::new(),
            sink,
        })
    }
    /// Container-level tags set before the first packet. Bounded so hostile
    /// attribution text cannot inflate a finalizing output without limit.
    pub fn set_metadata(&mut self, tags: &[(String, String)]) -> Result<()> {
        if self.started {
            return Err(MediaError::Invalid("metadata after first packet"));
        }
        if tags.len() > 32
            || tags
                .iter()
                .map(|(key, value)| key.len().saturating_add(value.len()))
                .try_fold(0usize, |total, len| total.checked_add(len))
                .is_none_or(|total| total > 64 * 1024)
            || tags.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || key.bytes().any(|b| b.is_ascii_control() || b == b'=')
                    || value.len() > 8 * 1024
                    || value
                        .bytes()
                        .any(|b| b.is_ascii_control() && !matches!(b, b'\n'))
            })
        {
            return Err(MediaError::TooLarge("container metadata"));
        }
        let mut dictionary = ff::Dictionary::new();
        for (key, value) in tags {
            dictionary.set(key, value);
        }
        self.output.set_metadata(dictionary);
        Ok(())
    }
    pub fn add_stream(&mut self, stream: &StreamInfo) -> Result<usize> {
        self.add_parameters(&stream.parameters, stream.time_base)
    }
    pub fn add_parameters(
        &mut self,
        parameters: &CodecParameters,
        time_base: TimeBase,
    ) -> Result<usize> {
        if self.started {
            return Err(MediaError::Invalid("add streams before first packet"));
        }
        if self.time_bases.len() >= 256 {
            return Err(MediaError::TooLarge("mux stream count"));
        }
        let mut stream = self
            .output
            .add_stream(ff::encoder::find(ff::codec::Id::None))
            .map_err(error("add mux stream"))?;
        stream.set_parameters(parameters.0.clone());
        crate::ffi::clear_codec_tag(&mut stream);
        stream.set_time_base(time_base.rational());
        let index = stream.index();
        self.time_bases.push(time_base);
        Ok(index)
    }
    fn start(&mut self) -> Result<()> {
        if !self.started {
            if self.time_bases.is_empty() {
                return Err(MediaError::Invalid("muxer has no streams"));
            }
            self.output
                .write_header()
                .map_err(error("write container header"))?;
            for (i, base) in self.time_bases.iter_mut().enumerate() {
                *base = TimeBase::from_ff(
                    self.output
                        .stream(i)
                        .ok_or(MediaError::Invalid("output stream"))?
                        .time_base(),
                )?;
            }
            self.started = true;
        }
        Ok(())
    }
    /// Packet buffers are retained by reference; only metadata is rescaled into
    /// the final mux time base. FFmpeg owns codec padding and edit-list creation.
    pub fn write_packet(&mut self, track: usize, packet: &Packet) -> Result<()> {
        packet.pts()?;
        packet.dts()?;
        packet.duration()?;
        if track >= self.time_bases.len() {
            return Err(MediaError::Invalid("output track"));
        }
        self.start()?;
        let target = *self
            .time_bases
            .get(track)
            .ok_or(MediaError::Invalid("output track"))?;
        let mut packet_copy = crate::ffi::reference_packet(&packet.inner)?;
        packet_copy.set_stream(track);
        packet_copy.set_position(-1);
        packet_copy.rescale_ts(packet.time_base.rational(), target.rational());
        packet_copy
            .write_interleaved(&mut self.output)
            .map_err(error("mux packet"))
    }
    pub fn finish(mut self) -> Result<()> {
        self.start()?;
        self.output
            .write_trailer()
            .map_err(error("write container trailer"))?;
        self.sink.sync_all()?;
        Ok(())
    }
}

/// Seekable output supplied by the publication owner. The returned File is used
/// only to sync after FFmpeg has finalized the trailer; writes use this sink.
pub trait MuxSink: Write + Seek + Send {
    fn sync_file(&self) -> std::io::Result<File>;
}
impl MuxSink for File {
    fn sync_file(&self) -> std::io::Result<File> {
        self.try_clone()
    }
}
