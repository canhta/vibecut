//! Decodes access units into retained frames for batch and sequential callers.

use crate::{
    ColorInfo, MediaError, Packet, PacketReader, Result, StreamInfo, StreamKind, TimeBase, drained,
    error, ffi, init,
};
use ffmpeg_next as ff;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::Tick;

const MAX_VIDEO_DECODER_THREADS: usize = 8;
/// Reference pictures a decoder keeps at HD and above: the 8 slots of VP9 and AV1 bound
/// H.264 and HEVC at those sizes (levels allow 4-6).
const DECODER_REFERENCE_FRAMES: u64 = 8;

/// Frame-thread count every video decoder opens with: the host's cores, capped.
pub fn decoder_threads() -> u8 {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_VIDEO_DECODER_THREADS)) as u8
}

/// Bytes one [`VideoReader`] plus its [`RgbaConverter`] with `conversion_threads` workers hold
/// for a video stream: a decoded picture per decoder thread and reference slot, one for the
/// packet being decoded (an intra packet never exceeds its raw picture) and two for the
/// cursor's current and next pictures, the RGBA frame being converted, and each extra
/// conversion worker's slice scratch. Zero-size or non-video streams are refused.
pub fn video_decode_bytes(stream: &StreamInfo, conversion_threads: u8) -> Result<u64> {
    let info = stream
        .video
        .as_ref()
        .ok_or(MediaError::Invalid("video stream"))?;
    let pixels = u64::from(info.width) * u64::from(info.height);
    if pixels == 0 {
        return Err(MediaError::Invalid("video dimensions"));
    }
    let picture = pixels * ffi::video_bits_per_pixel(&stream.parameters.0) / 8;
    Ok(
        picture * (u64::from(decoder_threads()) + DECODER_REFERENCE_FRAMES + 3)
            + pixels * 4
            + u64::from(conversion_threads.saturating_sub(1)) * pixels,
    )
}

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

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeFault {
    Send,
    Receive,
    Drain,
}

#[cfg(test)]
thread_local! { pub(crate) static FRAME_DELIVERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub struct VideoDecoder {
    #[cfg(test)]
    pub(crate) fault: Option<DecodeFault>,
    decoder: ff::decoder::Video,
    time_base: TimeBase,
    stream: usize,
    finished: bool,
}
impl VideoDecoder {
    pub fn new(stream: &StreamInfo, preference: DecodePreference) -> Result<Self> {
        Self::with_backend(
            stream,
            preference,
            false,
            ffi::hardware_decoder,
            |builder| builder.video(),
        )
    }
    /// Bounded single-worker software decode for thumbnails and geometry recovery.
    pub(crate) fn for_thumbnail(stream: &StreamInfo) -> Result<Self> {
        Self::with_backend(
            stream,
            DecodePreference::Software,
            true,
            ffi::hardware_decoder,
            |builder| builder.video(),
        )
    }
    fn with_backend(
        stream: &StreamInfo,
        preference: DecodePreference,
        allow_unknown_dimensions: bool,
        setup: impl FnOnce(&mut ff::codec::context::Context),
        mut open: impl FnMut(ff::decoder::Decoder) -> std::result::Result<ff::decoder::Video, ff::Error>,
    ) -> Result<Self> {
        init()?;
        if stream.kind != StreamKind::Video {
            return Err(MediaError::Invalid("not a video stream"));
        }
        let mut context = ff::codec::context::Context::from_parameters(stream.parameters.0.clone())
            .map_err(error("create video decoder"))?;
        ffi::thread_and_limits(&mut context);
        if allow_unknown_dimensions {
            ffi::thumbnail_limits(&mut context);
        }
        match stream.video.as_ref() {
            Some(info) => {
                let area = u64::from(info.width) * u64::from(info.height);
                if area == 0
                    || area
                        > if allow_unknown_dimensions {
                            super::MAX_SOURCE_PIXELS
                        } else {
                            1 << 26
                        }
                {
                    return Err(MediaError::TooLarge("video dimensions"));
                }
            }
            None if allow_unknown_dimensions => {}
            None => return Err(MediaError::Invalid("video metadata")),
        }
        // Keep thread_and_limits' native pixel ceiling: FFmpeg also applies it
        // to codec scratch buffers, which can exceed the visible frame's area.
        // Hardware setup is added at the narrow FFI seam; initialization failures
        // return to this same decoder's software path without changing timing.
        if preference == DecodePreference::Hardware {
            setup(&mut context);
        }
        let mut builder = context.decoder();
        builder.set_packet_time_base(stream.time_base.rational());
        let decoder = match open(builder) {
            Ok(decoder) => decoder,
            Err(_) if preference == DecodePreference::Hardware => {
                let mut software =
                    ff::codec::context::Context::from_parameters(stream.parameters.0.clone())
                        .map_err(error("create software decoder"))?;
                ffi::thread_and_limits(&mut software);
                let mut builder = software.decoder();
                builder.set_packet_time_base(stream.time_base.rational());
                open(builder).map_err(error("open software decoder"))?
            }
            Err(source) => return Err(error("open video decoder")(source)),
        };
        Ok(Self {
            #[cfg(test)]
            fault: None,
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
        #[cfg(test)]
        self.fail_at(DecodeFault::Send, "send video packet")?;
        self.decoder
            .send_packet(&packet.inner)
            .map_err(error("send video packet"))
    }
    pub fn pull_frame(&mut self) -> Result<Option<VideoFrame>> {
        #[cfg(test)]
        self.fail_at(DecodeFault::Receive, "receive video frame")?;
        let mut frame = ff::frame::Video::empty();
        match self.decoder.receive_frame(&mut frame) {
            Ok(()) => {
                if frame.width() == 0
                    || frame.height() == 0
                    || u64::from(frame.width()) * u64::from(frame.height()) > 1 << 26
                {
                    return Err(MediaError::TooLarge("decoded video dimensions"));
                }
                #[cfg(test)]
                FRAME_DELIVERIES.with(|count| count.set(count.get() + 1));
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
        #[cfg(test)]
        self.fail_at(DecodeFault::Drain, "flush video decoder")?;
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
    #[cfg(test)]
    fn fail_at(&mut self, stage: DecodeFault, operation: &'static str) -> Result<()> {
        if self.fault == Some(stage) {
            self.fault = None;
            return Err(MediaError::Ffmpeg {
                operation,
                source: ff::Error::External,
            });
        }
        Ok(())
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
    /// Convert directly into bounded RGBA output; no full-size RGBA intermediary.
    pub fn convert_scaled(
        &mut self,
        frame: &VideoFrame,
        width: u32,
        height: u32,
    ) -> Result<VideoFrame> {
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            return Err(MediaError::TooLarge("scaled RGBA dimensions"));
        }
        let cpu = frame.download()?;
        Ok(VideoFrame {
            inner: self.scaler.rgba_scaled(&cpu.inner, width, height)?,
            time_base: frame.time_base,
            presentation: frame.presentation,
        })
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod acceleration_tests {
    use super::*;

    #[test]
    fn failed_hardware_device_or_open_uses_a_fresh_software_decoder() {
        let Some(path) = super::recovery_tests::fixture() else {
            return;
        };
        for fail_open in [false, true] {
            let mut packets = crate::PacketReader::open(&path).unwrap();
            let stream = packets
                .probe()
                .streams
                .iter()
                .find(|s| s.kind == StreamKind::Video)
                .unwrap()
                .clone();
            let mut attempts = 0;
            let mut decoder = VideoDecoder::with_backend(
                &stream,
                DecodePreference::Hardware,
                false,
                |context| {
                    if fail_open {
                        ffi::hardware_decoder(context);
                    }
                },
                |builder| {
                    attempts += 1;
                    if fail_open && attempts == 1 {
                        Err(ff::Error::External)
                    } else {
                        builder.video()
                    }
                },
            )
            .unwrap();
            assert_eq!(attempts, if fail_open { 2 } else { 1 });
            let mut count = 0;
            while let Some(packet) = packets.next_packet().unwrap() {
                if packet.stream() == stream.index {
                    for frame in decoder.decode(&packet).unwrap() {
                        assert!(!frame.is_hardware());
                        count += 1;
                    }
                }
            }
            count += decoder.flush().unwrap().len();
            assert_eq!(count, 48);
            assert!(decoder.flush().unwrap().is_empty());
        }
    }
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
    restart: Tick,
    recovering: bool,
    software: bool,
    finished: bool,
    thumbnail_only: bool,
}
impl VideoReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// The shared token interrupts opening, packet reads and frame delivery,
    /// including buffered frames and software recovery. Cancellation is terminal.
    pub fn open_with_cancel(path: impl AsRef<Path>, cancel: Arc<AtomicBool>) -> Result<Self> {
        Self::open_mode(path.as_ref(), cancel, false)
    }
    pub(crate) fn open_thumbnail(path: &Path, cancel: Arc<AtomicBool>) -> Result<Self> {
        Self::open_mode(path, cancel, true)
    }
    fn open_mode(path: &Path, cancel: Arc<AtomicBool>, thumbnail_only: bool) -> Result<Self> {
        let packets = if thumbnail_only {
            PacketReader::open_with_read_limit(path, cancel, 64 << 20)?
        } else {
            PacketReader::open_with_cancel(path, cancel)?
        };
        let stream = packets
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(MediaError::Unsupported("no video stream"))?
            .clone();
        let decoder = if thumbnail_only {
            crate::VideoDecoder::for_thumbnail(&stream)?
        } else {
            crate::VideoDecoder::new(&stream, crate::DecodePreference::Hardware)?
        };
        Ok(Self {
            packets,
            decoder,
            stream,
            last_pts: None,
            minimum: None,
            restart: Tick::ZERO,
            recovering: false,
            software: false,
            finished: false,
            thumbnail_only,
        })
    }
    pub(crate) fn take_recovered_thumbnail(&mut self) -> Option<crate::Thumbnail> {
        self.packets.recovered_thumbnail.take()
    }
    pub fn stream(&self) -> &StreamInfo {
        &self.stream
    }
    /// Bytes read from the file so far; probe and first-frame decode stay far
    /// below the whole-file size for long media.
    pub fn bytes_read(&self) -> u64 {
        self.packets.bytes_read()
    }
    #[cfg(test)]
    pub(crate) fn remaining_read_budget(&self) -> u64 {
        self.packets.remaining_read_budget()
    }
    /// Reopens one bounded read window between sampled positions of a thumbnail
    /// pass; packets already read stay consumed and the next leg gets its own
    /// allowance.
    pub(crate) fn start_read_window(&mut self, max_bytes: u64) {
        self.packets.start_read_window(max_bytes);
    }
    pub fn seek(&mut self, target: Tick) -> Result<()> {
        self.packets.seek(target)?;
        self.decoder.reset();
        self.minimum = Some(target);
        self.restart = target;
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
            if self.packets.cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
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
                Err(MediaError::Ffmpeg {
                    operation: "send video packet" | "receive video frame" | "flush video decoder",
                    ..
                }) if !self.software && !self.thumbnail_only => {
                    let target = self.last_pts.or(self.minimum).unwrap_or(self.restart);
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
pub(crate) mod recovery_tests {
    use super::*;
    use crate::{DecodePreference, RgbaConverter, VideoDecoder, VideoFrame};
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use time::Rounding;

    pub(crate) fn fixture() -> Option<std::path::PathBuf> {
        testkit::oracle::generate(
            &testkit::fixture::dir("media"),
            "accelerated recovery",
            "accelerated-recovery-bframes.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=192x108:rate=24:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
                "-c:v",
                "libx264",
                "-bf",
                "3",
                "-g",
                "12",
                "-pix_fmt",
                "yuv420p",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
                "-color_range",
                "tv",
                "-chroma_sample_location",
                "left",
                "-c:a",
                "aac",
            ],
        )
    }

    fn software(path: &Path) -> VideoReader {
        let mut reader = VideoReader::open(path).unwrap();
        reader.decoder = VideoDecoder::new(&reader.stream, DecodePreference::Software).unwrap();
        reader.software = true;
        reader
    }

    fn fingerprint(frame: &VideoFrame) -> (Tick, crate::ColorInfo, Vec<u8>) {
        let mut converter = RgbaConverter::new().unwrap();
        let rgba = converter.convert(frame).unwrap();
        let stride = rgba.stride(0).unwrap();
        let pixels = rgba
            .plane(0)
            .unwrap()
            .chunks(stride)
            .take(frame.height() as usize)
            .flat_map(|row| &row[..frame.width() as usize * 4])
            .copied()
            .collect();
        (frame.pts().unwrap().unwrap(), frame.color(), pixels)
    }

    fn frames(reader: &mut VideoReader) -> Vec<(Tick, crate::ColorInfo, Vec<u8>)> {
        let mut output = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            assert_eq!(frame.bit_depth(), Some(8));
            output.push(fingerprint(&frame));
        }
        output
    }

    #[test]
    fn decoder_loss_reseeks_without_dropping_or_repeating_presentation_frames() {
        let Some(path) = fixture() else { return };
        let expected = frames(&mut software(&path));
        assert_eq!(expected.len(), 48);
        for stage in [DecodeFault::Send, DecodeFault::Receive, DecodeFault::Drain] {
            let positions: &[usize] = if stage == DecodeFault::Receive {
                &[0, 1, 11, 12, 13, 46, 48]
            } else {
                &[0, 1, 11, 12, 13]
            };
            for &emitted in positions {
                let mut reader = VideoReader::open(&path).unwrap();
                let mut actual = Vec::new();
                for _ in 0..emitted {
                    actual.push(fingerprint(&reader.next_frame().unwrap().unwrap()));
                }
                reader.decoder.fault = Some(stage);
                actual.extend(frames(&mut reader));
                assert!(
                    reader.software,
                    "injected decoder failure must exercise recovery"
                );
                assert_eq!(actual.len(), expected.len());
                let mut maximum = 0;
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert_eq!(actual.0, expected.0);
                    assert_eq!(actual.1, expected.1);
                    for (a, b) in actual.2.iter().zip(&expected.2) {
                        maximum = maximum.max(a.abs_diff(*b));
                    }
                }
                assert!(
                    maximum <= 4,
                    "hardware/software RGBA channel difference {maximum}"
                );
            }
        }
    }

    #[test]
    fn decoder_loss_preserves_seek_minimum_and_software_errors_are_terminal() {
        let Some(path) = fixture() else { return };
        let expected = frames(&mut software(&path));
        for target in [0, 1, 12, 47] {
            let pts = expected[target].0;
            let mut reader = VideoReader::open(&path).unwrap();
            reader.seek(pts).unwrap();
            reader.decoder.fault = Some(DecodeFault::Receive);
            let actual = frames(&mut reader);
            assert_eq!(actual.len(), expected.len() - target);
            assert!(
                actual == expected[target..],
                "recovery changed exact seek output"
            );
            reader.seek(pts).unwrap();
            reader.decoder.fault = Some(DecodeFault::Receive);
            assert!(matches!(
                reader.next_frame(),
                Err(MediaError::Ffmpeg { .. })
            ));
        }
    }

    #[test]
    fn recovery_follows_reverse_keyframe_seeks_and_in_between_frame_targets() {
        let Some(path) = fixture() else { return };
        for index in [46, 13, 1, 0] {
            let mut baseline = software(&path);
            let target = Tick::from_units(index, 24, Rounding::Nearest).unwrap();
            baseline.seek_keyframe(target).unwrap();
            let expected = frames(&mut baseline);
            let mut reader = VideoReader::open(&path).unwrap();
            reader.seek_keyframe(target).unwrap();
            reader.decoder.fault = Some(DecodeFault::Receive);
            let actual = frames(&mut reader);
            assert_eq!(actual.len(), expected.len());
            assert!(actual == expected, "recovery changed keyframe seek output");
            reader.seek(Tick(target.0 + 1)).unwrap();
            let actual = frames(&mut reader);
            assert!(actual.first().unwrap().0 > target);
        }
    }

    #[test]
    fn missing_presentation_timestamps_keep_the_typed_error_without_recovery() {
        let Some(path) = testkit::oracle::generate(
            &testkit::fixture::dir("media"),
            "missing video timestamps",
            "accelerated-no-pts.h264",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=24:duration=1",
                "-c:v",
                "libx264",
                "-f",
                "h264",
            ],
        ) else {
            return;
        };
        let mut reader = VideoReader::open(path).unwrap();
        assert!(matches!(
            reader.next_frame(),
            Err(MediaError::Unsupported(
                "video presentation timestamp unavailable"
            ))
        ));
        assert!(!reader.software);
    }

    #[test]
    fn retained_frame_converts_for_the_compositor_after_its_reader_drops() {
        let Some(path) = fixture() else { return };
        let mut reader = VideoReader::open(path).unwrap();
        let first = reader.next_frame().unwrap().unwrap();
        let mut expected = fingerprint(&first);
        let pts = Tick(999);
        let retained = first.with_pts(pts).unwrap();
        assert_eq!(retained.is_hardware(), first.is_hardware());
        drop(first);
        drop(reader);
        expected.0 = pts;
        assert!(
            fingerprint(&retained) == expected,
            "retained frame pixels or metadata changed"
        );
    }

    #[test]
    fn cancellation_refuses_buffered_frames_and_does_not_restart_decoder() {
        let Some(path) = fixture() else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reader = VideoReader::open_with_cancel(&path, cancel.clone()).unwrap();
        assert!(reader.next_frame().unwrap().is_some());
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(reader.next_frame(), Err(MediaError::Cancelled)));
        assert!(!reader.software);
        assert!(matches!(
            VideoReader::open_with_cancel(&path, cancel),
            Err(MediaError::Cancelled)
        ));
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod recovery_benchmark {
    use super::*;
    use crate::{Container, Muxer, VideoEncoder, VideoEncoderSettings};
    use std::time::{Duration, Instant};
    use time::{FrameRate, Rounding};

    #[test]
    #[ignore = "manual 60-second hardware recovery throughput and quality measurement"]
    fn transcode_60_seconds_with_optional_decoder_loss() {
        let input = std::env::var_os("VIBECUT_ACCEL_BENCH_INPUT").expect("benchmark input");
        let output = std::env::var_os("VIBECUT_ACCEL_BENCH_OUTPUT").expect("benchmark output");
        let loss = std::env::var_os("VIBECUT_ACCEL_BENCH_LOSS").is_some();
        let start = Instant::now();
        let mut reader = VideoReader::open(input).unwrap();
        let mut encoder: Option<VideoEncoder> = None;
        let mut mux = Muxer::create(output, Container::Mp4).unwrap();
        let mut decode_time = Duration::ZERO;
        let mut encode_time = Duration::ZERO;
        let mut count = 0;
        let mut hardware = 0;
        loop {
            if loss && count == 900 {
                reader.decoder.fault = Some(DecodeFault::Receive);
            }
            let mark = Instant::now();
            let frame = reader.next_frame().unwrap();
            decode_time += mark.elapsed();
            let Some(frame) = frame else { break };
            assert_eq!(
                frame.pts().unwrap().unwrap(),
                Tick::from_units(count, 30, Rounding::Nearest).unwrap()
            );
            assert_eq!(frame.bit_depth(), Some(8));
            hardware += usize::from(frame.is_hardware());
            if encoder.is_none() {
                let settings = VideoEncoderSettings::h264(
                    frame.width(),
                    frame.height(),
                    FrameRate::new(30, 1).unwrap(),
                    frame.color(),
                );
                let enc = VideoEncoder::new(settings, &frame).unwrap();
                mux.add_parameters(&enc.parameters(), enc.time_base())
                    .unwrap();
                encoder = Some(enc);
            }
            let mark = Instant::now();
            let packets = encoder.as_mut().unwrap().encode(&frame).unwrap();
            encode_time += mark.elapsed();
            for packet in packets {
                mux.write_packet(0, &packet).unwrap();
            }
            count += 1;
        }
        let mark = Instant::now();
        let tail = encoder.as_mut().unwrap().flush().unwrap();
        encode_time += mark.elapsed();
        for packet in tail {
            mux.write_packet(0, &packet).unwrap();
        }
        mux.finish().unwrap();
        assert_eq!(count, 1800);
        assert_eq!(hardware, if loss { 900 } else { 1800 });
        eprintln!(
            "recovery_loss={loss} frames={count} hardware_frames={hardware} decode_seconds={:.6} decode_fps={:.3} encode_seconds={:.6} encode_fps={:.3} export_seconds={:.6}",
            decode_time.as_secs_f64(),
            count as f64 / decode_time.as_secs_f64(),
            encode_time.as_secs_f64(),
            count as f64 / encode_time.as_secs_f64(),
            start.elapsed().as_secs_f64()
        );
    }
}
