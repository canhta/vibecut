//! The size an export is expected to write, from the settings the encoder will receive.
use super::ExportSettings;
use media::{Container, RateControl};

/// The audio encoder's bit rate in bits per second; `execution` hands this to the encoder.
pub(super) const AUDIO_BIT_RATE: usize = 192_000;
/// Encoded bits per pixel per frame at quality 85 in quality mode, measured on a 1080p60 export
/// (47.25 MB over 9.58 s, 39.46 Mb/s, less the audio track).
const BITS_PER_PIXEL_AT_85: f64 = 0.3156;

impl ExportSettings {
    /// The video bit rate the output is expected to reach, in bits per second, for the
    /// rate control the encoder selection will open for these settings.
    pub fn estimated_video_bit_rate(&self) -> f64 {
        self.estimated_video_bit_rate_in(media::expected_rate_control(
            self.encoder,
            matches!(self.container, Container::WebM),
        ))
    }

    /// The video bit rate under `mode`. A [`RateControl::Target`] encoder holds `bit_rate`,
    /// so that is the estimate whatever the picture. A [`RateControl::Quality`] encoder
    /// ignores it: the rate follows pixels per second and the quality scalar, the measured
    /// anchor at 85, doubling per 10 points above it and halving per 15 below (the spacing
    /// of the Lower, Recommended and Higher presets), with `bit_rate` as a floor.
    pub fn estimated_video_bit_rate_in(&self, mode: RateControl) -> f64 {
        match mode {
            RateControl::Target => self.bit_rate as f64,
            RateControl::Quality => {
                let quality = f64::from(self.quality.min(100)) - 85.0;
                let octaves = if quality < 0.0 {
                    quality / 15.0
                } else {
                    quality / 10.0
                };
                let pixels = f64::from(self.render.width) * f64::from(self.render.height);
                let model = pixels * self.fps.as_f64() * BITS_PER_PIXEL_AT_85 * octaves.exp2();
                model.max(self.bit_rate as f64)
            }
        }
    }

    /// The audio track's bit rate in bits per second at `sample_rate`. WAV is exact
    /// (16-bit PCM). FLAC is lossless and its size depends on the signal, so it is
    /// estimated at its PCM size, an upper bound. The lossy codecs follow the target
    /// the encoder receives.
    pub fn estimated_audio_bit_rate(&self, sample_rate: u32) -> f64 {
        match self.container {
            Container::Wav | Container::Flac => {
                f64::from(sample_rate) * f64::from(self.channels) * 16.0
            }
            _ => AUDIO_BIT_RATE as f64,
        }
    }

    /// Expected output bytes for `seconds` of output: video (none for audio-only output)
    /// plus the audio track when it is included, at the project's `sample_rate`.
    pub fn estimated_bytes(&self, seconds: f64, sample_rate: u32) -> f64 {
        // Reconcile point with the per-encoder-mode estimate (518): GIF has its own model in
        // `gif.rs`, so it stays a branch on the container, ahead of any video-mode split.
        if self.container == media::Container::Gif {
            return super::gif::estimated_bytes(self, seconds);
        }
        let video = if self.audio_only {
            0.0
        } else {
            self.estimated_video_bit_rate()
        };
        let audio = if self.include_audio || self.audio_only {
            self.estimated_audio_bit_rate(sample_rate)
        } else {
            0.0
        };
        seconds.max(0.0) * (video + audio) / 8.0
    }
}
