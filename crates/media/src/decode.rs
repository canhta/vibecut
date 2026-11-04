use crate::{
    ColorInfo, MediaError, Packet, Result, StreamInfo, StreamKind, TimeBase, drained, error, ffi,
    init,
};
use ffmpeg_next as ff;
use time::Tick;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecodePreference {
    #[default]
    Hardware,
    Software,
}
/// Retains the decoded FFmpeg buffer, including GPU frames. No pixel copy occurs
/// until an explicit CPU download or format conversion.
pub struct VideoFrame {
    pub(crate) inner: ff::frame::Video,
    time_base: TimeBase,
    presentation: Option<Tick>,
}
impl VideoFrame {
    pub fn pts(&self) -> Result<Option<Tick>> {
        if let Some(pts) = self.presentation {
            return Ok(Some(pts));
        }
        self.inner
            .timestamp()
            .or(self.inner.pts())
            .map(|v| self.time_base.ticks(v))
            .transpose()
    }
    /// Retain pixels and assign an exact output-timeline PTS without copying planes.
    pub fn with_pts(&self, pts: Tick) -> Result<Self> {
        Ok(Self {
            inner: ffi::reference_video(&self.inner)?,
            time_base: self.time_base,
            presentation: Some(pts),
        })
    }
    pub fn width(&self) -> u32 {
        self.inner.width()
    }
    pub fn height(&self) -> u32 {
        self.inner.height()
    }
    pub fn pixel_format(&self) -> crate::PixelFormat {
        self.inner.format()
    }
    pub fn bit_depth(&self) -> Option<u8> {
        ffi::frame_bit_depth(&self.inner)
    }
    pub fn color(&self) -> ColorInfo {
        ffi::frame_color(&self.inner)
    }
    pub fn is_hardware(&self) -> bool {
        ffi::is_hardware(&self.inner)
    }
    /// Explicit CPU reference transfer, preserving color tags and source PTS.
    pub fn download(&self) -> Result<Self> {
        Ok(Self {
            inner: if self.is_hardware() {
                ffi::download(&self.inner)?
            } else {
                ffi::reference_video(&self.inner)?
            },
            time_base: self.time_base,
            presentation: self.presentation,
        })
    }
    pub fn plane(&self, index: usize) -> Result<&[u8]> {
        if self.is_hardware() {
            return Err(MediaError::Unsupported(
                "download GPU frames before CPU plane access",
            ));
        }
        if index >= self.inner.planes() {
            return Err(MediaError::Invalid("plane index"));
        }
        ffi::cpu_stride(&self.inner, index)?;
        Ok(self.inner.data(index))
    }
    pub fn stride(&self, index: usize) -> Result<usize> {
        if self.is_hardware() || index >= self.inner.planes() {
            return Err(MediaError::Invalid("CPU plane index"));
        }
        ffi::cpu_stride(&self.inner, index)
    }
    pub(crate) fn from_inner(inner: ff::frame::Video, time_base: TimeBase) -> Self {
        Self {
            inner,
            time_base,
            presentation: None,
        }
    }
}

pub struct VideoDecoder {
    decoder: ff::decoder::Video,
    time_base: TimeBase,
    stream: usize,
    finished: bool,
}
impl VideoDecoder {
    pub fn new(stream: &StreamInfo, preference: DecodePreference) -> Result<Self> {
        init()?;
        if stream.kind != StreamKind::Video {
            return Err(MediaError::Invalid("not a video stream"));
        }
        let mut context = ff::codec::context::Context::from_parameters(stream.parameters.0.clone())
            .map_err(error("create video decoder"))?;
        ffi::thread_and_limits(&mut context);
        // Hardware setup is added at the narrow FFI seam; initialization failures
        // return to this same decoder's software path without changing timing.
        if preference == DecodePreference::Hardware {
            ffi::hardware_decoder(&mut context);
        }
        let mut builder = context.decoder();
        builder.set_packet_time_base(stream.time_base.rational());
        let decoder = match builder.video() {
            Ok(decoder) => decoder,
            Err(_) if preference == DecodePreference::Hardware => {
                let mut software =
                    ff::codec::context::Context::from_parameters(stream.parameters.0.clone())
                        .map_err(error("create software decoder"))?;
                ffi::thread_and_limits(&mut software);
                let mut builder = software.decoder();
                builder.set_packet_time_base(stream.time_base.rational());
                builder.video().map_err(error("open software decoder"))?
            }
            Err(source) => return Err(error("open video decoder")(source)),
        };
        Ok(Self {
            decoder,
            time_base: stream.time_base,
            stream: stream.index,
            finished: false,
        })
    }
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<VideoFrame>> {
        if self.finished || packet.stream() != self.stream {
            return Err(MediaError::Invalid("decoder state or stream"));
        }
        self.decoder
            .send_packet(&packet.inner)
            .map_err(error("send video packet"))?;
        self.receive()
    }
    fn receive(&mut self) -> Result<Vec<VideoFrame>> {
        let mut frames = Vec::new();
        loop {
            let mut f = ff::frame::Video::empty();
            match self.decoder.receive_frame(&mut f) {
                Ok(()) => {
                    if f.width() == 0
                        || f.height() == 0
                        || u64::from(f.width()) * u64::from(f.height()) > 1 << 26
                    {
                        return Err(MediaError::TooLarge("decoded video dimensions"));
                    }
                    if frames.len() >= 64 {
                        return Err(MediaError::TooLarge("frames per access unit"));
                    }
                    frames.push(VideoFrame::from_inner(f, self.time_base));
                }
                Err(e) if drained(e) => break,
                Err(e) => return Err(error("receive video frame")(e)),
            }
        }
        Ok(frames)
    }
    pub fn flush(&mut self) -> Result<Vec<VideoFrame>> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.decoder
            .send_eof()
            .map_err(error("flush video decoder"))?;
        self.receive()
    }
    pub fn reset(&mut self) {
        self.decoder.flush();
        self.finished = false;
    }
}

/// Source-rate decoded audio. Conversion to planar f32 is explicit and never
/// changes rate, layout, priming or source PTS.
pub struct AudioFrame {
    pub(crate) inner: ff::frame::Audio,
    pub(crate) time_base: TimeBase,
    pub(crate) presentation: Option<Tick>,
}
impl AudioFrame {
    /// Construct bounded finite planar PCM for cropped speech chunks, audio mix
    /// output or tests. `pts` is the caller's explicit source/output tick origin.
    pub fn from_planar_f32(pts: Tick, sample_rate: u32, planes: &[Vec<f32>]) -> Result<Self> {
        init()?;
        let count = planes.first().map_or(0, Vec::len);
        if !(8_000..=192_000).contains(&sample_rate)
            || planes.is_empty()
            || planes.len() > 32
            || count == 0
            || count > 65_536
            || planes
                .iter()
                .any(|p| p.len() != count || p.iter().any(|v| !v.is_finite()))
        {
            return Err(MediaError::Invalid("planar audio input"));
        }
        let mut inner = ffi::audio_buffer(
            ff::format::Sample::F32(ff::format::sample::Type::Planar),
            count,
            ff::ChannelLayout::default(planes.len() as i32),
            sample_rate,
        )?;
        for (index, plane) in planes.iter().enumerate() {
            inner.plane_mut::<f32>(index).copy_from_slice(plane);
        }
        Ok(Self {
            inner,
            time_base: TimeBase::new(1, sample_rate as i32)?,
            presentation: Some(pts),
        })
    }
    pub fn pts(&self) -> Result<Option<Tick>> {
        if let Some(pts) = self.presentation {
            return Ok(Some(pts));
        }
        self.inner
            .timestamp()
            .or(self.inner.pts())
            .map(|v| self.time_base.ticks(v))
            .transpose()
    }
    pub fn sample_rate(&self) -> u32 {
        self.inner.rate()
    }
    pub fn samples(&self) -> usize {
        self.inner.samples()
    }
    pub fn channels(&self) -> u16 {
        self.inner.channels()
    }
    pub fn planar_f32(&self) -> Result<Vec<Vec<f32>>> {
        if self.channels() > 32 || self.samples() > 65_536 {
            return Err(MediaError::TooLarge("audio frame"));
        }
        let mut converter = ff::software::resampling::Context::get(
            self.inner.format(),
            self.inner.channel_layout(),
            self.inner.rate(),
            ff::format::Sample::F32(ff::format::sample::Type::Planar),
            self.inner.channel_layout(),
            self.inner.rate(),
        )
        .map_err(error("create audio conversion"))?;
        let mut output = ff::frame::Audio::empty();
        converter
            .run(&self.inner, &mut output)
            .map_err(error("convert planar audio"))?;
        Ok((0..usize::from(output.channels()))
            .map(|i| output.plane::<f32>(i).to_vec())
            .collect())
    }
}
pub struct AudioDecoder {
    decoder: ff::decoder::Audio,
    time_base: TimeBase,
    stream: usize,
    finished: bool,
}
impl AudioDecoder {
    pub fn new(stream: &StreamInfo) -> Result<Self> {
        init()?;
        if stream.kind != StreamKind::Audio {
            return Err(MediaError::Invalid("not an audio stream"));
        }
        let mut context = ff::codec::context::Context::from_parameters(stream.parameters.0.clone())
            .map_err(error("create audio decoder"))?;
        ffi::thread_and_limits(&mut context);
        let mut builder = context.decoder();
        builder.set_packet_time_base(stream.time_base.rational());
        Ok(Self {
            decoder: builder.audio().map_err(error("open audio decoder"))?,
            time_base: stream.time_base,
            stream: stream.index,
            finished: false,
        })
    }
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<AudioFrame>> {
        if self.finished || packet.stream() != self.stream {
            return Err(MediaError::Invalid("decoder state or stream"));
        }
        self.decoder
            .send_packet(&packet.inner)
            .map_err(error("send audio packet"))?;
        self.receive()
    }
    fn receive(&mut self) -> Result<Vec<AudioFrame>> {
        let mut frames = Vec::new();
        loop {
            let mut inner = ff::frame::Audio::empty();
            match self.decoder.receive_frame(&mut inner) {
                Ok(()) => {
                    if inner.samples() > 65_536 || inner.channels() > 32 || frames.len() >= 64 {
                        return Err(MediaError::TooLarge("audio output"));
                    }
                    frames.push(AudioFrame {
                        inner,
                        time_base: self.time_base,
                        presentation: None,
                    });
                }
                Err(e) if drained(e) => break,
                Err(e) => return Err(error("receive audio frame")(e)),
            }
        }
        Ok(frames)
    }
    pub fn flush(&mut self) -> Result<Vec<AudioFrame>> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.decoder
            .send_eof()
            .map_err(error("flush audio decoder"))?;
        self.receive()
    }
    pub fn reset(&mut self) {
        self.decoder.flush();
        self.finished = false;
    }
}
