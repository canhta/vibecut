//! Face identity embedding input and bounded output validation.
use super::{finite, sized};
use crate::Error;

/// Aligned 112x112 CHW RGB pixels in 0..=255. Media owns alignment;
/// the pinned ONNX graph owns its internal centering/scaling.
#[derive(Clone, Copy)]
pub struct IdentityFrame<'a>(&'a [f32]);
impl<'a> IdentityFrame<'a> {
    pub fn new(values: &'a [f32]) -> Result<Self, Error> {
        sized(values, 3 * 112 * 112)?;
        if values.iter().any(|v| !(0.0..=255.).contains(v)) {
            return Err(Error::VisionShape);
        }
        Ok(Self(values))
    }
    pub fn values(&self) -> &[f32] {
        self.0
    }
}

/// Normalized SFace descriptor. Debug redacts biometric data; descriptors are
/// transient, never serialized into shot-analysis artifacts.
#[derive(Clone)]
pub struct FaceEmbedding(Vec<f32>);
impl std::fmt::Debug for FaceEmbedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FaceEmbedding([redacted])")
    }
}
impl FaceEmbedding {
    pub fn new(values: &[f32]) -> Result<Self, Error> {
        if values.len() != 128 {
            return Err(Error::VisionShape);
        }
        finite(values)?;
        let norm = values
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        if !norm.is_finite() || norm <= 1e-12 {
            return Err(Error::VisionNonFinite);
        }
        Ok(Self(
            values
                .iter()
                .map(|v| (f64::from(*v) / norm) as f32)
                .collect(),
        ))
    }
    /// Cosine similarity in [-1,1], independent of descriptor magnitude.
    pub fn similarity(&self, other: &Self) -> f32 {
        self.0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .clamp(-1., 1.)
    }
}
