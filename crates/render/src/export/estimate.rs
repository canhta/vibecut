//! The size an export is expected to write, from the settings the encoder will receive.
use super::ExportSettings;

/// The audio encoder's bit rate in bits per second; `execution` hands this to the encoder.
pub(super) const AUDIO_BIT_RATE: usize = 192_000;
/// Encoded bits per pixel per frame at quality 85, measured on a 1080p60 export
/// (47.25 MB over 9.58 s, 39.46 Mb/s, less the audio track).
const BITS_PER_PIXEL_AT_85: f64 = 0.3156;

impl ExportSettings {
    /// The video bit rate the output is expected to reach, in bits per second.
    ///
    /// The encoders get the quality scalar and `bit_rate` as a target only (VideoToolbox in
    /// quality mode ignores it), so the rate follows pixels per second and the quality scalar: the measured anchor at 85,
    /// doubling per 10 points above it and halving per 15 below (the spacing of the
    /// Lower, Recommended and Higher presets). The requested `bit_rate` is a floor.
    pub fn estimated_video_bit_rate(&self) -> f64 {
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

    /// Expected output bytes for `seconds` of output: video (none for audio-only output)
    /// plus the audio track when it is included.
    pub fn estimated_bytes(&self, seconds: f64) -> f64 {
        let video = if self.audio_only {
            0.0
        } else {
            self.estimated_video_bit_rate()
        };
        let audio = if self.include_audio || self.audio_only {
            AUDIO_BIT_RATE as f64
        } else {
            0.0
        };
        seconds.max(0.0) * (video + audio) / 8.0
    }
}
