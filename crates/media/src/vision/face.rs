//! Prepares face crops and detector tensors from a decoded vision frame.

use crate::{MediaError, Result};
use time::Tick;

use super::frame::VisionFrame;
use super::limits::VisionPrepLimits;

/// Side length of the grayscale face crop returned by [`VisionFrame::face_crop`].
pub const FACE_CROP_SIZE: u32 = 112;
const FACE_CROP_RESIZE: u32 = 224;
const FACE_CROP_INSET: u32 = 56;
const FACE_CROP_EXPANSION: f64 = 0.40;
const FACE_PADDING: u8 = 110;

impl VisionFrame {
    /// SFace five-landmark similarity alignment to 112x112 CHW RGB, raw 0..255.
    /// Landmarks are source pixels, ordered eyes, nose, mouth corners as YuNet.
    /// Bilinear sampling pads outside with black. Missing, tiny, degenerate or
    /// out-of-frame landmarks return None; nonfinite coordinates are errors.
    pub fn identity_tensor(&self, points: [[f32; 2]; 5]) -> Result<Option<Vec<f32>>> {
        const TARGET: [[f64; 2]; 5] = [
            [38.2946, 51.6963],
            [73.5318, 51.5014],
            [56.0252, 71.7366],
            [41.5493, 92.3655],
            [70.7299, 92.2041],
        ];
        if points.iter().flatten().any(|v| !v.is_finite()) {
            return Err(MediaError::Invalid("face landmarks"));
        }
        if points
            .iter()
            .any(|[x, y]| *x < 0. || *y < 0. || *x >= self.width as f32 || *y >= self.height as f32)
        {
            return Ok(None);
        }
        let [[x0, y0], [x1, y1], ..] = points;
        if (x0 - x1).hypot(y0 - y1) < 8. {
            return Ok(None);
        }
        let mut source_mean = [0.; 2];
        let mut target_mean = [0.; 2];
        for (p, q) in points.iter().zip(TARGET) {
            for ((s, t), (v, w)) in source_mean
                .iter_mut()
                .zip(&mut target_mean)
                .zip(p.iter().zip(q))
            {
                *s += f64::from(*v) / 5.;
                *t += w / 5.;
            }
        }
        let [sx, sy] = source_mean;
        let [tx, ty] = target_mean;
        let (mut dot, mut cross, mut variance) = (0., 0., 0.);
        for ([x, y], [u, v]) in points.iter().zip(TARGET) {
            let (x, y) = (f64::from(*x) - sx, f64::from(*y) - sy);
            let (u, v) = (u - tx, v - ty);
            dot += x * u + y * v;
            cross += x * v - y * u;
            variance += x * x + y * y;
        }
        if variance <= 1e-8 {
            return Ok(None);
        }
        let (a, b) = (dot / variance, cross / variance);
        let det = a * a + b * b;
        if det <= 1e-12 || !det.is_finite() {
            return Ok(None);
        }
        let mut output = vec![0.; 3 * 112 * 112];
        for y in 0..112usize {
            for x in 0..112usize {
                let (u, v) = (x as f64 - tx, y as f64 - ty);
                let (px, py) = ((a * u + b * v) / det + sx, (-b * u + a * v) / det + sy);
                let (ix, iy) = (px.floor() as i64, py.floor() as i64);
                let (fx, fy) = (px - px.floor(), py - py.floor());
                for c in 0..3usize {
                    let mut value = 0.;
                    for (dx, wx) in [(0, 1. - fx), (1, fx)] {
                        for (dy, wy) in [(0, 1. - fy), (1, fy)] {
                            let (xx, yy) = (ix + dx, iy + dy);
                            if xx >= 0
                                && yy >= 0
                                && xx < i64::from(self.width)
                                && yy < i64::from(self.height)
                            {
                                let offset =
                                    (yy as usize * self.width as usize + xx as usize) * 4 + c;
                                value += f64::from(
                                    *self
                                        .pixels
                                        .get(offset)
                                        .ok_or(MediaError::Invalid("face pixel"))?,
                                ) * wx
                                    * wy;
                            }
                        }
                    }
                    *output
                        .get_mut(c * 112 * 112 + y * 112 + x)
                        .ok_or(MediaError::Invalid("face tensor"))? = value.round() as f32;
                }
            }
        }
        Ok(Some(output))
    }
    /// Crop, grayscale and resize a face region using the `p1-clips/12`
    /// reference recipe: expand by 0.40, pad outside pixels with gray 110,
    /// resize to 224x224 with linear interpolation and keep the centre 112.
    /// The returned pixels are raw 0-255 grayscale.
    pub fn face_crop(&self, face: FaceBox, limits: &VisionPrepLimits) -> Result<FaceCrop> {
        let limits = limits.validate()?;
        let geometry = FaceGeometry::new(face, limits.max_face_crop_pixels)?;
        let mut gray = vec![0u8; geometry.width * geometry.height];
        for row in 0..geometry.height {
            let source_y = geometry.top + row as i64;
            for column in 0..geometry.width {
                let source_x = geometry.left + column as i64;
                let value = if source_x < 0
                    || source_y < 0
                    || source_x >= i64::from(self.width)
                    || source_y >= i64::from(self.height)
                {
                    FACE_PADDING
                } else {
                    self.gray_at(source_x as u32, source_y as u32)?
                };
                if let Some(slot) = gray.get_mut(row * geometry.width + column) {
                    *slot = value;
                }
            }
        }
        let resized = resize_center(&gray, geometry.width, geometry.height)?;
        FaceCrop::new(self.source_pts, FACE_CROP_SIZE, FACE_CROP_SIZE, resized)
    }
}

/// Face box in source pixels. The box is expanded by the reference recipe, so
/// it may validly extend outside the decoded frame; missing pixels are padded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Raw 0-255 grayscale face crop for one source frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceCrop {
    source_pts: Tick,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl FaceCrop {
    fn new(source_pts: Tick, width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) != pixels.len() as u64
        {
            return Err(MediaError::Invalid("face crop geometry"));
        }
        Ok(Self {
            source_pts,
            width,
            height,
            pixels,
        })
    }
    pub fn source_pts(&self) -> Tick {
        self.source_pts
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

struct FaceGeometry {
    left: i64,
    top: i64,
    width: usize,
    height: usize,
}
impl FaceGeometry {
    fn new(face: FaceBox, max_pixels: usize) -> Result<Self> {
        if !face.x.is_finite()
            || !face.y.is_finite()
            || !face.width.is_finite()
            || !face.height.is_finite()
            || face.width <= 0.0
            || face.height <= 0.0
        {
            return Err(MediaError::Invalid("face box"));
        }
        let half = face.width.max(face.height) / 2.0;
        if half <= 0.0 || !half.is_finite() {
            return Err(MediaError::Invalid("face box"));
        }
        let center_x = face.x + face.width / 2.0;
        let center_y = face.y + face.height / 2.0;
        let border = trunc_f64(half * (1.0 + 2.0 * FACE_CROP_EXPANSION))?;
        let my = trunc_f64(center_y + border as f64)?;
        let mx = trunc_f64(center_x + border as f64)?;
        let top = trunc_f64(my as f64 - half)?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let bottom = trunc_f64(my as f64 + half * (1.0 + 2.0 * FACE_CROP_EXPANSION))?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let left = trunc_f64(mx as f64 - half * (1.0 + FACE_CROP_EXPANSION))?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let right = trunc_f64(mx as f64 + half * (1.0 + FACE_CROP_EXPANSION))?
            .checked_sub(border)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let height = bottom
            .checked_sub(top)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        let width = right
            .checked_sub(left)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        if width <= 0 || height <= 0 {
            return Err(MediaError::Invalid("face crop geometry"));
        }
        let width =
            usize::try_from(width).map_err(|_| MediaError::TooLarge("face crop geometry"))?;
        let height =
            usize::try_from(height).map_err(|_| MediaError::TooLarge("face crop geometry"))?;
        let pixels = width
            .checked_mul(height)
            .ok_or(MediaError::TooLarge("face crop geometry"))?;
        if pixels > max_pixels {
            return Err(MediaError::TooLarge("face crop pixels"));
        }
        Ok(Self {
            left,
            top,
            width,
            height,
        })
    }
}
fn trunc_f64(value: f64) -> Result<i64> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(MediaError::TooLarge("face crop coordinate"));
    }
    Ok(value.trunc() as i64)
}

#[derive(Clone, Copy)]
struct LinearCoefficient {
    index: usize,
    first: i64,
    second: i64,
}
fn linear_coefficients(
    source: usize,
    destination: u32,
    start: u32,
    count: u32,
) -> Result<Vec<LinearCoefficient>> {
    if source == 0 || destination == 0 {
        return Err(MediaError::Invalid("resize geometry"));
    }
    if start.checked_add(count).is_none_or(|end| end > destination) {
        return Err(MediaError::Invalid("resize region"));
    }
    let scale = source as f64 / destination as f64;
    let last = source - 1;
    let mut coefficients = Vec::with_capacity(count as usize);
    for value in start..start + count {
        let coordinate = (value as f64 + 0.5) * scale - 0.5;
        let mut index = coordinate.floor() as i64;
        let mut fraction = coordinate - index as f64;
        if index < 0 {
            index = 0;
            fraction = 0.0;
        }
        if index > last as i64 {
            index = last as i64;
            fraction = 0.0;
        }
        let first = ((1.0 - fraction) * 2048.0).round_ties_even() as i64;
        let second = (fraction * 2048.0).round_ties_even() as i64;
        coefficients.push(LinearCoefficient {
            index: index as usize,
            first,
            second,
        });
    }
    Ok(coefficients)
}
fn resize_center(crop: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if width == 0
        || height == 0
        || width
            .checked_mul(height)
            .is_none_or(|pixels| pixels != crop.len())
    {
        return Err(MediaError::Invalid("face crop geometry"));
    }
    let columns = FACE_CROP_RESIZE - FACE_CROP_INSET * 2;
    let x = linear_coefficients(width, FACE_CROP_RESIZE, FACE_CROP_INSET, columns)?;
    let y = linear_coefficients(height, FACE_CROP_RESIZE, FACE_CROP_INSET, columns)?;
    let row_length = usize::try_from(columns).map_err(|_| MediaError::TooLarge("resize width"))?;
    let mut horizontal = vec![0i64; height * row_length];
    for source_y in 0..height {
        let row_start = source_y
            .checked_mul(width)
            .ok_or(MediaError::TooLarge("resize row"))?;
        let row = crop
            .get(row_start..row_start + width)
            .ok_or(MediaError::Invalid("resize row"))?;
        for (destination_x, coefficient) in x.iter().enumerate() {
            let first = i64::from(
                *row.get(coefficient.index)
                    .ok_or(MediaError::Invalid("resize column"))?,
            );
            let second = i64::from(
                *row.get((coefficient.index + 1).min(width - 1))
                    .ok_or(MediaError::Invalid("resize column"))?,
            );
            let value = first * coefficient.first + second * coefficient.second;
            if let Some(slot) = horizontal.get_mut(source_y * row_length + destination_x) {
                *slot = value;
            }
        }
    }
    let mut output = vec![0u8; row_length * row_length];
    for (destination_y, coefficient) in y.iter().enumerate() {
        let first_row = coefficient.index * row_length;
        let second_row = (coefficient.index + 1).min(height - 1) * row_length;
        for destination_x in 0..row_length {
            let first = horizontal
                .get(first_row + destination_x)
                .copied()
                .ok_or(MediaError::Invalid("resize row"))?;
            let second = horizontal
                .get(second_row + destination_x)
                .copied()
                .ok_or(MediaError::Invalid("resize row"))?;
            let value = (first * coefficient.first + second * coefficient.second + (1 << 21)) >> 22;
            let clamped = value.clamp(0, 255) as u8;
            if let Some(slot) = output.get_mut(destination_y * row_length + destination_x) {
                *slot = clamped;
            }
        }
    }
    Ok(output)
}

/// Prepared YuNet tensor and the exact rounded resize geometry used for boxes.
pub struct FaceTensor {
    values: Vec<f32>,
    resized_width: u32,
    resized_height: u32,
}
impl FaceTensor {
    pub fn values(&self) -> &[f32] {
        &self.values
    }
    pub fn resized_width(&self) -> u32 {
        self.resized_width
    }
    pub fn resized_height(&self) -> u32 {
        self.resized_height
    }
}

impl VisionFrame {
    /// CHW BGR 640x384, bilinear resize preserving aspect ratio, zero padding
    /// at bottom/right. Box coordinates are in this tensor's pixel plane.
    pub fn face_tensor(&self) -> Result<FaceTensor> {
        let scale = (640.0 / f64::from(self.width)).min(384.0 / f64::from(self.height));
        let w = (f64::from(self.width) * scale).round_ties_even().max(1.) as u32;
        let h = (f64::from(self.height) * scale).round_ties_even().max(1.) as u32;
        let mut values = vec![0.; 3 * 640 * 384];
        for y in 0..h {
            for x in 0..w {
                for channel in 0..3 {
                    let value = self.sample_resized(x, y, w, h, 2 - channel, false)?;
                    *values
                        .get_mut(channel * 640 * 384 + y as usize * 640 + x as usize)
                        .ok_or(MediaError::Invalid("face tensor index"))? = value;
                }
            }
        }
        Ok(FaceTensor {
            values,
            resized_width: w,
            resized_height: h,
        })
    }
}
