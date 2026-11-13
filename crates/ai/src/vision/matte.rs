//! Person matte contract: validated segmentation tensors and frame-size feathered alpha.
use super::{finite, sized};
use crate::{Error, ExecutionHooks, Stage, contracts::checkpoint};

/// Side of the square segmentation plane the graph reads and writes.
pub const MATTE_SIDE: usize = 192;
/// Largest edge feather, a project-style 0..=100 slider.
pub const MAX_FEATHER: f32 = 100.0;
/// Largest output edge and area one `alpha` call admits (8K fits, one byte per pixel).
pub const MAX_MATTE_SIDE: u32 = 16_384;
pub const MAX_MATTE_PIXELS: u64 = 1 << 26;
/// Identity of the pre/post recipe; it enters the fingerprint with the weights.
pub const PERSON_RECIPE: &str = "person-v1:pphumanseg-rgb192-stretch-bilinear-norm.5:class1-probability:feather-box2x-r8:bilinear-u8";
/// Box radius, in matte cells, at `MAX_FEATHER`. A new decision, not measured parity.
const BLUR_RADIUS_AT_MAX: f32 = 8.0;
const BLUR_PASSES: usize = 2;
/// Softmax rounding slack admitted before a value is refused as out of range.
const PROBABILITY_SLACK: f32 = 1e-3;

/// CHW RGB 192x192, unscaled pixel values 0..=255, the whole frame stretched to
/// the plane (media owns the resize). The adapter owns the centring and scaling.
#[derive(Clone, Copy)]
pub struct PersonFrame<'a>(&'a [f32]);
impl<'a> PersonFrame<'a> {
    pub fn values(&self) -> &[f32] {
        self.0
    }
    pub fn new(values: &'a [f32]) -> Result<Self, Error> {
        sized(values, 3 * MATTE_SIDE * MATTE_SIDE)?;
        if values.iter().any(|v| !(0.0..=255.0).contains(v)) {
            return Err(Error::VisionShape);
        }
        Ok(Self(values))
    }
}

/// Per-cell person probability on the 192x192 plane, finite and within 0..=1.
/// Debug is redacted: the plane is an image of a person.
#[derive(Clone, PartialEq)]
pub struct PersonMatte(Vec<f32>);
impl std::fmt::Debug for PersonMatte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PersonMatte([redacted])")
    }
}
impl PersonMatte {
    pub fn new(mut probabilities: Vec<f32>) -> Result<Self, Error> {
        sized(&probabilities, MATTE_SIDE * MATTE_SIDE)?;
        if probabilities
            .iter()
            .any(|v| !(-PROBABILITY_SLACK..=1.0 + PROBABILITY_SLACK).contains(v))
        {
            return Err(Error::VisionShape);
        }
        for value in &mut probabilities {
            *value = value.clamp(0.0, 1.0);
        }
        Ok(Self(probabilities))
    }
    pub fn probabilities(&self) -> &[f32] {
        &self.0
    }
    /// Straight 8-bit alpha, `width * height` row-major bytes. `feather` 0 keeps
    /// the model's own probabilities (a bilinear resample only, exact at 192x192);
    /// above 0 the plane first takes two box passes of radius
    /// `ceil(feather / 100 * 8)` cells with clamped edges. Cancellation is
    /// checked every 16 output rows; the only allocation is the result.
    pub fn alpha(
        &self,
        width: u32,
        height: u32,
        feather: f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<u8>, Error> {
        checkpoint(hooks, Stage::Vision)?;
        finite(&[feather])?;
        if width == 0
            || height == 0
            || width > MAX_MATTE_SIDE
            || height > MAX_MATTE_SIDE
            || u64::from(width) * u64::from(height) > MAX_MATTE_PIXELS
            || !(0.0..=MAX_FEATHER).contains(&feather)
        {
            return Err(Error::VisionShape);
        }
        let (width, height) = (width as usize, height as usize);
        let radius = (feather / MAX_FEATHER * BLUR_RADIUS_AT_MAX).ceil() as usize;
        let mut field = self.0.clone();
        if radius > 0 {
            for _ in 0..BLUR_PASSES {
                field = blur(&field, radius, false);
                field = blur(&field, radius, true);
            }
        }
        let columns = taps(width);
        let rows = taps(height);
        let at = |row: usize, column: usize| {
            field
                .get(row * MATTE_SIDE + column)
                .copied()
                .unwrap_or_default()
        };
        let mut out = Vec::new();
        out.try_reserve_exact(width * height)
            .map_err(|_| Error::Payload)?;
        for (y, &(top, bottom, fy)) in rows.iter().enumerate() {
            if y % 16 == 0 {
                checkpoint(hooks, Stage::Vision)?;
            }
            for &(left, right, fx) in &columns {
                let upper = at(top, left) * (1.0 - fx) + at(top, right) * fx;
                let lower = at(bottom, left) * (1.0 - fx) + at(bottom, right) * fx;
                let value = upper * (1.0 - fy) + lower * fy;
                out.push((value * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        checkpoint(hooks, Stage::Vision)?;
        Ok(out)
    }
}

fn cell(value: isize) -> usize {
    value.clamp(0, MATTE_SIDE as isize - 1) as usize
}
/// Half-pixel bilinear taps (low cell, high cell, high weight) for one output axis.
fn taps(output: usize) -> Vec<(usize, usize, f32)> {
    (0..output)
        .map(|i| {
            let source = (i as f64 + 0.5) * MATTE_SIDE as f64 / output as f64 - 0.5;
            let floor = source.floor();
            (
                cell(floor as isize),
                cell(floor as isize + 1),
                (source - floor) as f32,
            )
        })
        .collect()
}
/// One clamped-edge box pass along one axis.
fn blur(source: &[f32], radius: usize, vertical: bool) -> Vec<f32> {
    let width = (2 * radius + 1) as f32;
    let mut out = Vec::with_capacity(source.len());
    for y in 0..MATTE_SIDE {
        for x in 0..MATTE_SIDE {
            let mut sum = 0.0;
            for k in 0..=2 * radius {
                let offset = k as isize - radius as isize;
                let (px, py) = if vertical {
                    (x, cell(y as isize + offset))
                } else {
                    (cell(x as isize + offset), y)
                };
                sum += source
                    .get(py * MATTE_SIDE + px)
                    .copied()
                    .unwrap_or_default();
            }
            out.push(sum / width);
        }
    }
    out
}
