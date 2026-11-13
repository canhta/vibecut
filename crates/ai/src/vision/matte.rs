//! Person matte contract: validated segmentation input and probability planes.
use super::sized;
use crate::Error;

/// Side of the square segmentation plane the graph reads and writes.
pub const MATTE_SIDE: usize = 192;
/// Identity of the input preparation and output reading; it enters the fingerprint with the
/// weights. Feather and frame-size application belong to render.
pub const PERSON_RECIPE: &str =
    "person-v1:pphumanseg-rgb192-stretch-bilinear-norm.5:class1-probability";
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
    /// The plane quantized to one byte per cell, row-major, 255 for certainly a person: the form
    /// the engine stores and render applies.
    pub fn quantized(&self) -> Vec<u8> {
        self.0
            .iter()
            .map(|p| (p * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    }
}
