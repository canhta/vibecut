//! Encodes video and audio under explicit quality and timing boundaries.

use crate::{
    AudioFrame, CodecParameters, ColorInfo, MediaError, Packet, Result, TimeBase, VideoFrame,
    drained, error, ffi, init,
};
use ffmpeg_next as ff;
use time::{FrameRate, Rounding};

// Bound retained output by bytes, not by codec latency. Charge vector capacity
// slack and packet metadata as well, so tiny/empty packets cannot grow forever.
const MAX_ENCODED_VIDEO_BATCH_BYTES: usize = 64 * 1024 * 1024;
const MAX_VIDEO_ENCODER_THREADS: usize = 8;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum H264Profile {
    #[default]
    Automatic,
    Main,
    High,
}

fn encoded_video_batch_bytes(retained: usize, packet: &ff::Packet) -> Result<usize> {
    let bytes = retained
        .checked_add(4 * std::mem::size_of::<Packet>())
        .and_then(|n| n.checked_add(ff::ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize))
        .and_then(|n| n.checked_add(packet.size()))
        .and_then(|bytes| {
            packet.side_data().try_fold(bytes, |bytes, side| {
                bytes
                    .checked_add(std::mem::size_of::<ff::ffi::AVPacketSideData>())
                    .and_then(|n| n.checked_add(ff::ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize))
                    .and_then(|n| n.checked_add(side.data().len()))
            })
        });
    bytes
        .filter(|&n| n <= MAX_ENCODED_VIDEO_BATCH_BYTES)
        .ok_or(MediaError::TooLarge("encoded video batch bytes"))
}

#[derive(Clone, Copy, Debug, Default)]
pub enum EncodePreference {
    #[cfg_attr(not(windows), default)]
    Hardware,
    // Hardware MF is correct with copied NV12 input (p0-core/78) but remains
    // opt-in on Windows until other vendors' encoders are verified.
    #[cfg_attr(windows, default)]
    Software,
}

#[derive(Clone, Debug)]
pub struct VideoEncoderSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub bit_rate: usize,
    pub quality: u8,
    pub profile: H264Profile,
    pub color: ColorInfo,
    pub webm: bool,
    /// FFmpeg input-conversion workers, 1..=8; caller owns scheduling/accounting.
    pub conversion_threads: u8,
}
impl VideoEncoderSettings {
    /// 85/100 quality and 12 Mb/s minimum for a 1080p canvas at 30 fps. Callers
    /// choose a different target explicitly; hardware failure never lowers it.
    pub fn h264(width: u32, height: u32, frame_rate: FrameRate, color: ColorInfo) -> Self {
        Self {
            width,
            height,
            frame_rate,
            bit_rate: 12_000_000,
            quality: 85,
            profile: H264Profile::Automatic,
            color,
            webm: false,
            conversion_threads: 1,
        }
    }
}
// FFmpeg gates QSCALE by architecture; opening a hardware-only session tests
// the selected encoder's Quality property. A fresh bitrate attempt permits the
// platform software fallback without sending it an unsupported property.
fn videotoolbox_quality_attempts(qscale_supported: bool, software: bool) -> &'static [bool] {
    if qscale_supported && !software {
        &[true, false]
    } else {
        &[false]
    }
}

// NVENC reserves zero for automatic CQ; QSV disables ICQ at zero.
// Both backends need a positive level to retain an explicit quality request.
fn explicit_quality_quantizer(quality: u8) -> u32 {
    ((100 - u32::from(quality)) * 51 / 100).max(1)
}

fn video_open_error(last: Option<ff::Error>, software_h264: bool, webm: bool) -> MediaError {
    match last {
        Some(source) => MediaError::Ffmpeg {
            operation: "open video encoder",
            source,
        },
        None if webm => MediaError::Unsupported("VP9 encoder unavailable in this FFmpeg build"),
        None if software_h264 => MediaError::EncoderUnavailable { source: None },
        None => MediaError::Unsupported("no available encoder for target quality"),
    }
}

/// Registry lookup for the one audio encoder a container admits. A missing
/// encoder names the absent capability; there is no fallback pair.
fn audio_encoder(name: &'static str, webm: bool) -> Result<ff::Codec> {
    ff::encoder::find_by_name(name).ok_or(MediaError::Unsupported(if webm {
        "Opus encoder unavailable in this FFmpeg build"
    } else {
        "AAC encoder unavailable in this FFmpeg build"
    }))
}

/// Encoders this host can open for MP4, in preference order.
fn video_candidates(webm: bool) -> &'static [(&'static str, bool)] {
    if webm {
        &[("libvpx-vp9", true)]
    } else if cfg!(target_os = "macos") {
        &[("h264_videotoolbox", true)]
    } else if cfg!(target_os = "linux") {
        // Hardware first; the GPL build carries libx264 for software export.
        &[
            ("h264_nvenc", true),
            ("h264_vaapi", true),
            ("libx264", false),
        ]
    } else {
        &[
            ("h264_nvenc", true),
            ("h264_qsv", true),
            ("h264_amf", true),
            ("h264_mf", true),
            ("h264_mf", false),
        ]
    }
}

/// True when this build exposes an MP4 video encoder the export path can open.
pub fn h264_encoder_available() -> bool {
    video_candidates(false)
        .iter()
        .any(|(name, _)| ff::encoder::find_by_name(name).is_some())
}

/// Registry admission for the built-in video/audio pairs and the silent GIF
/// encoder; opening still reports device-specific failures through the normal
/// encoder error path.
pub fn export_container_available(container: crate::Container) -> bool {
    if matches!(container, crate::Container::Gif) {
        return ff::encoder::find_by_name("gif").is_some();
    }
    let webm = matches!(container, crate::Container::WebM);
    video_candidates(webm)
        .iter()
        .any(|(name, _)| ff::encoder::find_by_name(name).is_some())
        && ff::encoder::find_by_name(if webm { "libopus" } else { "aac" }).is_some()
}

/// Registry admission for audio-only exports: AAC in MP4 or Opus in WebM, the
/// same audio encoders the video pairs use.
pub fn audio_export_container_available(container: crate::Container) -> bool {
    let webm = matches!(container, crate::Container::WebM);
    ff::encoder::find_by_name(if webm { "libopus" } else { "aac" }).is_some()
}

/// Drains every packet the encoder has ready under the retained-byte bound.
/// After `send_eof` (`finished`) it drains through EOF, not EAGAIN.
pub(super) fn receive_video_packets(
    encoder: &mut ff::encoder::Video,
    time_base: TimeBase,
    finished: bool,
) -> Result<Vec<Packet>> {
    let mut packets = Vec::new();
    let mut retained_bytes = 0;
    loop {
        let mut inner = ff::Packet::empty();
        match encoder.receive_packet(&mut inner) {
            Ok(()) => {
                retained_bytes = encoded_video_batch_bytes(retained_bytes, &inner)?;
                packets
                    .try_reserve(1)
                    .map_err(|_| MediaError::TooLarge("encoded video batch allocation"))?;
                if inner.duration() <= 0 {
                    inner.set_duration(1);
                }
                packets.push(Packet { inner, time_base });
            }
            Err(ff::Error::Eof) => break,
            Err(e) if !finished && drained(e) => break,
            Err(e) => return Err(error("receive encoded video")(e)),
        }
    }
    Ok(packets)
}

pub struct VideoEncoder {
    encoder: ff::encoder::Video,
    time_base: TimeBase,
    settings: VideoEncoderSettings,
    scaler: Option<ffi::Scaler>,
    finished: bool,
    name: String,
    last_pts: Option<i64>,
}
impl VideoEncoder {
    pub fn new(settings: VideoEncoderSettings, first: &VideoFrame) -> Result<Self> {
        Self::with_preference(settings, first, EncodePreference::default())
    }
    pub fn with_preference(
        mut settings: VideoEncoderSettings,
        first: &VideoFrame,
        preference: EncodePreference,
    ) -> Result<Self> {
        init()?;
        if settings.width == 0
            || settings.height == 0
            || !settings.width.is_multiple_of(2)
            || !settings.height.is_multiple_of(2)
            || u64::from(settings.width) * u64::from(settings.height) > 1 << 26
            || !(1..=8).contains(&settings.conversion_threads)
            || settings.quality > 100
            || settings.bit_rate == 0
        {
            return Err(MediaError::Invalid("video encoder settings"));
        }
        if first.bit_depth().is_some_and(|depth| depth > 8) {
            return Err(MediaError::Unsupported(
                "high bit depth requires explicit conversion or stream copy",
            ));
        }
        if settings.color.is_hdr() || first.color().is_hdr() {
            return Err(MediaError::Unsupported(
                "HDR must be explicitly tone-mapped or stream-copied; SDR encode refuses HDR",
            ));
        }
        let rate = settings.frame_rate;
        let time_base = TimeBase::new(
            i32::try_from(rate.den()).map_err(|_| MediaError::Invalid("frame rate"))?,
            i32::try_from(rate.num()).map_err(|_| MediaError::Invalid("frame rate"))?,
        )?;
        let candidates = video_candidates(settings.webm);
        let mut last = None;
        for &(name, hardware) in candidates {
            if matches!(preference, EncodePreference::Software)
                && hardware
                && name != "h264_videotoolbox"
                && !settings.webm
            {
                continue;
            }
            let Some(codec) = ff::encoder::find_by_name(name) else {
                continue;
            };
            let attempts = if name == "h264_videotoolbox" {
                videotoolbox_quality_attempts(
                    cfg!(target_arch = "aarch64"),
                    matches!(preference, EncodePreference::Software),
                )
            } else {
                &[false]
            };
            for &quality_mode in attempts {
                let mut encoder = ff::codec::context::Context::new_with_codec(codec)
                    .encoder()
                    .video()
                    .map_err(error("create video encoder"))?;
                encoder.set_width(settings.width);
                encoder.set_height(settings.height);
                encoder.set_time_base(time_base.rational());
                encoder.set_frame_rate(Some((time_base.denominator(), time_base.numerator())));
                encoder.set_bit_rate(settings.bit_rate);
                encoder.set_gop(60);
                encoder.set_max_b_frames(0);
                // Avoid automatic thread counts (and retained frame queues)
                // growing with host CPU count. This is independent of scaler workers.
                let threads = std::thread::available_parallelism()
                    .map_or(1, |n| n.get().min(MAX_VIDEO_ENCODER_THREADS));
                encoder.set_threading(ff::codec::threading::Config::count(threads));
                encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER);
                // FFmpeg mfenc wraps a D3D11 subresource without retaining the
                // AVFrame's decoder-pool lease while its asynchronous MFT reads
                // it. NV12 takes mfenc's owned sample-copy path instead. This
                // keeps hardware encoding available without reusing GPU input.
                encoder.set_format(
                    if !settings.webm
                        && name != "h264_mf"
                        && hardware
                        && !matches!(preference, EncodePreference::Software)
                        && first.is_hardware()
                        && first.width() == settings.width
                        && first.height() == settings.height
                        && codec.video().ok().and_then(|v| v.formats()).is_some_and(
                            |mut formats| formats.any(|format| format == first.inner.format()),
                        )
                    {
                        first.inner.format()
                    } else if settings.webm {
                        ff::format::Pixel::YUV420P
                    } else {
                        ff::format::Pixel::NV12
                    },
                );
                if first.is_hardware() && encoder.format() == first.inner.format() {
                    ffi::share_hardware_frames(&mut encoder, &first.inner)?;
                }
                settings.color = settings.color.resolved_sdr();
                if settings.color != first.color().resolved_sdr() {
                    return Err(MediaError::Unsupported(
                        "color conversion must be explicit before encoding",
                    ));
                }
                ffi::encoder_color(&mut encoder, settings.color)?;
                let mut options = ff::Dictionary::new();
                if !settings.webm {
                    match settings.profile {
                        H264Profile::Automatic => {}
                        // MF exposes the generic integer AVCodecContext option;
                        // unlike the other backends it has no named constants.
                        H264Profile::Main => {
                            options.set("profile", if name == "h264_mf" { "77" } else { "main" })
                        }
                        H264Profile::High => {
                            options.set("profile", if name == "h264_mf" { "100" } else { "high" })
                        }
                    }
                } else if settings.profile != H264Profile::Automatic {
                    return Err(MediaError::Unsupported("H.264 profile on VP9"));
                }
                match name {
                    "h264_videotoolbox" => {
                        options.set("allow_sw", if quality_mode { "0" } else { "1" });
                        if matches!(preference, EncodePreference::Software) {
                            options.set("require_sw", "1");
                        }
                        if quality_mode {
                            encoder.set_quality(usize::from(settings.quality) * 118);
                            encoder.set_flags(
                                ff::codec::Flags::GLOBAL_HEADER | ff::codec::Flags::QSCALE,
                            );
                        }
                    }
                    "h264_nvenc" => {
                        options.set("preset", "p6");
                        options.set("rc", "vbr");
                        options.set(
                            "cq",
                            &explicit_quality_quantizer(settings.quality).to_string(),
                        );
                    }
                    "h264_qsv" => {
                        options.set("preset", "slow");
                        options.set(
                            "global_quality",
                            &explicit_quality_quantizer(settings.quality).to_string(),
                        );
                    }
                    "h264_amf" => {
                        options.set("quality", "quality");
                        options.set("rc", "qvbr");
                        options.set(
                            "qvbr_quality_level",
                            &((100 - u32::from(settings.quality)) * 51 / 100).to_string(),
                        );
                    }
                    "h264_mf" => {
                        options.set("hw_encoding", if hardware { "1" } else { "0" });
                        options.set("rate_control", "quality");
                        options.set("quality", &settings.quality.to_string());
                    }
                    "libvpx-vp9" => {
                        options.set(
                            "crf",
                            &((100 - u32::from(settings.quality)) * 63 / 100).to_string(),
                        );
                        options.set("deadline", "good");
                        options.set("cpu-used", "2");
                    }
                    _ => {}
                }
                match encoder.open_with(options) {
                    Ok(encoder) => {
                        return Ok(Self {
                            encoder,
                            time_base,
                            settings,
                            scaler: None,
                            finished: false,
                            last_pts: None,
                            name: if hardware && !matches!(preference, EncodePreference::Software) {
                                name.to_owned()
                            } else {
                                format!("{name} (software)")
                            },
                        });
                    }
                    Err(e) => last = Some(e),
                }
            }
        }
        Err(video_open_error(
            last,
            !settings.webm && matches!(preference, EncodePreference::Software),
            settings.webm,
        ))
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }
    pub fn parameters(&self) -> CodecParameters {
        CodecParameters(ff::codec::Parameters::from(&self.encoder))
    }
    pub fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<Packet>> {
        if self.finished {
            return Err(MediaError::Invalid("video encoder finished"));
        }
        if frame.color().is_hdr() {
            return Err(MediaError::Unsupported("HDR requires explicit handling"));
        }
        if frame.bit_depth().is_some_and(|depth| depth > 8)
            || frame.color().resolved_sdr() != self.settings.color
        {
            return Err(MediaError::Unsupported(
                "source depth/color changed; conversion must be explicit",
            ));
        }
        let shared = ffi::compatible_hardware_frame(&self.encoder, &frame.inner);
        let mut output = if frame.inner.format() == self.encoder.format()
            && (!frame.is_hardware() || shared)
            && frame.width() == self.settings.width
            && frame.height() == self.settings.height
        {
            ffi::reference_video(&frame.inner)?
        } else {
            let cpu = frame.download()?;
            if self.scaler.is_none() {
                self.scaler = Some(ffi::Scaler::with_threads(self.settings.conversion_threads)?);
            }
            let format = ffi::encoder_software_format(&self.encoder);
            let converted = self
                .scaler
                .as_mut()
                .ok_or(MediaError::Invalid("scaler state"))?
                .scale(
                    &cpu.inner,
                    self.settings.width,
                    self.settings.height,
                    format,
                    &self.encoder,
                )?;
            ffi::upload_for_encoder(&self.encoder, converted)?
        };
        let pts = frame.pts()?.ok_or(MediaError::Unsupported(
            "video encoding requires output PTS",
        ))?;
        let units = self.time_base.units(pts, Rounding::Nearest)?;
        if (i128::from(pts.0) - i128::from(self.time_base.ticks(units)?.0)).abs() > 1
            || self
                .last_pts
                .is_some_and(|last| last.checked_add(1) != Some(units))
        {
            return Err(MediaError::Invalid(
                "video frames must follow the requested constant-rate grid",
            ));
        }
        output.set_pts(Some(units));
        self.last_pts = Some(units);
        output.set_kind(ff::picture::Type::None);
        ffi::encoder_frame_metadata(&self.encoder, &mut output);
        self.encoder
            .send_frame(&output)
            .map_err(error("send video frame"))?;
        self.receive()
    }
    fn receive(&mut self) -> Result<Vec<Packet>> {
        receive_video_packets(&mut self.encoder, self.time_base, self.finished)
    }
    pub fn flush(&mut self) -> Result<Vec<Packet>> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.encoder
            .send_eof()
            .map_err(error("flush video encoder"))?;
        self.receive()
    }
}

#[derive(Clone, Debug)]
pub struct AudioEncoderSettings {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_rate: usize,
    pub webm: bool,
}
impl AudioEncoderSettings {
    pub fn aac(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
            bit_rate: usize::from(channels) * 128_000,
            webm: false,
        }
    }
}
pub struct AudioEncoder {
    encoder: ff::encoder::Audio,
    time_base: TimeBase,
    finished: bool,
    fifo: ffi::AudioFifo,
    converter: Option<ff::software::resampling::Context>,
    next_pts: Option<i64>,
    input_end: Option<i64>,
}
impl AudioEncoder {
    pub fn new(settings: AudioEncoderSettings) -> Result<Self> {
        init()?;
        if settings.channels == 0
            || settings.channels > 8
            || !(8_000..=192_000).contains(&settings.sample_rate)
            || settings.bit_rate == 0
        {
            return Err(MediaError::Invalid("audio encoder settings"));
        }
        let name = if settings.webm { "libopus" } else { "aac" };
        let codec = audio_encoder(name, settings.webm)?;
        let mut encoder = ff::codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(error("create audio encoder"))?;
        encoder.set_rate(settings.sample_rate as i32);
        encoder.set_channel_layout(ff::ChannelLayout::default(i32::from(settings.channels)));
        encoder.set_format(ff::format::Sample::F32(if settings.webm {
            ff::format::sample::Type::Packed
        } else {
            ff::format::sample::Type::Planar
        }));
        encoder.set_bit_rate(settings.bit_rate);
        encoder.set_time_base((1, settings.sample_rate as i32));
        encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        let encoder = encoder.open().map_err(error("open audio encoder"))?;
        if encoder.frame_size() == 0 || encoder.frame_size() > 65_536 {
            return Err(MediaError::Unsupported("audio encoder frame size"));
        }
        let fifo = ffi::AudioFifo::new(encoder.format(), encoder.channel_layout(), encoder.rate())?;
        Ok(Self {
            encoder,
            time_base: TimeBase::new(1, settings.sample_rate as i32)?,
            finished: false,
            fifo,
            converter: None,
            next_pts: None,
            input_end: None,
        })
    }
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }
    pub fn parameters(&self) -> CodecParameters {
        CodecParameters(ff::codec::Parameters::from(&self.encoder))
    }
    pub fn frame_samples(&self) -> u32 {
        self.encoder.frame_size()
    }
    pub fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<Packet>> {
        if self.finished {
            return Err(MediaError::Invalid("audio encoder finished"));
        }
        if frame.sample_rate() != self.encoder.rate()
            || frame.inner.channel_layout() != self.encoder.channel_layout()
        {
            return Err(MediaError::Unsupported(
                "audio rate/layout change must be explicit",
            ));
        }
        let pts = self.time_base.units(
            frame.pts()?.ok_or(MediaError::Unsupported(
                "audio encoding requires source PTS",
            ))?,
            Rounding::Nearest,
        )?;
        if frame.samples() == 0 {
            return Ok(Vec::new());
        }
        let tolerance = self
            .time_base
            .units(frame.time_base.ticks(1)?, Rounding::Ceil)?
            .max(1);
        if self
            .input_end
            .is_some_and(|end| (i128::from(end) - i128::from(pts)).abs() > i128::from(tolerance))
        {
            return Err(MediaError::Invalid("audio source discontinuity"));
        }
        self.next_pts.get_or_insert(pts);
        self.input_end = Some(
            self.input_end
                .unwrap_or(pts)
                .checked_add(frame.samples() as i64)
                .ok_or(MediaError::TooLarge("audio timestamp"))?,
        );
        let converted = if frame.inner.format() == self.encoder.format() {
            ffi::reference_audio(&frame.inner)?
        } else {
            if self.converter.is_none() {
                self.converter = Some(
                    ff::software::resampling::Context::get(
                        frame.inner.format(),
                        frame.inner.channel_layout(),
                        frame.sample_rate(),
                        self.encoder.format(),
                        self.encoder.channel_layout(),
                        self.encoder.rate(),
                    )
                    .map_err(error("create encoder sample conversion"))?,
                );
            }
            let converter = self
                .converter
                .as_mut()
                .ok_or(MediaError::Invalid("audio converter state"))?;
            if converter.input().format != frame.inner.format() {
                return Err(MediaError::Unsupported("audio sample format changed"));
            }
            let mut output = ffi::audio_buffer(
                self.encoder.format(),
                frame.samples(),
                self.encoder.channel_layout(),
                self.encoder.rate(),
            )?;
            converter
                .run(&frame.inner, &mut output)
                .map_err(error("convert encoder samples"))?;
            output
        };
        self.fifo.push(&converted)?;
        self.drain_frames(false)
    }
    fn drain_frames(&mut self, last: bool) -> Result<Vec<Packet>> {
        let mut packets = Vec::new();
        let frame_size = self.encoder.frame_size() as usize;
        while self.fifo.samples() >= frame_size || (last && self.fifo.samples() > 0) {
            let count = self.fifo.samples().min(frame_size);
            let mut frame = self.fifo.pop(count)?;
            let pts = self
                .next_pts
                .ok_or(MediaError::Invalid("audio timestamp state"))?;
            frame.set_pts(Some(pts));
            self.next_pts = Some(
                pts.checked_add(count as i64)
                    .ok_or(MediaError::TooLarge("audio timestamp"))?,
            );
            self.encoder
                .send_frame(&frame)
                .map_err(error("send buffered audio frame"))?;
            packets.extend(self.receive()?);
            if packets.len() > 128 {
                return Err(MediaError::TooLarge("encoded audio packets"));
            }
        }
        Ok(packets)
    }
    fn receive(&mut self) -> Result<Vec<Packet>> {
        let mut packets = Vec::new();
        loop {
            let mut inner = ff::Packet::empty();
            match self.encoder.receive_packet(&mut inner) {
                Ok(()) => packets.push(Packet {
                    inner,
                    time_base: self.time_base,
                }),
                Err(e) if drained(e) => break,
                Err(e) => return Err(error("receive encoded audio")(e)),
            }
        }
        Ok(packets)
    }
    pub fn flush(&mut self) -> Result<Vec<Packet>> {
        if self.finished {
            return Ok(Vec::new());
        }
        let mut packets = self.drain_frames(true)?;
        self.finished = true;
        self.encoder
            .send_eof()
            .map_err(error("flush audio encoder"))?;
        packets.extend(self.receive()?);
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_video_batch_refuses_only_when_its_byte_budget_is_exceeded() {
        let packet = ff::Packet::copy(&[0; 32]);
        let charge = encoded_video_batch_bytes(0, &packet).unwrap();
        for (retained, accepted) in [
            (0, true),
            (MAX_ENCODED_VIDEO_BATCH_BYTES - charge - 1, true),
            (MAX_ENCODED_VIDEO_BATCH_BYTES - charge, true),
            (MAX_ENCODED_VIDEO_BATCH_BYTES - charge + 1, false),
            (usize::MAX, false),
        ] {
            match encoded_video_batch_bytes(retained, &packet) {
                Ok(bytes) => {
                    assert!(accepted);
                    assert_eq!(bytes, retained + charge);
                }
                Err(error) => {
                    assert!(!accepted);
                    assert!(matches!(
                        error,
                        MediaError::TooLarge("encoded video batch bytes")
                    ));
                }
            }
        }
        // Even zero payloads consume budget for metadata and vector capacity.
        assert!(encoded_video_batch_bytes(0, &ff::Packet::empty()).unwrap() > 0);
        let mut bytes = 0;
        for _ in 0..128 {
            bytes = encoded_video_batch_bytes(bytes, &packet).unwrap();
        }
        assert_eq!(bytes, 128 * charge);
    }

    #[test]
    fn threaded_final_flush_retains_more_than_64_packets() {
        init().unwrap();
        let codec = ff::encoder::find_by_name("libx264");
        assert!(
            codec.is_some() || !cfg!(target_os = "linux"),
            "Linux requires libx264"
        );
        let Some(codec) = codec else {
            // Only the Linux GPL build includes this encoder.
            eprintln!("skipping threaded lookahead regression: libx264 absent on this platform");
            return;
        };
        for threads in [2, 16] {
            let rate = FrameRate::new(30, 1).unwrap();
            let time_base = TimeBase::new(1, 30).unwrap();
            let color = ColorInfo::default().resolved_sdr();
            let settings = VideoEncoderSettings::h264(128, 96, rate, color);
            let mut native = ff::codec::context::Context::new_with_codec(codec)
                .encoder()
                .video()
                .unwrap();
            native.set_width(128);
            native.set_height(96);
            native.set_format(ff::format::Pixel::NV12);
            native.set_time_base(time_base.rational());
            native.set_bit_rate(settings.bit_rate);
            native.set_max_b_frames(0);
            native.set_threading(ff::codec::threading::Config::count(threads));
            let mut options = ff::Dictionary::new();
            // A long, real lookahead makes the final burst deterministic even on
            // small hosts, without changing the production encoder's quality.
            options.set("rc-lookahead", "80");
            let mut encoder = VideoEncoder {
                encoder: native.open_with(options).unwrap(),
                time_base,
                settings,
                scaler: None,
                finished: false,
                name: "libx264 test".into(),
                last_pts: None,
            };
            let first =
                VideoFrame::from_rgba8(128, 96, time::Tick::ZERO, color, &vec![128; 128 * 96 * 4])
                    .unwrap();
            let mut emitted = Vec::new();
            for index in 0..128 {
                let frame = first.with_pts(time_base.ticks(index).unwrap()).unwrap();
                emitted.extend(encoder.encode(&frame).unwrap());
            }
            let delayed = encoder.flush().unwrap();
            assert!(delayed.len() > 64, "burst must exercise the old cap");
            println!(
                "{threads} threads final flush: {} delayed packets",
                delayed.len()
            );
            emitted.extend(delayed);
            assert_eq!(emitted.len(), 128);
            for (index, packet) in emitted.iter().enumerate() {
                assert_eq!(
                    packet.pts().unwrap(),
                    Some(time_base.ticks(index as i64).unwrap())
                );
            }
            assert!(encoder.flush().unwrap().is_empty());
        }
    }

    #[test]
    fn maximum_quality_never_selects_automatic_or_disabled_native_quality() {
        assert_eq!(explicit_quality_quantizer(100), 1);
        assert_eq!(explicit_quality_quantizer(85), 7);
        assert_eq!(explicit_quality_quantizer(0), 51);
        let mut previous = 51;
        for quality in 0..=100 {
            let level = explicit_quality_quantizer(quality);
            assert!((1..=51).contains(&level));
            assert!(level <= previous);
            previous = level;
        }
    }

    #[test]
    fn videotoolbox_quality_requires_ffmpeg_support_and_hardware_then_retries_bitrate() {
        assert_eq!(videotoolbox_quality_attempts(true, false), &[true, false]);
        assert_eq!(videotoolbox_quality_attempts(true, true), &[false]);
        assert_eq!(videotoolbox_quality_attempts(false, false), &[false]);
        assert_eq!(videotoolbox_quality_attempts(false, true), &[false]);
    }

    #[test]
    fn rejected_settings_keep_native_cause_instead_of_claiming_encoder_absence() {
        for software in [true, false] {
            for source in [ff::Error::External, ff::Error::InvalidData] {
                assert!(matches!(
                    video_open_error(Some(source), software, false),
                    MediaError::Ffmpeg { operation: "open video encoder", source: actual }
                        if actual == source
                ));
            }
        }
        assert!(matches!(
            video_open_error(None, true, false),
            MediaError::EncoderUnavailable { source: None }
        ));
        assert!(matches!(
            video_open_error(None, false, false),
            MediaError::Unsupported(_)
        ));
    }

    #[test]
    fn missing_webm_encoders_are_typed_capability_errors() {
        // The branch taken when no admitted VP9 encoder opens names the
        // capability instead of silently selecting the MP4 pair.
        assert!(matches!(
            video_open_error(None, false, true),
            MediaError::Unsupported("VP9 encoder unavailable in this FFmpeg build")
        ));
        // The audio registry lookup fails with the container's own codec name.
        assert!(matches!(
            audio_encoder("vibecut-no-such-encoder", true),
            Err(MediaError::Unsupported(message)) if message.contains("Opus")
        ));
        assert!(matches!(
            audio_encoder("vibecut-no-such-encoder", false),
            Err(MediaError::Unsupported(message)) if message.contains("AAC")
        ));
        // Capability admission is the registry itself, not a platform adapter.
        init().unwrap();
        let vp9 = ff::encoder::find_by_name("libvpx-vp9").is_some();
        let opus = ff::encoder::find_by_name("libopus").is_some();
        assert_eq!(
            export_container_available(crate::Container::WebM),
            vp9 && opus
        );
        assert_eq!(video_candidates(true), &[("libvpx-vp9", true)][..]);
        if !opus {
            assert!(matches!(
                audio_encoder("libopus", true),
                Err(MediaError::Unsupported(message)) if message.contains("Opus")
            ));
        }
        if !vp9 {
            assert!(matches!(
                video_open_error(None, false, true),
                MediaError::Unsupported(message) if message.contains("VP9")
            ));
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod acceleration_tests {
    use super::*;
    use crate::{Container, Muxer, VideoReader};
    use time::Tick;

    #[test]
    #[ignore = "requires a working VideoToolbox hardware decoder and platform software encoder"]
    fn explicit_software_encoder_converts_gpu_input_without_retaining_its_pool() {
        let Some(path) = crate::read::decode::recovery_tests::fixture() else {
            return;
        };
        let mut reader = VideoReader::open(path).unwrap();
        let first = reader.next_frame().unwrap().unwrap();
        assert!(first.is_hardware());
        let settings = VideoEncoderSettings::h264(
            first.width(),
            first.height(),
            FrameRate::new(24, 1).unwrap(),
            first.color(),
        );
        let mut encoder =
            VideoEncoder::with_preference(settings.clone(), &first, EncodePreference::Software)
                .unwrap();
        assert!(!ffi::compatible_hardware_frame(
            &encoder.encoder,
            &first.inner
        ));
        assert_eq!(encoder.settings.bit_rate, settings.bit_rate);
        assert_eq!(encoder.settings.quality, settings.quality);
        let packets = encoder.encode(&first).unwrap().len() + encoder.flush().unwrap().len();
        assert_eq!(packets, 1);
    }

    #[test]
    #[ignore = "requires a working VideoToolbox hardware decoder and encoder"]
    fn hardware_pool_survives_decoder_and_accepts_software_frames() {
        let Some(path) = crate::read::decode::recovery_tests::fixture() else {
            return;
        };
        let mut reader = VideoReader::open(&path).unwrap();
        let first = reader.next_frame().unwrap().unwrap();
        assert!(
            first.is_hardware(),
            "this macOS conformance test requires a working VideoToolbox decoder"
        );
        let retained = first.with_pts(Tick::ZERO).unwrap();
        drop(first);
        drop(reader);
        let settings = VideoEncoderSettings::h264(
            retained.width(),
            retained.height(),
            FrameRate::new(24, 1).unwrap(),
            retained.color(),
        );
        let mut encoder = VideoEncoder::new(settings, &retained).unwrap();
        assert!(ffi::compatible_hardware_frame(
            &encoder.encoder,
            &retained.inner
        ));
        let wide = VideoFrame::from_rgba8(
            retained.width() + 2,
            retained.height(),
            Tick::ZERO,
            retained.color(),
            &vec![0; (retained.width() as usize + 2) * retained.height() as usize * 4],
        )
        .unwrap();
        assert!(matches!(
            ffi::upload_for_encoder(&encoder.encoder, wide.inner),
            Err(MediaError::Ffmpeg {
                operation: "upload encoder GPU frame",
                ..
            })
        ));
        let output = path.with_file_name("accelerated-recovery-upload.mp4");
        let mut muxer = Muxer::create(&output, Container::Mp4).unwrap();
        muxer
            .add_parameters(&encoder.parameters(), encoder.time_base())
            .unwrap();
        for packet in encoder.encode(&retained).unwrap() {
            muxer.write_packet(0, &packet).unwrap();
        }
        assert!(
            encoder.scaler.is_none(),
            "shared frame must bypass CPU conversion"
        );
        drop(retained);
        let mut reader = VideoReader::open(&path).unwrap();
        reader.next_frame().unwrap().unwrap();
        let mut count = 1;
        while let Some(frame) = reader.next_frame().unwrap() {
            // The new reader owns a different hardware pool; format equality
            // must not send that foreign surface directly to the old encoder.
            let cpu;
            let input = if count == 1 {
                assert!(!ffi::compatible_hardware_frame(
                    &encoder.encoder,
                    &frame.inner
                ));
                &frame
            } else {
                cpu = frame.download().unwrap();
                &cpu
            };
            for packet in encoder.encode(input).unwrap() {
                muxer.write_packet(0, &packet).unwrap();
            }
            count += 1;
        }
        for packet in encoder.flush().unwrap() {
            muxer.write_packet(0, &packet).unwrap();
        }
        muxer.finish().unwrap();
        assert_eq!(count, 48);
        assert_eq!(testkit::oracle::decoded_frames(&output, "v:0"), 48);
        assert!(testkit::oracle::decodes(&output));
        let mut output = VideoReader::open(&output).unwrap();
        let mut decoded = 0;
        while let Some(frame) = output.next_frame().unwrap() {
            assert_eq!(
                frame.pts().unwrap().unwrap(),
                Tick::from_units(decoded, 24, Rounding::Nearest).unwrap()
            );
            assert_eq!(frame.bit_depth(), Some(8));
            assert_eq!(frame.color().resolved_sdr(), encoder.settings.color);
            decoded += 1;
        }
        assert_eq!(decoded, 48);
    }
}
