//! Bounds one vision preparation request.

use crate::{MediaError, Result};
use time::{TICKS_PER_SECOND, Tick};

/// Hard bounds for one preparation request: by default ten minutes of output,
/// eleven minutes of decoded audio including preroll, one hundred thousand
/// video frames and 60,001 MFCC frames (ten minutes at a 10 ms hop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisionPrepLimits {
    /// Longest shot or audio range admitted by one call.
    pub max_shot_duration: Tick,
    /// Maximum video frames decoded by one call, including seek preroll and
    /// replayed frames; emitted frames are a subset of this budget.
    pub max_frames: usize,
    /// Maximum packed output frame area, checked before RGBA conversion.
    pub max_frame_pixels: u64,
    /// Maximum decoded duration per analysis range or first-preparation slice,
    /// including anchor search and preroll.
    pub max_audio_decode_duration: Tick,
    /// Maximum input reads per analysis range or first-preparation slice,
    /// including probing and seeking.
    pub max_audio_bytes: u64,
    /// Maximum number of MFCC frames emitted by one call.
    pub max_mfcc_frames: usize,
    /// Maximum pixel area of one face-crop rectangle before resizing.
    pub max_face_crop_pixels: usize,
}
impl Default for VisionPrepLimits {
    fn default() -> Self {
        Self {
            max_shot_duration: Tick(600 * TICKS_PER_SECOND),
            max_frames: 100_000,
            max_frame_pixels: 1 << 26,
            max_audio_decode_duration: Tick(660 * TICKS_PER_SECOND),
            max_audio_bytes: 256 << 20,
            max_mfcc_frames: 60_001,
            max_face_crop_pixels: 1 << 24,
        }
    }
}
impl VisionPrepLimits {
    pub(super) fn validate(self) -> Result<Self> {
        if self.max_shot_duration <= Tick::ZERO
            || self.max_frames == 0
            || self.max_frames > 1_000_000
            || self.max_frame_pixels == 0
            || self.max_frame_pixels > 1 << 26
            || self.max_audio_decode_duration <= Tick::ZERO
            || self.max_audio_decode_duration > Tick(86_400 * TICKS_PER_SECOND)
            || self.max_audio_bytes == 0
            || self.max_audio_bytes > 4 << 30
            || self.max_mfcc_frames == 0
            || self.max_mfcc_frames > 1_000_000
            || self.max_face_crop_pixels == 0
            || self.max_face_crop_pixels > 1 << 26
        {
            return Err(MediaError::Invalid("vision preparation limits"));
        }
        Ok(self)
    }
}
