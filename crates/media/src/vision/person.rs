//! Prepares the person-segmentation tensor from a decoded vision frame.

use crate::{MediaError, Result};

use super::frame::VisionFrame;

/// Side of the square segmentation plane.
const PERSON_TENSOR_SIDE: u32 = 192;

impl VisionFrame {
    /// CHW RGB 192x192, raw 0..=255: the whole frame stretched to the square
    /// plane with bilinear interpolation (the upstream PP-HumanSeg recipe). The
    /// aspect ratio is not kept, so the matte maps back by the same stretch.
    /// Only the 110,592-value output is allocated, whatever the frame size.
    pub fn person_tensor(&self) -> Result<Vec<f32>> {
        let side = PERSON_TENSOR_SIDE;
        let plane = (side * side) as usize;
        let mut values = vec![0.; 3 * plane];
        for channel in 0..3 {
            for y in 0..side {
                for x in 0..side {
                    let value = self.sample_resized(x, y, side, side, channel, false)?;
                    *values
                        .get_mut(channel * plane + (y * side + x) as usize)
                        .ok_or(MediaError::Invalid("person tensor index"))? = value;
                }
            }
        }
        Ok(values)
    }
}
