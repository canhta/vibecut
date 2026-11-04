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
    /// Straight RGBA8 at the caller's explicit transfer/color and output PTS.
    /// Positive dimensions, at most 2^26 pixels; exactly width*height*4 bytes.
    pub fn from_rgba8(
        width: u32,
        height: u32,
        pts: Tick,
        color: ColorInfo,
        pixels: &[u8],
    ) -> Result<Self> {
        init()?;
        let count = u64::from(width) * u64::from(height);
        if width == 0
            || height == 0
            || count > 1 << 26
            || count.checked_mul(4) != Some(pixels.len() as u64)
        {
            return Err(MediaError::Invalid("RGBA geometry"));
        }
        let mut inner = ffi::rgba_buffer(width, height, color)?;
        let stride = ffi::cpu_stride(&inner, 0)?;
        let row = width as usize * 4;
        for (dst, src) in inner
            .data_mut(0)
            .chunks_mut(stride)
            .zip(pixels.chunks_exact(row))
        {
            dst.get_mut(..row)
                .ok_or(MediaError::Invalid("RGBA stride"))?
                .copy_from_slice(src);
        }
        Ok(Self {
            inner,
            time_base: TimeBase::new(1, 1)?,
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
    pub fn premultiplied_alpha(&self) -> bool {
        ffi::premultiplied_alpha(&self.inner)
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
        let info = stream
            .video
            .as_ref()
            .ok_or(MediaError::Invalid("video metadata"))?;
        let area = u64::from(info.width) * u64::from(info.height);
        if area == 0 || area > 1 << 26 {
            return Err(MediaError::TooLarge("video dimensions"));
        }
        let pixels =
            (u64::from(info.width).div_ceil(64) * 64 * u64::from(info.height).div_ceil(64) * 64)
                .min(1 << 26);
        ffi::decoder_bounds(&mut context, Some(pixels), None);
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
                ffi::decoder_bounds(&mut software, Some(pixels), None);
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
    /// Send one access unit after draining pull_frame to None. This pull seam
    /// lets sequential consumers retain only their actual look-ahead frames.
    pub fn push_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.finished || packet.stream() != self.stream {
            return Err(MediaError::Invalid("decoder state or stream"));
        }
        self.decoder
            .send_packet(&packet.inner)
            .map_err(error("send video packet"))
    }
    pub fn pull_frame(&mut self) -> Result<Option<VideoFrame>> {
        let mut frame = ff::frame::Video::empty();
        match self.decoder.receive_frame(&mut frame) {
            Ok(()) => {
                if frame.width() == 0
                    || frame.height() == 0
                    || u64::from(frame.width()) * u64::from(frame.height()) > 1 << 26
                {
                    return Err(MediaError::TooLarge("decoded video dimensions"));
                }
                Ok(Some(VideoFrame::from_inner(frame, self.time_base)))
            }
            Err(e) if drained(e) => Ok(None),
            Err(e) => Err(error("receive video frame")(e)),
        }
    }
    pub fn finish_input(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.decoder
            .send_eof()
            .map_err(error("flush video decoder"))
    }
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<VideoFrame>> {
        self.push_packet(packet)?;
        self.receive()
    }
    fn receive(&mut self) -> Result<Vec<VideoFrame>> {
        let mut frames = Vec::new();
        while let Some(frame) = self.pull_frame()? {
            if frames.len() >= 64 {
                return Err(MediaError::TooLarge("frames per access unit"));
            }
            frames.push(frame);
        }
        Ok(frames)
    }
    pub fn flush(&mut self) -> Result<Vec<VideoFrame>> {
        self.finish_input()?;
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
        let channels = stream
            .audio
            .as_ref()
            .ok_or(MediaError::Invalid("audio metadata"))?
            .channels;
        if channels == 0 || channels > 32 {
            return Err(MediaError::TooLarge("audio channels"));
        }
        ffi::decoder_bounds(&mut context, None, Some(65536 * u64::from(channels)));
        let mut builder = context.decoder();
        builder.set_packet_time_base(stream.time_base.rational());
        Ok(Self {
            decoder: builder.audio().map_err(error("open audio decoder"))?,
            time_base: stream.time_base,
            stream: stream.index,
            finished: false,
        })
    }
    pub fn push_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.finished || packet.stream() != self.stream {
            return Err(MediaError::Invalid("decoder state or stream"));
        }
        self.decoder
            .send_packet(&packet.inner)
            .map_err(error("send audio packet"))
    }
    pub fn pull_frame(&mut self) -> Result<Option<AudioFrame>> {
        let mut inner = ff::frame::Audio::empty();
        match self.decoder.receive_frame(&mut inner) {
            Ok(()) => {
                if inner.samples() > 65536 || inner.channels() > 32 {
                    return Err(MediaError::TooLarge("audio output"));
                }
                Ok(Some(AudioFrame {
                    inner,
                    time_base: self.time_base,
                    presentation: None,
                }))
            }
            Err(e) if drained(e) => Ok(None),
            Err(e) => Err(error("receive audio frame")(e)),
        }
    }
    pub fn finish_input(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.decoder
            .send_eof()
            .map_err(error("flush audio decoder"))
    }
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<AudioFrame>> {
        self.push_packet(packet)?;
        self.receive()
    }
    fn receive(&mut self) -> Result<Vec<AudioFrame>> {
        let mut frames = Vec::new();
        while let Some(frame) = self.pull_frame()? {
            if frames.len() >= 64 {
                return Err(MediaError::TooLarge("audio output"));
            }
            frames.push(frame);
        }
        Ok(frames)
    }
    pub fn flush(&mut self) -> Result<Vec<AudioFrame>> {
        self.finish_input()?;
        self.receive()
    }
    pub fn reset(&mut self) {
        self.decoder.flush();
        self.finished = false;
    }
}

/// Reusable frame-aware conversion; produces CPU RGBA, retaining the alpha convention and without changing
/// transfer or primaries. Render owns conversion into its linear working space.
pub struct RgbaConverter {
    scaler: ffi::Scaler,
}
impl RgbaConverter {
    pub fn new() -> Result<Self> {
        Self::with_threads(1)
    }
    /// Bounded FFmpeg conversion concurrency (1..=8), supplied and accounted
    /// by the execution owner. Pixel/color settings are unchanged.
    pub fn with_threads(threads: u8) -> Result<Self> {
        init()?;
        Ok(Self {
            scaler: ffi::Scaler::with_threads(threads)?,
        })
    }
    /// Convert a horizontal sampling window with a 64-pixel reconstruction
    /// halo on aligned NV12/YUV420P CPU frames. Returns the RGBA image and its
    /// integer source-column origin; other layouts retain a full-frame result.
    /// The requested range must be nonempty and inside the original width.
    /// Only the requested interior is an exact replacement for full conversion.
    pub fn convert_horizontal(
        &mut self,
        frame: &VideoFrame,
        window: std::ops::Range<u32>,
    ) -> Result<(VideoFrame, u32)> {
        if window.start >= window.end || window.end > frame.width() {
            return Err(MediaError::Invalid("conversion window"));
        }
        let cpu = frame.download()?;
        let left = window.start.saturating_sub(64) / 128 * 128;
        let right = window
            .end
            .saturating_add(64)
            .div_ceil(128)
            .saturating_mul(128)
            .min(frame.width());
        let cropped = ffi::horizontal_window(&cpu.inner, left, right)?;
        let origin = if cropped.is_some() { left } else { 0 };
        let source = cropped.as_ref().unwrap_or(&cpu.inner);
        Ok((
            VideoFrame {
                inner: self.scaler.rgba(source)?,
                time_base: frame.time_base,
                presentation: frame.presentation,
            },
            origin,
        ))
    }
    pub fn convert(&mut self, frame: &VideoFrame) -> Result<VideoFrame> {
        let cpu = frame.download()?;
        Ok(VideoFrame {
            inner: self.scaler.rgba(&cpu.inner)?,
            time_base: frame.time_base,
            presentation: frame.presentation,
        })
    }
}
