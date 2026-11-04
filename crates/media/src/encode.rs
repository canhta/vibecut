use crate::{
    AudioFrame, CodecParameters, ColorInfo, MediaError, Packet, Result, TimeBase, VideoFrame,
    drained, error, ffi, init,
};
use ffmpeg_next as ff;
use time::{FrameRate, Rounding};

#[derive(Clone, Copy, Debug, Default)]
pub enum EncodePreference {
    #[default]
    Hardware,
    Software,
}

#[derive(Clone, Debug)]
pub struct VideoEncoderSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub bit_rate: usize,
    pub quality: u8,
    pub color: ColorInfo,
    pub webm: bool,
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
            color,
            webm: false,
        }
    }
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
        Self::with_preference(settings, first, EncodePreference::Hardware)
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
        let candidates: &[(&str, bool)] = if settings.webm {
            &[("libvpx-vp9", true)]
        } else if cfg!(target_os = "macos") {
            &[("h264_videotoolbox", true)]
        } else {
            &[
                ("h264_nvenc", true),
                ("h264_qsv", true),
                ("h264_amf", true),
                ("h264_mf", true),
                ("h264_mf", false),
            ]
        };
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
            encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER);
            encoder.set_format(
                if !settings.webm
                    && first.is_hardware()
                    && first.width() == settings.width
                    && first.height() == settings.height
                    && codec
                        .video()
                        .ok()
                        .and_then(|v| v.formats())
                        .is_some_and(|mut formats| {
                            formats.any(|format| format == first.inner.format())
                        })
                {
                    ff::format::Pixel::VIDEOTOOLBOX
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
            match name {
                "h264_videotoolbox" => {
                    options.set("allow_sw", "1");
                    if matches!(preference, EncodePreference::Software) {
                        options.set("require_sw", "1");
                    }
                    encoder.set_quality(usize::from(settings.quality) * 118);
                    encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER | ff::codec::Flags::QSCALE);
                }
                "h264_nvenc" => {
                    options.set("preset", "p6");
                    options.set("rc", "vbr");
                    options.set(
                        "cq",
                        &((100 - u32::from(settings.quality)) * 51 / 100).to_string(),
                    );
                }
                "h264_qsv" => {
                    options.set("preset", "slow");
                    options.set(
                        "global_quality",
                        &((100 - u32::from(settings.quality)) * 51 / 100).to_string(),
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
        if !settings.webm && matches!(preference, EncodePreference::Software) {
            return Err(MediaError::EncoderUnavailable { source: last });
        }
        Err(last.map_or(
            MediaError::Unsupported("no available encoder for target quality"),
            error("open video encoder"),
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
        let mut output = if frame.inner.format() == self.encoder.format()
            && frame.width() == self.settings.width
            && frame.height() == self.settings.height
        {
            ffi::reference_video(&frame.inner)?
        } else {
            let cpu = frame.download()?;
            if self.scaler.is_none() {
                self.scaler = Some(ffi::Scaler::new()?);
            }
            self.scaler
                .as_mut()
                .ok_or(MediaError::Invalid("scaler state"))?
                .scale(
                    &cpu.inner,
                    self.settings.width,
                    self.settings.height,
                    self.encoder.format(),
                    &self.encoder,
                )?
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
        let mut packets = Vec::new();
        loop {
            let mut inner = ff::Packet::empty();
            match self.encoder.receive_packet(&mut inner) {
                Ok(()) => {
                    if packets.len() >= 64 {
                        return Err(MediaError::TooLarge("encoded packets"));
                    }
                    if inner.duration() <= 0 {
                        inner.set_duration(1);
                    }
                    packets.push(Packet {
                        inner,
                        time_base: self.time_base,
                    });
                }
                Err(e) if drained(e) => break,
                Err(e) => return Err(error("receive encoded video")(e)),
            }
        }
        Ok(packets)
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
        let codec = ff::encoder::find_by_name(name)
            .ok_or(MediaError::Unsupported("audio encoder unavailable"))?;
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
