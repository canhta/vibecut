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
    /// Composite a masked/effected working item over its track backdrop.
    /// Unsupported must leave `below` unchanged so the renderer can use CPU.
    fn composite(
        &self,
        below: &mut Image,
        item: &Image,
        opacity: f32,
        mode: project::BlendMode,
        hooks: &dyn ExecutionHooks,
    ) -> Result<()> {
        super::blend::composite(below, item, opacity, mode, hooks)
    }
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
/// Linear-light mid grey that contrast pivots around.
pub const GRADE_PIVOT: f32 = 0.18;
/// Grade pass values: `[gain, contrast exponent, saturation, hue radians]`.
/// Controls are the persisted -50..=50 sliders: brightness is +-1 stop of
/// linear gain, contrast a power around [`GRADE_PIVOT`] (0.5..1.5), saturation
/// a Rec. 709 luma blend (0..2) and hue a rotation about the grey axis (+-180
/// degrees). All zero is identity and yields no pass. These ranges are new,
/// not measured parity (ticket 68).
pub fn grade_passes(
    brightness: f64,
    contrast: f64,
    saturation: f64,
    hue: f64,
) -> Result<Vec<EffectPass>> {
    let range = project::GRADE_MIN..=project::GRADE_MAX;
    if [brightness, contrast, saturation, hue]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("grade control"));
    }
    if brightness == 0.0 && contrast == 0.0 && saturation == 0.0 && hue == 0.0 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass {
        entry_point: "grade",
        horizontal: false,
        weights: vec![
            (brightness / 50.0).exp2() as f32,
            (1.0 + contrast / 100.0) as f32,
            (1.0 + saturation / 50.0) as f32,
            (hue / 50.0 * std::f64::consts::PI) as f32,
        ],
    }])
}
/// One premultiplied linear pixel through a grade pass; the reference the
/// GPU shader mirrors. Colour stays within `0..=alpha`.
pub fn grade_pixel(pixel: [f32; 4], weights: &[f32]) -> [f32; 4] {
    let [gain, exponent, sat, hue] = [0, 1, 2, 3].map(|i| weights.get(i).copied().unwrap_or(0.0));
    let alpha = pixel[3];
    if alpha <= 0.0 {
        return pixel;
    }
    let mut c = [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha];
    for v in &mut c {
        *v = (*v * gain).max(0.0);
        *v = GRADE_PIVOT * (*v / GRADE_PIVOT).powf(exponent);
    }
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for v in &mut c {
        *v = luma + (*v - luma) * sat;
    }
    let (s, cos) = hue.sin_cos();
    let a = cos + (1.0 - cos) / 3.0;
    let off = (1.0 - cos) / 3.0;
    let k = s * 0.577_350_3;
    let [r, g, b] = c;
    let out = [
        a * r + (off - k) * g + (off + k) * b,
        (off + k) * r + a * g + (off - k) * b,
        (off - k) * r + (off + k) * g + a * b,
    ];
    let [r, g, b] = out.map(|v| v.clamp(0.0, 1.0) * alpha);
    [r, g, b, alpha]
}
pub(crate) fn evaluate(
    item: &Item,
    local: Tick,
    width: u32,
    height: u32,
) -> Result<Vec<EffectPass>> {
    let mut passes = Vec::new();
    for effect in item.effects.iter().filter(|e| e.enabled) {
        match effect.params {
            EffectParams::Blur { intensity } => {
                let value = match item.animations.value_at(&effect.intensity_path()?, local)? {
                    Some(ParamValue::Scalar(value)) => value.clamp(0.0, 100.0),
                    Some(_) => return Err(Error::Invalid("blur animation type")),
                    None => intensity,
                };
                passes.extend(blur_passes(value, width, height)?);
            }
            EffectParams::ColorGrade {
                brightness,
                contrast,
                saturation,
                hue,
            } => passes.extend(grade_passes(brightness, contrast, saturation, hue)?),
        }
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
            if pass.entry_point == "grade" {
                crate::image::row_chunks(&mut b.pixels, a.width as usize, hooks, |start, rows| {
                    check(hooks)?;
                    let source = a
                        .pixels
                        .get(start..start + rows.len())
                        .ok_or(Error::Invalid("effect pixel"))?;
                    for (out, px) in rows.iter_mut().zip(source) {
                        *out = grade_pixel(*px, &pass.weights);
                    }
                    Ok(())
                })?;
                std::mem::swap(&mut a, &mut b);
                continue;
            }
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
    if pass.entry_point == "grade" {
        return if pass.weights.len() == 4
            && pass.weights.iter().all(|w| w.is_finite())
            && pass.weights.get(1).is_some_and(|k| *k > 0.0)
        {
            Ok(())
        } else {
            Err(Error::Unsupported("effect pass"))
        };
    }
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
