//! Device-free evaluated passes, shared by CPU and app GPU adapters.
use crate::{Error, ExecutionHooks, Image, Result, check};
use project::{EffectParams, Item, ParamValue};
use time::Tick;

/// One normalized, separable convolution. Shader names are pipeline data.
#[derive(Clone, Debug)]
pub struct EffectPass {
    pub entry_point: &'static str,
    pub horizontal: bool,
    pub weights: Vec<f32>,
}
/// Inputs share output geometry and the premultiplied linear working space.
/// Blur consumes the item only; future passes can consume mask coverage/below.
pub struct EffectInputs<'a> {
    pub coverage: Option<&'a Image>,
    pub below: &'a Image,
}
pub trait EffectCompositor: Send + Sync {
    /// Unsupported requests fall back to CPU at the renderer; other failures propagate.
    /// Implementations must reserve all transient bytes and check cancellation.
    fn apply(
        &self,
        item: &Image,
        passes: &[EffectPass],
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image>;
}
pub struct CpuEffects;

/// Sigma in output pixels, referenced to 1920x1080. Uniform scale uses the
/// smaller axis so aspect-ratio changes do not stretch the circular kernel.
pub fn blur_sigma(intensity: f64, width: u32, height: u32) -> Result<f32> {
    Image::bytes(width, height)?;
    if !intensity.is_finite() || !(0.0..=100.0).contains(&intensity) {
        return Err(Error::Invalid("blur intensity"));
    }
    Ok((intensity / 5.0 * (f64::from(width) / 1920.0).min(f64::from(height) / 1080.0)) as f32)
}
/// At most sigma 8/radius 24 per axis per iteration. Variances add, so n
/// iterations use sigma/sqrt(n). The image-area cap bounds n to 203.
pub fn blur_passes(intensity: f64, width: u32, height: u32) -> Result<Vec<EffectPass>> {
    let sigma = blur_sigma(intensity, width, height)?;
    if sigma <= 0.01 {
        return Ok(Vec::new());
    }
    let iterations = (sigma / 8.0).powi(2).ceil().max(1.0) as usize;
    if iterations > 256 {
        return Err(Error::Unsupported("blur iteration bound"));
    }
    let sigma = sigma / (iterations as f32).sqrt();
    let radius = (sigma * 3.0).ceil().clamp(1.0, 24.0) as i32;
    let mut weights: Vec<_> = (-radius..=radius)
        .map(|x| (-(x as f32).powi(2) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    for w in &mut weights {
        *w /= total;
    }
    Ok((0..iterations)
        .flat_map(|_| [true, false])
        .map(|horizontal| EffectPass {
            entry_point: "gaussian",
            horizontal,
            weights: weights.clone(),
        })
        .collect())
}
pub(crate) fn evaluate(
    item: &Item,
    local: Tick,
    width: u32,
    height: u32,
) -> Result<Vec<EffectPass>> {
    let mut passes = Vec::new();
    for effect in item.effects.iter().filter(|e| e.enabled) {
        let EffectParams::Blur { intensity } = effect.params;
        let value = match item.animations.value_at(&effect.intensity_path()?, local)? {
            Some(ParamValue::Scalar(value)) => value.clamp(0.0, 100.0),
            Some(_) => return Err(Error::Invalid("blur animation type")),
            None => intensity,
        };
        passes.extend(blur_passes(value, width, height)?);
    }
    Ok(passes)
}
impl EffectCompositor for CpuEffects {
    fn apply(
        &self,
        item: &Image,
        passes: &[EffectPass],
        _: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        check(hooks)?;
        let mut a = item.copy(hooks)?;
        let mut b = Image::new(item.width(), item.height(), [0.0; 4], hooks)?;
        for pass in passes {
            validate_pass(pass)?;
            let w = a.width as usize;
            let h = a.height as usize;
            let radius = pass.weights.len() as i64 / 2;
            crate::image::row_chunks(&mut b.pixels, w, hooks, |start, rows| {
                for (row, pixels) in rows.chunks_mut(w).enumerate() {
                    check(hooks)?;
                    let y = start / w + row;
                    for (x, output) in pixels.iter_mut().enumerate() {
                        let mut sum = [0.0; 4];
                        for (tap, weight) in pass.weights.iter().enumerate() {
                            let delta = tap as i64 - radius;
                            let sx = if pass.horizontal {
                                (x as i64 + delta).clamp(0, w as i64 - 1) as usize
                            } else {
                                x
                            };
                            let sy = if pass.horizontal {
                                y
                            } else {
                                (y as i64 + delta).clamp(0, h as i64 - 1) as usize
                            };
                            let source = a
                                .pixels
                                .get(sy * w + sx)
                                .ok_or(Error::Invalid("effect pixel"))?;
                            for (v, s) in sum.iter_mut().zip(source) {
                                *v += s * weight;
                            }
                        }
                        // Roundoff must not violate the working-image contract.
                        let alpha = sum.get(3).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                        for v in &mut sum {
                            *v = v.clamp(0.0, alpha);
                        }
                        *output = sum;
                    }
                }
                Ok(())
            })?;
            std::mem::swap(&mut a, &mut b);
        }
        Ok(a)
    }
}
pub fn validate_pass(pass: &EffectPass) -> Result<()> {
    if pass.entry_point != "gaussian"
        || pass.weights.is_empty()
        || pass.weights.len() > 49
        || pass.weights.len().is_multiple_of(2)
        || pass.weights.iter().any(|w| !w.is_finite() || *w < 0.0)
        || (pass.weights.iter().sum::<f32>() - 1.0).abs() > 1e-5
    {
        return Err(Error::Unsupported("effect pass"));
    }
    Ok(())
}
