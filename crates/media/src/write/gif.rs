//! Encodes RGBA frames as bounded animated-GIF packets through FFmpeg's GIF encoder.

use super::encode::receive_video_packets;
use super::quantize::quantize;
use crate::{CodecParameters, MediaError, Packet, Result, TimeBase, error, ffi, init};
use ffmpeg_next as ff;
use time::{FrameRate, Rounding, Tick};

/// Longest edge a GIF output may have.
pub const GIF_MAX_EDGE: u32 = 4096;
/// Pixel ceiling of one GIF frame (1920x1080 fits).
pub const GIF_MAX_PIXELS: u64 = 1 << 21;
/// Palette bounds. Every palette table is 256 entries; this limits the colours used.
pub const GIF_MIN_COLORS: u16 = 2;
pub const GIF_MAX_COLORS: u16 = 256;
/// Highest frame rate: GIF delays are whole centiseconds and viewers raise
/// delays below 2 cs, so 50 fps (2 cs) is the fastest honest rate.
pub const GIF_MAX_FPS: i64 = 50;
/// Longest animation and most frames one GIF may hold.
pub const GIF_MAX_SECONDS: i64 = 60;
pub const GIF_MAX_FRAMES: u64 = 3000;

/// Why GIF settings or an animation were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GifLimit {
    #[error("GIF palette must hold 2..=256 colours")]
    Colors,
    #[error("GIF frames must be 2..=4096 pixels per edge and at most 2^21 pixels")]
    Dimensions,
    #[error("GIF frame rate must be at most 50 fps")]
    FrameRate,
    #[error("GIF animation must be at most 60 seconds and 3000 frames")]
    Duration,
}

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
pub enum GifDither {
    None,
    /// 4x4 ordered dither: deterministic and LZW-friendly.
    #[default]
    Bayer,
}

/// Portable GIF palette choices. The animation loops forever.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct GifOptions {
    /// Distinct colours per frame, 2..=256.
    pub max_colors: u16,
    pub dither: GifDither,
}
impl Default for GifOptions {
    fn default() -> Self {
        Self {
            max_colors: GIF_MAX_COLORS,
            dither: GifDither::default(),
        }
    }
}
impl GifOptions {
    pub fn validate(self) -> std::result::Result<(), GifLimit> {
        if (GIF_MIN_COLORS..=GIF_MAX_COLORS).contains(&self.max_colors) {
            Ok(())
        } else {
            Err(GifLimit::Colors)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GifEncoderSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub options: GifOptions,
}
impl GifEncoderSettings {
    /// Refuses every setting outside the GIF bounds before any encoder opens.
    pub fn validate(&self) -> std::result::Result<(), GifLimit> {
        self.options.validate()?;
        if !(2..=GIF_MAX_EDGE).contains(&self.width)
            || !(2..=GIF_MAX_EDGE).contains(&self.height)
            || u64::from(self.width) * u64::from(self.height) > GIF_MAX_PIXELS
        {
            return Err(GifLimit::Dimensions);
        }
        if self.frame_rate.num() <= 0
            || self.frame_rate.den() <= 0
            || self.frame_rate.num() > GIF_MAX_FPS.saturating_mul(self.frame_rate.den())
        {
            return Err(GifLimit::FrameRate);
        }
        Ok(())
    }
    /// Whether `frames` frames at this rate fit one GIF.
    pub fn admits_frames(&self, frames: u64) -> std::result::Result<(), GifLimit> {
        let seconds = i128::from(frames) * i128::from(self.frame_rate.den());
        if frames == 0
            || frames > GIF_MAX_FRAMES
            || seconds > i128::from(GIF_MAX_SECONDS) * i128::from(self.frame_rate.num())
        {
            return Err(GifLimit::Duration);
        }
        Ok(())
    }
}

/// Palette-indexed GIF encoder. Each frame gets its own palette; FFmpeg's `gif`
/// encoder writes it and the muxer turns packet times into centisecond delays.
pub struct GifEncoder {
    encoder: ff::encoder::Video,
    time_base: TimeBase,
    settings: GifEncoderSettings,
    frames: u64,
    finished: bool,
    last_pts: Option<i64>,
}
impl GifEncoder {
    pub fn new(settings: GifEncoderSettings) -> Result<Self> {
        init()?;
        settings.validate()?;
        let rate = settings.frame_rate;
        let time_base = TimeBase::new(
            i32::try_from(rate.den()).map_err(|_| MediaError::Invalid("frame rate"))?,
            i32::try_from(rate.num()).map_err(|_| MediaError::Invalid("frame rate"))?,
        )?;
        let codec = ff::encoder::find_by_name("gif").ok_or(MediaError::Unsupported(
            "GIF encoder unavailable in this FFmpeg build",
        ))?;
        let mut encoder = ff::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(error("create GIF encoder"))?;
        encoder.set_width(settings.width);
        encoder.set_height(settings.height);
        encoder.set_time_base(time_base.rational());
        encoder.set_frame_rate(Some((time_base.denominator(), time_base.numerator())));
        encoder.set_format(ff::format::Pixel::PAL8);
        // Palettes change per frame, so inter-frame offsetting and transparency
        // diffs (which compare indices) stay off: every frame is a full image.
        let mut options = ff::Dictionary::new();
        options.set("gifflags", "0");
        let encoder = encoder
            .open_with(options)
            .map_err(error("open GIF encoder"))?;
        Ok(Self {
            encoder,
            time_base,
            settings,
            frames: 0,
            finished: false,
            last_pts: None,
        })
    }
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }
    pub fn parameters(&self) -> CodecParameters {
        CodecParameters(ff::codec::Parameters::from(&self.encoder))
    }
    /// Encodes one straight RGBA8 frame (alpha ignored) on the constant-rate grid.
    pub fn encode_rgba8(&mut self, rgba: &[u8], pts: Tick) -> Result<Vec<Packet>> {
        if self.finished {
            return Err(MediaError::Invalid("GIF encoder finished"));
        }
        let (width, height) = (self.settings.width, self.settings.height);
        if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err(MediaError::Invalid("RGBA geometry"));
        }
        if self.frames >= GIF_MAX_FRAMES {
            return Err(GifLimit::Duration.into());
        }
        let units = self.time_base.units(pts, Rounding::Nearest)?;
        if (i128::from(pts.0) - i128::from(self.time_base.ticks(units)?.0)).abs() > 1
            || self
                .last_pts
                .is_some_and(|last| last.checked_add(1) != Some(units))
            || (self.last_pts.is_none() && units != 0)
        {
            return Err(MediaError::Invalid(
                "video frames must follow the requested constant-rate grid",
            ));
        }
        let indexed = quantize(
            rgba,
            width as usize,
            usize::from(self.settings.options.max_colors),
            self.settings.options.dither,
        );
        let mut frame = ffi::pal8_frame(width, height, &indexed.indices, &indexed.palette)?;
        frame.set_pts(Some(units));
        self.encoder
            .send_frame(&frame)
            .map_err(error("send GIF frame"))?;
        self.last_pts = Some(units);
        self.frames += 1;
        receive_video_packets(&mut self.encoder, self.time_base, self.finished)
    }
    pub fn flush(&mut self) -> Result<Vec<Packet>> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.encoder
            .send_eof()
            .map_err(error("flush GIF encoder"))?;
        receive_video_packets(&mut self.encoder, self.time_base, self.finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> GifEncoderSettings {
        GifEncoderSettings {
            width: 640,
            height: 360,
            frame_rate: FrameRate::FPS_30,
            options: GifOptions::default(),
        }
    }

    #[test]
    fn hostile_settings_are_refused_with_the_limit_they_break() {
        assert_eq!(settings().validate(), Ok(()));
        type Edit = fn(&mut GifEncoderSettings);
        let cases: [(Edit, GifLimit); 7] = [
            (|s| s.options.max_colors = 1, GifLimit::Colors),
            (|s| s.options.max_colors = 257, GifLimit::Colors),
            (|s| s.width = 1, GifLimit::Dimensions),
            (|s| s.height = 0, GifLimit::Dimensions),
            (|s| s.width = 4098, GifLimit::Dimensions),
            (
                |s| {
                    s.width = 4096;
                    s.height = 4096
                },
                GifLimit::Dimensions,
            ),
            (|s| s.frame_rate = FrameRate::FPS_60, GifLimit::FrameRate),
        ];
        for (edit, limit) in cases {
            let mut hostile = settings();
            edit(&mut hostile);
            assert_eq!(hostile.validate(), Err(limit));
            assert!(matches!(
                GifEncoder::new(hostile),
                Err(MediaError::Gif(found)) if found == limit
            ));
        }
        let encoder = settings();
        assert_eq!(encoder.admits_frames(1800), Ok(()));
        assert_eq!(encoder.admits_frames(0), Err(GifLimit::Duration));
        assert_eq!(encoder.admits_frames(1801), Err(GifLimit::Duration));
        let fast = GifEncoderSettings {
            frame_rate: FrameRate::FPS_50,
            ..settings()
        };
        assert_eq!(fast.admits_frames(3000), Ok(()));
        assert_eq!(fast.admits_frames(3001), Err(GifLimit::Duration));
    }
}
