//! Prepares the CLIP scene tensor from a decoded vision frame.

use crate::{MediaError, Result};

use super::frame::VisionFrame;

impl VisionFrame {
    /// CHW RGB 224x224: resize shorter edge to 224 with cubic interpolation
    /// (a=-0.75), centre crop, then the pinned CLIP mean/std normalization.
    /// Only the centre crop is allocated, even for extreme aspect ratios.
    pub fn scene_tensor(&self) -> Result<Vec<f32>> {
        let scale = 224.0 / f64::from(self.width.min(self.height));
        let scaled_width = (f64::from(self.width) * scale).round_ties_even();
        let scaled_height = (f64::from(self.height) * scale).round_ties_even();
        if scaled_width > f64::from(u32::MAX) || scaled_height > f64::from(u32::MAX) {
            return Err(MediaError::TooLarge("scene resize dimensions"));
        }
        let w = scaled_width as u32;
        let h = scaled_height as u32;
        let mut values = vec![0.; 3 * 224 * 224];
        for (channel, (mean, std)) in [
            (0.48145466, 0.26862954),
            (0.4578275, 0.261_302_6),
            (0.40821073, 0.275_777_1),
        ]
        .into_iter()
        .enumerate()
        {
            for y in 0..224 {
                for x in 0..224 {
                    let pixel = self.sample_resized(
                        x + (w - 224) / 2,
                        y + (h - 224) / 2,
                        w,
                        h,
                        channel,
                        true,
                    )?;
                    *values
                        .get_mut(channel * 224 * 224 + y as usize * 224 + x as usize)
                        .ok_or(MediaError::Invalid("scene tensor index"))? =
                        (pixel / 255. - mean) / std;
                }
            }
        }
        Ok(values)
    }
}
