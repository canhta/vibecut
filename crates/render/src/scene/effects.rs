//! Device-free evaluated passes, shared by CPU and app GPU adapters.
use crate::{Error, ExecutionHooks, Image, Result, check};
use project::{Animations, EffectInstance, EffectParamKind, EffectValue, Item, Param, ParamValue};
use std::{borrow::Cow, collections::BTreeMap};
use time::{Rounding, Tick};

mod denoise;
mod image_ops;
mod layers;
pub mod lut;
mod spatial;
mod whole_image;
pub use denoise::reduce_noise_passes;
pub(crate) use layers::adjust_backdrop;
pub use layers::{
    FrameLayer, MAX_FRAME_LAYERS, composite_layers, reference_layers, validate_layers,
};

/// One shader uniform: what an effect hands a single-pass pixel op.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uniform {
    Float(f32),
    /// Straight sRGB-encoded colour, `0..=1` per channel.
    Color([f32; 3]),
    /// A frame fraction on each axis.
    Point([f32; 2]),
}
impl Uniform {
    fn extend(self, out: &mut Vec<f32>) {
        match self {
            Self::Float(v) => out.push(v),
            Self::Color(c) => out.extend(c),
            Self::Point(p) => out.extend(p),
        }
    }
}
/// One device-independent pass. Shader names are pipeline data.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectPass {
    /// CPU reference pass with immutable, admitted imported samples.
    Lut3d {
        table: std::sync::Arc<lut::Cube3d>,
        strength: f32,
    },
    /// One normalized, separable convolution.
    Convolve {
        entry_point: &'static str,
        horizontal: bool,
        weights: Vec<f32>,
    },
    /// One pixel-local op reading flat uniforms.
    Pixel {
        entry_point: &'static str,
        uniforms: Vec<Uniform>,
    },
    /// One pixel from its coordinate, flat uniforms and two readable images.
    Sample {
        entry_point: &'static str,
        uniforms: Vec<Uniform>,
    },
}
impl EffectPass {
    pub const fn entry_point(&self) -> &'static str {
        match self {
            Self::Lut3d { .. } => "lut_3d",
            Self::Convolve { entry_point, .. }
            | Self::Pixel { entry_point, .. }
            | Self::Sample { entry_point, .. } => entry_point,
        }
    }
    pub const fn horizontal(&self) -> bool {
        matches!(
            self,
            Self::Convolve {
                horizontal: true,
                ..
            }
        )
    }
    /// The flat floats a shader reads: convolution weights or flattened uniforms.
    pub fn values(&self) -> Cow<'_, [f32]> {
        match self {
            Self::Lut3d { .. } => Cow::Borrowed(&[]),
            Self::Convolve { weights, .. } => Cow::Borrowed(weights),
            Self::Pixel { uniforms, .. } | Self::Sample { uniforms, .. } => {
                let mut flat = Vec::with_capacity(uniforms.len() * 3);
                for uniform in uniforms {
                    uniform.extend(&mut flat);
                }
                Cow::Owned(flat)
            }
        }
    }
}
/// Inputs share output geometry and the premultiplied linear working space.
/// `apply` receives the evolving current image separately; `input` never changes
/// during an instance, including CPU/device run boundaries and Unsupported retries.
pub struct EffectInputs<'a> {
    /// Immutable image at the start of this effect instance, shared by all runs.
    pub input: &'a Image,
    pub coverage: Option<&'a Image>,
    pub below: &'a Image,
}
impl EffectInputs<'_> {
    /// Refuse mismatched image geometry before allocating or doing pixel work.
    pub fn validate(&self, current: &Image) -> Result<()> {
        for image in [Some(self.input), Some(self.below), self.coverage]
            .into_iter()
            .flatten()
        {
            if (image.width(), image.height()) != (current.width(), current.height()) {
                return Err(Error::Invalid("effect input dimensions"));
            }
        }
        Ok(())
    }
}
pub trait EffectCompositor: Send + Sync {
    /// Acquire optional per-frame resources before decoding or output allocation.
    /// Unsupported selects CPU for the entire frame; None retains this adapter.
    /// Forward the exact borrowed hooks object to every scoped call.
    fn frame<'a>(
        &'a self,
        hooks: &'a dyn ExecutionHooks,
    ) -> Result<Option<Box<dyn EffectCompositor + 'a>>>;
    /// Release completed idle capacity before CPU work; pending work stays charged.
    fn release_idle(&self) -> Result<()>;
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
    /// Optional whole-frame composition of already prepared layers, bottom to top
    /// over a transparent canvas. Callers go through [`composite_layers`], which
    /// validates first and recomputes on the CPU reference after `Unsupported`.
    /// Implementations decide admission before mutation or device work and never
    /// publish a partial result; the default declines.
    fn composite_layers(
        &self,
        _width: u32,
        _height: u32,
        _layers: &[FrameLayer<'_>],
        _hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        Err(Error::Unsupported("frame layers"))
    }
    /// Whether this compositor has a pipeline for `entry_point`. The executor runs
    /// unsupported passes on the CPU and keeps their supported neighbours here.
    fn supports(&self, entry_point: &str) -> bool;
    /// Unsupported requests fall back to CPU at `apply_effect`; other failures propagate.
    /// `item` is the current run input; `inputs.input` is the immutable instance
    /// input. Implementations validate dimensions before work, reserve all transient
    /// bytes and check cancellation.
    fn apply(
        &self,
        item: &Image,
        passes: &[EffectPass],
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image>;
}
/// Apply one effect instance's pass list: consecutive passes the adapter supports go
/// to it as one run, the rest to the CPU reference. Each run starts from the previous
/// run's output; the immutable instance input is retained separately for all runs
/// and retries, with its working copy reserved through the hooks before allocation.
/// Images use premultiplied linear working pixels and must share positive dimensions
/// within Image's area bound. Geometry and every pass are admitted before copying.
/// Only adapter Unsupported retries that run on CPU; invalid input, budget,
/// cancellation and other failures propagate. Below is borrowed and unchanged.
/// An empty list returns an exact accounted copy. All execution checks cancellation
/// through the supplied hooks; a successive call captures its own instance input.
pub fn apply_effect(
    image: Image,
    passes: &[EffectPass],
    below: &Image,
    adapter: &dyn EffectCompositor,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    check(hooks)?;
    EffectInputs {
        input: &image,
        coverage: None,
        below,
    }
    .validate(&image)?;
    for pass in passes {
        validate_pass(pass, image.width(), image.height())?;
    }
    let input = image;
    let mut image = input.copy(hooks)?;
    let cpu = CpuEffects;
    let mut index = 0;
    while index < passes.len() {
        let first = passes.get(index).ok_or(Error::Invalid("effect run"))?;
        let supported = adapter.supports(first.entry_point());
        let mut end = index + 1;
        while end < passes.len() {
            let next = passes.get(end).ok_or(Error::Invalid("effect run"))?;
            if adapter.supports(next.entry_point()) != supported {
                break;
            }
            end += 1;
        }
        let run = passes.get(index..end).ok_or(Error::Invalid("effect run"))?;
        let inputs = || EffectInputs {
            input: &input,
            coverage: None,
            below,
        };
        image = if supported {
            match adapter.apply(&image, run, inputs(), hooks) {
                Err(Error::Unsupported(_)) => {
                    adapter.release_idle()?;
                    cpu.apply(&image, run, inputs(), hooks)?
                }
                result => result?,
            }
        } else {
            adapter.release_idle()?;
            cpu.apply(&image, run, inputs(), hooks)?
        };
        index = end;
    }
    Ok(image)
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
    sigma_passes(blur_sigma(intensity, width, height)?, width, height)
}
/// The same kernel for a gaussian of `sigma` output pixels, which whole-image
/// ops ask for directly instead of an intensity.
pub(super) fn sigma_passes(sigma: f32, width: u32, height: u32) -> Result<Vec<EffectPass>> {
    Image::bytes(width, height)?;
    if !sigma.is_finite() || sigma < 0.0 {
        return Err(Error::Invalid("blur sigma"));
    }
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
        .map(|horizontal| EffectPass::Convolve {
            entry_point: GAUSSIAN,
            horizontal,
            weights: weights.clone(),
        })
        .collect())
}
/// Blurs a clip background frame at a preset intensity through the shared image
/// blur. Sigma follows the blur effect's scale: intensity / 5 at 1920x1080 on
/// the smaller axis.
pub(super) fn clip_background_blur(
    image: &Image,
    intensity: u32,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    /// Sigma one kernel iteration covers (`sigma_passes` splits larger sigmas into
    /// (sigma / 8)^2 iterations, which grows too fast to run per frame).
    const SINGLE_ITERATION_SIGMA: f32 = 8.0;
    let scale = (f64::from(image.width) / 1920.0).min(f64::from(image.height) / 1080.0);
    let mut sigma = (f64::from(intensity) / 5.0 * scale) as f32;
    // A clip background is smooth, so large sigmas blur a half-resolution copy
    // at half sigma per level, then scale back up. Each level halves both axes.
    let mut levels: Option<Image> = None;
    while sigma > SINGLE_ITERATION_SIGMA {
        let (width, height) = levels
            .as_ref()
            .map_or((image.width, image.height), |low| (low.width, low.height));
        if width.min(height) <= 2 {
            break;
        }
        let next = match levels.as_ref() {
            Some(low) => low.half(hooks)?,
            None => image.half(hooks)?,
        };
        levels = Some(next);
        sigma /= 2.0;
    }
    match levels {
        None => image_ops::blur(image, sigma, hooks),
        Some(low) => {
            let blurred = image_ops::blur(&low, sigma, hooks)?;
            image_ops::upsample(&blurred, image.width, image.height, hooks)
        }
    }
}
/// Linear-light mid grey that contrast pivots around.
const GRADE_PIVOT: f32 = 0.18;
/// BT.709 luma coefficients, also the key's luma for the shadow stage.
const KEY_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// BT.709 chroma scaling: 2*(1-Kb) and 2*(1-Kr) from the same coefficients.
const KEY_CB_SCALE: f32 = 1.8556;
const KEY_CR_SCALE: f32 = 1.5748;
/// Chroma-distance tolerance at intensity 0 and its span at intensity 100: the
/// implementation reference maps its 0..100 tolerance to `0.08 + 0.5*t`.
/// Intensity 0 emits no pass, matching the observed reference.
const KEY_TOLERANCE_MIN: f32 = 0.08;
const KEY_TOLERANCE_SPAN: f32 = 0.5;
/// Matte gain spans 1..3 across intensity, like the reference's
/// `1 + 2*transparency`.
const KEY_GAIN: f32 = 2.0;
/// The reference's shadow stage doubles the luma gap before adding it to the matte.
const KEY_SHADOW_GAIN: f32 = 2.0;
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
    Ok(vec![EffectPass::Pixel {
        entry_point: "grade",
        uniforms: [
            (brightness / 50.0).exp2() as f32,
            (1.0 + contrast / 100.0) as f32,
            (1.0 + saturation / 50.0) as f32,
            (hue / 50.0 * std::f64::consts::PI) as f32,
        ]
        .map(Uniform::Float)
        .to_vec(),
    }])
}
/// One premultiplied linear pixel through a grade pass; the reference the
/// GPU shader mirrors. Colour stays within `0..=alpha`.
fn grade_pixel(pixel: [f32; 4], weights: &[f32]) -> [f32; 4] {
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
/// Render the lift coefficients of `[r, g, b]` percentages.
pub fn lift_passes(r: f64, g: f64, b: f64) -> Result<Vec<EffectPass>> {
    let range = project::LIFT_MIN..=project::LIFT_MAX;
    if [r, g, b]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("lift control"));
    }
    if r == 0.0 && g == 0.0 && b == 0.0 {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "lift",
        [r, g, b]
            .map(|v| Uniform::Float((v / 100.0) as f32))
            .to_vec(),
    )])
}
/// Render the gamma coefficients of `[r, g, b]` percentages.
pub fn gamma_passes(r: f64, g: f64, b: f64) -> Result<Vec<EffectPass>> {
    let range = project::GAMMA_MIN..=project::GAMMA_MAX;
    if [r, g, b]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("gamma control"));
    }
    if r == 100.0 && g == 100.0 && b == 100.0 {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "tone_gamma",
        [r, g, b]
            .map(|v| Uniform::Float((v / 100.0) as f32))
            .to_vec(),
    )])
}
/// Render the gain coefficients of `[r, g, b]` percentages.
pub fn gain_passes(r: f64, g: f64, b: f64) -> Result<Vec<EffectPass>> {
    let range = project::GAIN_MIN..=project::GAIN_MAX;
    if [r, g, b]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("gain control"));
    }
    if r == 100.0 && g == 100.0 && b == 100.0 {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "gain",
        [r, g, b]
            .map(|v| Uniform::Float((v / 100.0) as f32))
            .to_vec(),
    )])
}
/// One pixel-local lift stage: `straight + L * (1 - straight)`, clamped.
fn lift_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let lift = [uniform(values, 0), uniform(values, 1), uniform(values, 2)];
    let mapped = zip3(c, lift, |c, l| (c + l * (1.0 - c)).clamp(0.0, 1.0));
    premultiply(mapped, alpha)
}
/// One pixel-local gamma stage: `straight^(1/gamma)`, clamped.
fn tone_gamma_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let gamma = [uniform(values, 0), uniform(values, 1), uniform(values, 2)];
    let mapped = zip3(c, gamma, |c, gamma| {
        c.max(0.0).powf(1.0 / gamma.max(0.25)).clamp(0.0, 1.0)
    });
    premultiply(mapped, alpha)
}
/// One pixel-local gain stage: `straight * gain`, clamped.
fn gain_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let gain = [uniform(values, 0), uniform(values, 1), uniform(values, 2)];
    let mapped = zip3(c, gain, |c, g| (c * g).clamp(0.0, 1.0));
    premultiply(mapped, alpha)
}
/// Fade mixes linear light toward this mid grey; negative values mix away.
const FADE_PIVOT: f32 = 0.18;
/// One shared basic stage after the primary grade controls:
/// `[temperature, highlight, shadow, shine, fade]`, each normalized to -1..=1.
/// Temperature is a red/blue gain pair, highlight and shadow add to bright and
/// dark linear values, shine scales highlights and fade mixes toward mid grey.
fn tone_passes(
    temperature: f64,
    highlight: f64,
    shadow: f64,
    shine: f64,
    fade: f64,
) -> Result<Vec<EffectPass>> {
    let range = project::GRADE_MIN..=project::GRADE_MAX;
    if [temperature, highlight, shadow, shine, fade]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("grade tone control"));
    }
    if [temperature, highlight, shadow, shine, fade]
        .iter()
        .all(|v| *v == 0.0)
    {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "tone",
        [temperature, highlight, shadow, shine, fade]
            .map(|v| Uniform::Float((v / 50.0) as f32))
            .to_vec(),
    )])
}
/// One pixel-local tone stage; values are normalized -1..=1.
fn tone_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let temperature = uniform(values, 0);
    let highlight = uniform(values, 1);
    let shadow = uniform(values, 2);
    let shine = uniform(values, 3);
    let fade = uniform(values, 4);
    let (c, alpha) = straight(pixel);
    let mut c = [
        (c[0] * (temperature / 4.0).exp2()).max(0.0),
        c[1],
        (c[2] * (-temperature / 4.0).exp2()).max(0.0),
    ];
    let y = luma(c).clamp(0.0, 1.0);
    let dark = (1.0 - y) * (1.0 - y);
    let bright = y * y;
    for v in &mut c {
        *v += highlight * 0.25 * bright;
        *v += shadow * 0.25 * dark;
        *v *= 1.0 + shine * 0.25 * bright;
        let mix = fade * 0.5;
        *v = *v * (1.0 - mix) + FADE_PIVOT * mix;
    }
    premultiply(c, alpha)
}
/// The flat HSL band layout: eight bands of `[hue, saturation, lightness]`,
/// each normalized to -1..=1.
const HSL_BAND_UNIFORMS: usize = project::HSL_BANDS.len() * project::HSL_CONTROLS.len();
/// HSL band centres in turns, in the catalogue's band order.
const HSL_CENTERS: [f32; 8] = [
    0.0,
    30.0 / 360.0,
    60.0 / 360.0,
    120.0 / 360.0,
    180.0 / 360.0,
    240.0 / 360.0,
    300.0 / 360.0,
    330.0 / 360.0,
];
/// One shared pixel stage applying per-band hue, saturation and lightness.
fn hsl_bands_passes(values: &[f64; HSL_BAND_UNIFORMS]) -> Result<Vec<EffectPass>> {
    let range = project::GRADE_MIN..=project::GRADE_MAX;
    if values.iter().any(|v| !v.is_finite() || !range.contains(v)) {
        return Err(Error::Invalid("HSL band control"));
    }
    if values.iter().all(|v| *v == 0.0) {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "hsl_bands",
        values
            .iter()
            .map(|v| Uniform::Float((v / 50.0) as f32))
            .collect(),
    )])
}
/// One pixel-local HSL band stage. Encoded sRGB bands overlap with a
/// triangular 45-degree kernel and normalize, so no hue falls between bands.
fn hsl_bands_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    if values.len() != HSL_BAND_UNIFORMS {
        return pixel;
    }
    let (c, alpha) = straight(pixel);
    let encoded = encoded_color(c);
    let [h, s, l] = hsl(encoded);
    let mut nearest = (0usize, f32::MAX);
    let mut second = (0usize, f32::MAX);
    for (index, center) in HSL_CENTERS.iter().copied().enumerate() {
        let signed = (h - center + 0.5).rem_euclid(1.0) - 0.5;
        let distance = signed.abs();
        if distance < nearest.1 {
            second = nearest;
            nearest = (index, distance);
        } else if distance < second.1 {
            second = (index, distance);
        }
    }
    let mut weights = [0.0f32; 8];
    let total = nearest.1 + second.1;
    if total > 0.0 {
        if let Some(weight) = weights.get_mut(nearest.0) {
            *weight = second.1 / total;
        }
        if let Some(weight) = weights.get_mut(second.0) {
            *weight = nearest.1 / total;
        }
    } else if let Some(weight) = weights.get_mut(nearest.0) {
        *weight = 1.0;
    }
    let mut hue = 0.0;
    let mut saturation = 0.0;
    let mut lightness = 0.0;
    for (index, weight) in weights.iter().enumerate() {
        let base = index * project::HSL_CONTROLS.len();
        hue += weight * uniform(values, base);
        saturation += weight * uniform(values, base + 1);
        lightness += weight * uniform(values, base + 2);
    }
    if hue == 0.0 && saturation == 0.0 && lightness == 0.0 {
        return pixel;
    }
    let mapped = hsl_color(
        (h + hue / 12.0).rem_euclid(1.0),
        (s * (1.0 + saturation * 0.5)).clamp(0.0, 1.0),
        (l + lightness * 0.25).clamp(0.0, 1.0),
    );
    premultiply(decoded_color(mapped), alpha)
}
/// The flat curve layout: point count, then 32 ordered `x, y` pairs zero-padded.
const CURVE_UNIFORMS: usize = 1 + 2 * project::CURVE_MAX_POINTS;
/// One monotone piecewise-linear lookup; malformed data is the identity.
fn curve_map(values: &[f32], x: f32) -> f32 {
    let Some(&raw) = values.first() else {
        return x;
    };
    let count = raw as usize;
    if !(project::CURVE_MIN_POINTS..=project::CURVE_MAX_POINTS).contains(&count)
        || values.len() < 1 + count * 2
    {
        return x;
    }
    let point = |index: usize| {
        [
            uniform(values, 1 + index * 2),
            uniform(values, 2 + index * 2),
        ]
    };
    let mut low = point(0);
    for index in 1..count {
        let high = point(index);
        if x <= high[0] {
            let span = high[0] - low[0];
            if span <= 0.0 {
                return high[1];
            }
            let t = (x - low[0]) / span;
            return (low[1] + (high[1] - low[1]) * t).clamp(0.0, 1.0);
        }
        low = high;
    }
    low[1]
}
fn curve_channel_pixel(pixel: [f32; 4], values: &[f32], channel: usize) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let mut mapped = c;
    if let Some(value) = mapped.get_mut(channel) {
        *value = curve_map(values, *value);
    }
    premultiply(mapped, alpha)
}
fn curve_red_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    curve_channel_pixel(pixel, values, 0)
}
fn curve_green_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    curve_channel_pixel(pixel, values, 1)
}
fn curve_blue_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    curve_channel_pixel(pixel, values, 2)
}
/// One pixel-local luma stage: preserves chroma by scaling straight RGB by
/// `C(Y)/Y`; black stays black because every curve fixes `(0,0)`.
fn curve_luma_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let y = luma(c).clamp(0.0, 1.0);
    let mapped = curve_map(values, y);
    let rgb = if y > 0.0 {
        c.map(|v| (v * mapped / y).clamp(0.0, 1.0))
    } else {
        [0.0; 3]
    };
    premultiply(rgb, alpha)
}
/// The 65 flat floats a curve pass carries.
fn curve_uniforms(curve: &project::ToneCurve) -> Vec<Uniform> {
    let mut uniforms = Vec::with_capacity(CURVE_UNIFORMS);
    let points = curve.points();
    uniforms.push(Uniform::Float(points.len() as f32));
    for index in 0..project::CURVE_MAX_POINTS {
        let [x, y] = points.get(index).copied().unwrap_or([0.0, 0.0]);
        uniforms.push(Uniform::Float(x as f32));
        uniforms.push(Uniform::Float(y as f32));
    }
    uniforms
}
/// Chroma-key pass values: `[key r, key g, key b, intensity, shadow]` with the
/// key sRGB-encoded and both controls in 0..=1. Intensity scales the
/// chroma-distance tolerance and 0 emits no pass; shadow restores pixels darker
/// than the key. Stages follow the implementation reference's matte and spill
/// stages; the observed UI exposes no spill control, so removal is inherent.
/// `key_color` is finite straight encoded sRGB RGBA in 0..=1; alpha is unused.
/// Invalid colour/control values refuse before allocating a pass list.
pub fn chroma_key_passes(
    key_color: [f64; 4],
    intensity: f64,
    shadow: f64,
) -> Result<Vec<EffectPass>> {
    let range = project::CHROMA_KEY_MIN..=project::CHROMA_KEY_MAX;
    if key_color
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::Invalid("chroma key colour"));
    }
    if [intensity, shadow]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("chroma key control"));
    }
    if intensity == 0.0 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Pixel {
        entry_point: "chroma_key",
        uniforms: vec![
            color_uniform(key_color),
            Uniform::Float((intensity / 100.0) as f32),
            Uniform::Float((shadow / 100.0) as f32),
        ],
    }])
}
fn key_chroma(color: [f32; 3]) -> (f32, f32, f32) {
    let y = KEY_LUMA[0] * color[0] + KEY_LUMA[1] * color[1] + KEY_LUMA[2] * color[2];
    (
        y,
        (color[2] - y) / KEY_CB_SCALE,
        (color[0] - y) / KEY_CR_SCALE,
    )
}
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// One premultiplied linear pixel through a chroma-key pass; the reference the
/// GPU shader mirrors. Alpha is multiplied by the matte, and the key's dominant
/// channel loses any excess over the other two (spill suppression). Colour stays
/// within `0..=alpha`.
pub fn chroma_key_pixel(pixel: [f32; 4], weights: &[f32]) -> [f32; 4] {
    let alpha = pixel[3];
    if alpha <= 0.0 {
        return pixel;
    }
    let key = [0, 1, 2].map(|i| weights.get(i).copied().unwrap_or(0.0));
    let intensity = weights.get(3).copied().unwrap_or(0.0).clamp(0.0, 1.0);
    let shadow = weights.get(4).copied().unwrap_or(0.0).clamp(0.0, 1.0);
    let mut c = [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha];
    for v in &mut c {
        *v = crate::image::oetf(v.clamp(0.0, 1.0), 13);
    }
    let (y, cb, cr) = key_chroma(c);
    let (key_y, key_cb, key_cr) = key_chroma(key);
    let magnitude_squared = (key_cb * key_cb + key_cr * key_cr).max(1e-6);
    let projection = (cb * key_cb + cr * key_cr) / magnitude_squared;
    let distance = ((cb - projection * key_cb).powi(2) + (cr - projection * key_cr).powi(2)).sqrt()
        / magnitude_squared.sqrt();
    let tolerance = KEY_TOLERANCE_MIN + KEY_TOLERANCE_SPAN * intensity;
    let keyness = projection.clamp(0.0, 1.5) * (1.0 - smoothstep(0.0, tolerance, distance));
    let protected = (key_y - y).max(0.0) * shadow * KEY_SHADOW_GAIN * keyness.min(1.0);
    let matte = (1.0 - keyness * (1.0 + KEY_GAIN * intensity) + protected).clamp(0.0, 1.0);
    let dominant = if key[1] >= key[0] && key[1] >= key[2] {
        1
    } else if key[2] >= key[0] {
        2
    } else {
        0
    };
    let limit = match dominant {
        0 => c[1].max(c[2]),
        1 => c[0].max(c[2]),
        _ => c[0].max(c[1]),
    };
    if let Some(channel) = c.get_mut(dominant) {
        *channel = channel.min(limit);
    }
    let alpha = matte * alpha;
    let mut out = c.map(|v| crate::image::eotf(v.clamp(0.0, 1.0), 13) * alpha);
    for v in &mut out {
        *v = v.clamp(0.0, alpha);
    }
    [out[0], out[1], out[2], alpha]
}
fn three_channel(values: &[f32], range: std::ops::RangeInclusive<f32>) -> bool {
    values.len() == 3 && values.iter().all(|v| v.is_finite() && range.contains(v))
}
fn lift_uniforms(values: &[f32]) -> bool {
    three_channel(values, -1.0..=1.0)
}
fn gamma_uniforms(values: &[f32]) -> bool {
    three_channel(values, 0.25..=4.0)
}
fn gain_uniforms(values: &[f32]) -> bool {
    three_channel(values, 0.0..=4.0)
}
/// The tone stage carries five normalized controls in -1..=1.
fn tone_uniforms(values: &[f32]) -> bool {
    values.len() == 5 && values.iter().all(|v| (-1.0..=1.0).contains(v))
}
/// The HSL stage carries eight bands of three normalized controls.
fn hsl_band_uniforms(values: &[f32]) -> bool {
    values.len() == HSL_BAND_UNIFORMS && values.iter().all(|v| (-1.0..=1.0).contains(v))
}
/// The flat curve shape: integer count 2..=32, ordered grid x, monotone y and
/// zero padding. Validation never sorts or repairs a proposal.
fn curve_uniforms_valid(values: &[f32]) -> bool {
    if values.len() != CURVE_UNIFORMS {
        return false;
    }
    let Some(&raw) = values.first() else {
        return false;
    };
    if !raw.is_finite() || raw.fract() != 0.0 || !(2.0..=32.0).contains(&raw) {
        return false;
    }
    let count = raw as usize;
    let mut previous = [0.0f32; 2];
    let (pairs, _) = values.get(1..).unwrap_or_default().as_chunks::<2>();
    for (index, pair) in pairs.iter().enumerate() {
        let [x, y] = *pair;
        if index >= count {
            if x != 0.0 || y != 0.0 {
                return false;
            }
            continue;
        }
        let ticks = x * project::CURVE_GRID as f32;
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(&x)
            || !(0.0..=1.0).contains(&y)
            || ticks.round() != ticks
        {
            return false;
        }
        if index == 0 {
            if x != 0.0 || y != 0.0 {
                return false;
            }
        } else if x - previous[0] < 1.0 / project::CURVE_GRID as f32 || y < previous[1] {
            return false;
        }
        previous = [x, y];
    }
    previous == [1.0, 1.0]
}
/// Entry name of the shared separable gaussian, CPU and shader.
const GAUSSIAN: &str = "gaussian";
/// Entry name of the shader-only blend composite.
pub const BLEND_ENTRY: &str = "blend";
/// The one image, besides the current one, that a shader entry binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadImage {
    /// The image the effect instance started with.
    Instance,
    /// The backdrop below the item.
    Below,
}
/// Whether `effects.wgsl` has a compute entry named like the op, and what it reads.
#[derive(Clone, Copy)]
enum Wgsl {
    /// CPU reference only; the run splitter leaves the pass to the CPU.
    Absent,
    /// A shader entry reading the current image alone.
    Entry,
    /// A shader entry that also reads one more image.
    Reads(ReadImage),
}
/// How the CPU runs an op. The shader side is [`Wgsl`].
#[derive(Clone, Copy)]
enum Kernel {
    /// Normalised separable convolution; the executor owns the loop.
    Convolve,
    /// Shader-only composite of an item over its backdrop; no pass reaches the CPU.
    Composite,
    Pixel(PixelOp),
    Sample(SampleOp),
}
/// The one description of a pass kind: its id, which is also its WGSL entry name,
/// how the CPU runs it and what a device adapter must provide. Pass admission, the
/// CPU executor and the GPU adapter's pipeline list and image bindings all read
/// this table; a new kind is one entry.
struct EffectOp {
    entry_point: &'static str,
    wgsl: Wgsl,
    kernel: Kernel,
}
/// A pixel-local op: flat uniform count, uniform check and the CPU reference the
/// shader mirrors.
#[derive(Clone, Copy)]
struct PixelOp {
    len: usize,
    valid: fn(&[f32]) -> bool,
    apply: fn([f32; 4], &[f32]) -> [f32; 4],
}

/// Straight linear colour and alpha of a premultiplied working pixel; transparent
/// black for a fully transparent one so every op keeps its contract.
fn straight(pixel: [f32; 4]) -> ([f32; 3], f32) {
    let alpha = pixel[3];
    if alpha <= 0.0 {
        return ([0.0; 3], 0.0);
    }
    (
        [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha].map(|v| v.clamp(0.0, 1.0)),
        alpha,
    )
}
fn encoded_color(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|v| crate::image::oetf(v.clamp(0.0, 1.0), 13))
}
fn decoded_color(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|v| crate::image::eotf(v.clamp(0.0, 1.0), 13))
}
fn premultiply(rgb: [f32; 3], alpha: f32) -> [f32; 4] {
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * alpha).clamp(0.0, alpha));
    [r, g, b, alpha]
}
fn uniform(values: &[f32], index: usize) -> f32 {
    values.get(index).copied().unwrap_or(0.0)
}
/// Elementwise `f` over the three channels of two colour arrays.
pub(super) fn zip3<T: Copy, U: Copy, V>(a: [T; 3], b: [U; 3], f: impl Fn(T, U) -> V) -> [V; 3] {
    let [a0, a1, a2] = a;
    let [b0, b1, b2] = b;
    [f(a0, b0), f(a1, b1), f(a2, b2)]
}
/// The RGB channels of one working pixel.
pub(super) fn rgb(pixel: [f32; 4]) -> [f32; 3] {
    let [r, g, b, _] = pixel;
    [r, g, b]
}
/// Replaces a working pixel's RGB channels, keeping its alpha.
pub(super) fn with_rgb(pixel: [f32; 4], channels: [f32; 3]) -> [f32; 4] {
    let [_, _, _, alpha] = pixel;
    let [r, g, b] = channels;
    [r, g, b, alpha]
}
fn uniform_color(values: &[f32], start: usize) -> [f32; 3] {
    [0, 1, 2].map(|k| uniform(values, start + k))
}
fn luma(rgb: [f32; 3]) -> f32 {
    KEY_LUMA[0] * rgb[0] + KEY_LUMA[1] * rgb[1] + KEY_LUMA[2] * rgb[2]
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
fn hsl(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let high = r.max(g).max(b);
    let low = r.min(g).min(b);
    let l = (high + low) / 2.0;
    if (high - low).abs() < 1e-7 {
        return [0.0, 0.0, l];
    }
    let d = high - low;
    let s = if l > 0.5 {
        d / (2.0 - high - low)
    } else {
        d / (high + low)
    };
    let h = if high == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if high == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}
fn hsl_color(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}
/// Rec. 709 luma of the straight linear colour replaces every channel.
fn black_white_pixel(pixel: [f32; 4], _: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    premultiply([luma(c); 3], alpha)
}
/// Encoded-space channel inversion mixed back by `blend`; the Alpha channel
/// inverts coverage instead.
fn invert_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let channel = uniform(values, 0).round().clamp(0.0, 4.0) as u32;
    let blend = uniform(values, 1).clamp(0.0, 1.0);
    if channel == 4 {
        // Coverage inverts and blends back by `blend`; the picture keeps its
        // straight colour, re-premultiplied by the new alpha.
        let (c, alpha) = straight(pixel);
        let out = (1.0 - alpha) * (1.0 - blend) + alpha * blend;
        return premultiply(c, out);
    }
    if pixel[3] <= 0.0 {
        return pixel;
    }
    let (c, alpha) = straight(pixel);
    let e = encoded_color(c);
    let inverted = e.map(|v| 1.0 - v);
    let mixed = if channel == 0 {
        zip3(e, inverted, |e, inv| inv + (e - inv) * blend)
    } else {
        let mix = |index: u32, e: f32, inv: f32| {
            if channel == index {
                inv + (e - inv) * blend
            } else {
                e
            }
        };
        let [er, eg, eb] = e;
        let [ir, ig, ib] = inverted;
        [mix(1, er, ir), mix(2, eg, ig), mix(3, eb, ib)]
    };
    premultiply(decoded_color(mixed), alpha)
}
/// Encoded-space quantisation into `n + 1` levels.
fn posterize_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let n = uniform(values, 0).max(1.0);
    let (c, alpha) = straight(pixel);
    let quantized = encoded_color(c).map(|v| (v * n).round() / n);
    premultiply(decoded_color(quantized), alpha)
}
/// Encoded-space input/output remap with a gamma power.
fn levels_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let [in_black, in_white, out_black, out_white, gamma] =
        [0, 1, 2, 3, 4].map(|i| uniform(values, i));
    let span = (in_white - in_black).max(1e-3);
    let (c, alpha) = straight(pixel);
    let mapped = encoded_color(c).map(|v| {
        out_black + ((v - in_black) / span).clamp(0.0, 1.0).powf(gamma) * (out_white - out_black)
    });
    premultiply(decoded_color(mapped), alpha)
}
/// Encoded-space power; a gain of 1 is the identity.
fn gamma_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let gamma = uniform(values, 0).max(1e-3);
    let (c, alpha) = straight(pixel);
    let mapped = encoded_color(c).map(|v| v.max(0.0).powf(gamma));
    premultiply(decoded_color(mapped), alpha)
}
/// Encoded luma ramps between the two mapped colours, then mixes by amount.
fn tint_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let black = uniform_color(values, 0);
    let white = uniform_color(values, 3);
    let amount = uniform(values, 6).clamp(0.0, 1.0);
    let (c, alpha) = straight(pixel);
    let l = crate::image::oetf(luma(c).max(0.0), 13);
    let target = zip3(black, white, |b, w| b + (w - b) * l);
    let e = encoded_color(c);
    let mixed = zip3(e, target, |e, t| e + (t - e) * amount);
    premultiply(decoded_color(mixed), alpha)
}
/// Keeps colours within `similarity` of the key and greys the rest.
fn color_pass_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let key = uniform_color(values, 0);
    let similarity = uniform(values, 3).clamp(0.0, 1.2);
    let reverse = uniform(values, 4) > 0.5;
    let (c, alpha) = straight(pixel);
    let e = encoded_color(c);
    if (distance(e, key) <= similarity * 1.2) != reverse {
        return pixel;
    }
    premultiply([luma(c); 3], alpha)
}
/// Replaces colours within `similarity` of the target, keeping the pixel's
/// lightness unless `solid` asks for the flat replace colour.
fn color_replace_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let similarity = uniform(values, 0).clamp(0.0, 1.2);
    let solid = uniform(values, 1) > 0.5;
    let target = uniform_color(values, 2);
    let replace = uniform_color(values, 5);
    let (c, alpha) = straight(pixel);
    let e = encoded_color(c);
    let weight = 1.0 - smoothstep(similarity * 0.85, similarity.max(1e-4), distance(e, target));
    if weight <= 0.0 {
        return pixel;
    }
    let to = if solid {
        replace
    } else {
        let [h, s, _] = hsl(replace);
        hsl_color(h, s, hsl(e)[2])
    };
    let mixed = zip3(e, to, |e, to| e + (to - e) * weight);
    premultiply(decoded_color(mixed), alpha)
}
/// Encoded 3x4 channel matrix; the last value of each row is a constant.
fn channel_mix_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let e = encoded_color(c);
    let [er, eg, eb] = e;
    let mixed = [0, 1, 2].map(|k| {
        let row = k * 4;
        (uniform(values, row) * er
            + uniform(values, row + 1) * eg
            + uniform(values, row + 2) * eb
            + uniform(values, row + 3))
        .clamp(0.0, 1.0)
    });
    premultiply(decoded_color(mixed), alpha)
}
/// Black-and-white threshold on encoded luma with an optional feathered band.
fn extract_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let low = uniform(values, 0);
    let high = uniform(values, 1);
    let softness = uniform(values, 2).max(0.0);
    let invert = uniform(values, 3) > 0.5;
    let (c, alpha) = straight(pixel);
    let l = crate::image::oetf(luma(c).max(0.0), 13);
    let inside = if softness <= 0.0 {
        if l > low && l < high { 1.0 } else { 0.0 }
    } else {
        ((l - low) / softness)
            .clamp(0.0, 1.0)
            .min(((high - l) / softness).clamp(0.0, 1.0))
    };
    premultiply([if invert { 1.0 - inside } else { inside }; 3], alpha)
}
/// Mixes the linear colour toward the strobe colour by `blend`.
fn strobe_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let strobe = decoded_color(uniform_color(values, 0));
    let blend = uniform(values, 3).clamp(0.0, 1.0);
    let (c, alpha) = straight(pixel);
    let mixed = zip3(strobe, c, |s, c| s + (c - s) * blend);
    premultiply(mixed, alpha)
}
fn op(entry_point: &str) -> Option<&'static EffectOp> {
    OPS.iter().find(|op| op.entry_point == entry_point)
}
fn pixel_op(entry_point: &str) -> Option<PixelOp> {
    match op(entry_point)?.kernel {
        Kernel::Pixel(op) => Some(op),
        _ => None,
    }
}
fn sample_op(entry_point: &str) -> Option<SampleOp> {
    match op(entry_point)?.kernel {
        Kernel::Sample(op) => Some(op),
        _ => None,
    }
}
/// A shader entry a device adapter builds a pipeline for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderEntry {
    pub name: &'static str,
    /// The image it binds besides the current one.
    pub reads: Option<ReadImage>,
}
fn shader_entry_of(op: &EffectOp) -> Option<ShaderEntry> {
    let reads = match op.wgsl {
        Wgsl::Absent => return None,
        Wgsl::Entry => None,
        Wgsl::Reads(image) => Some(image),
    };
    Some(ShaderEntry {
        name: op.entry_point,
        reads,
    })
}
/// Every compute entry `effects.wgsl` must define, in table order.
pub fn shader_entries() -> impl Iterator<Item = ShaderEntry> {
    OPS.iter().filter_map(shader_entry_of)
}
/// The shader entry for `name`, if the table gives it one.
pub fn shader_entry(name: &str) -> Option<ShaderEntry> {
    shader_entry_of(op(name)?)
}
/// The two images a sample op reads: `current`, the output of the previous pass,
/// and `input`, the image the effect instance started with.
pub(crate) struct SampleInputs<'a> {
    pub current: &'a Image,
    pub input: &'a Image,
}
impl SampleInputs<'_> {
    /// Exact texel of the current image; transparent outside it.
    pub fn texel_current(&self, x: u32, y: u32) -> [f32; 4] {
        texel(self.current, x, y)
    }
    /// Exact texel of the instance's input image; transparent outside it.
    pub fn texel_input(&self, x: u32, y: u32) -> [f32; 4] {
        texel(self.input, x, y)
    }
    /// Bilinear sample of the current image in texel-index coordinates (integers are exact).
    #[allow(dead_code, reason = "first used by the 264 and 265 ops")]
    pub fn sample_current(&self, x: f32, y: f32, edge: image_ops::Edge) -> [f32; 4] {
        image_ops::sample(self.current, x, y, edge)
    }
    /// Bilinear sample of the instance input in texel-index coordinates (integers are exact).
    pub fn sample_input(&self, x: f32, y: f32, edge: image_ops::Edge) -> [f32; 4] {
        image_ops::sample(self.input, x, y, edge)
    }
}
fn texel(image: &Image, x: u32, y: u32) -> [f32; 4] {
    if x >= image.width || y >= image.height {
        return [0.0; 4];
    }
    image
        .pixels
        .get(y as usize * image.width as usize + x as usize)
        .copied()
        .unwrap_or([0.0; 4])
}
/// A sample op: flat uniform count, uniform check and the CPU reference a shader
/// entry mirrors (one output pixel from its coordinate, the uniforms and
/// [`SampleInputs`]).
#[derive(Clone, Copy)]
struct SampleOp {
    len: usize,
    valid: fn(&[f32], u32, u32) -> bool,
    apply: fn(&SampleInputs<'_>, u32, u32, &[f32]) -> [f32; 4],
}
static OPS: &[EffectOp] = &[
    EffectOp {
        entry_point: GAUSSIAN,
        wgsl: Wgsl::Entry,
        kernel: Kernel::Convolve,
    },
    EffectOp {
        entry_point: BLEND_ENTRY,
        wgsl: Wgsl::Reads(ReadImage::Below),
        kernel: Kernel::Composite,
    },
    EffectOp {
        entry_point: "grade",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 4,
            valid: |v| v.get(1).is_some_and(|exponent| *exponent > 0.0),
            apply: grade_pixel,
        }),
    },
    EffectOp {
        entry_point: "lift",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 3,
            valid: lift_uniforms,
            apply: lift_pixel,
        }),
    },
    EffectOp {
        entry_point: "tone_gamma",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 3,
            valid: gamma_uniforms,
            apply: tone_gamma_pixel,
        }),
    },
    EffectOp {
        entry_point: "gain",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 3,
            valid: gain_uniforms,
            apply: gain_pixel,
        }),
    },
    EffectOp {
        entry_point: "tone",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 5,
            valid: tone_uniforms,
            apply: tone_pixel,
        }),
    },
    EffectOp {
        entry_point: "hsl_bands",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: HSL_BAND_UNIFORMS,
            valid: hsl_band_uniforms,
            apply: hsl_bands_pixel,
        }),
    },
    EffectOp {
        entry_point: "curve_red",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: CURVE_UNIFORMS,
            valid: curve_uniforms_valid,
            apply: curve_red_pixel,
        }),
    },
    EffectOp {
        entry_point: "curve_green",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: CURVE_UNIFORMS,
            valid: curve_uniforms_valid,
            apply: curve_green_pixel,
        }),
    },
    EffectOp {
        entry_point: "curve_blue",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: CURVE_UNIFORMS,
            valid: curve_uniforms_valid,
            apply: curve_blue_pixel,
        }),
    },
    EffectOp {
        entry_point: "curve_luma",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: CURVE_UNIFORMS,
            valid: curve_uniforms_valid,
            apply: curve_luma_pixel,
        }),
    },
    EffectOp {
        entry_point: "chroma_key",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 5,
            valid: |v| v.iter().all(|c| (0.0..=1.0).contains(c)),
            apply: chroma_key_pixel,
        }),
    },
    EffectOp {
        entry_point: "highlights",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 2,
            valid: |v| {
                v.first()
                    .is_some_and(|threshold| (0.0..=1.0).contains(threshold))
                    && v.get(1).is_some_and(|soft| *soft > 0.0 && *soft <= 1.0)
            },
            apply: whole_image::highlights,
        }),
    },
    EffectOp {
        entry_point: "alpha_plane",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 0,
            valid: |_| true,
            apply: whole_image::alpha_plane,
        }),
    },
    EffectOp {
        entry_point: "color_key",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 6,
            valid: whole_image::valid_color_key,
            apply: whole_image::color_key,
        }),
    },
    EffectOp {
        entry_point: "luma_key",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 3,
            valid: whole_image::valid_luma_key,
            apply: whole_image::luma_key,
        }),
    },
    EffectOp {
        entry_point: "black_white",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 0,
            valid: |_| true,
            apply: black_white_pixel,
        }),
    },
    EffectOp {
        entry_point: "channel_mix",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 12,
            valid: |v| v.iter().all(|g| (-2.0..=2.0).contains(g)),
            apply: channel_mix_pixel,
        }),
    },
    EffectOp {
        entry_point: "color_pass",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 5,
            valid: |v| {
                v.get(..3)
                    .is_some_and(|key| key.iter().all(|c| (0.0..=1.0).contains(c)))
                    && v.get(3).is_some_and(|sim| (0.0..=1.2).contains(sim))
                    && v.get(4)
                        .is_some_and(|reverse| (0.0..=1.0).contains(reverse))
            },
            apply: color_pass_pixel,
        }),
    },
    EffectOp {
        entry_point: "color_replace",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 8,
            valid: |v| {
                v.first().is_some_and(|sim| (0.0..=1.2).contains(sim))
                    && v.get(1).is_some_and(|solid| (0.0..=1.0).contains(solid))
                    && v.get(2..8)
                        .is_some_and(|c| c.iter().all(|v| (0.0..=1.0).contains(v)))
            },
            apply: color_replace_pixel,
        }),
    },
    EffectOp {
        entry_point: "extract",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 4,
            valid: |v| {
                v.first().is_some_and(|lo| (0.0..=1.0).contains(lo))
                    && v.get(1).is_some_and(|hi| (0.0..=1.0).contains(hi))
                    && v.get(2).is_some_and(|soft| (0.0..=0.2001).contains(soft))
                    && v.get(3).is_some_and(|invert| (0.0..=1.0).contains(invert))
            },
            apply: extract_pixel,
        }),
    },
    EffectOp {
        entry_point: "gamma",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 1,
            valid: |v| v.first().is_some_and(|g| *g > 0.0 && *g <= 4.0),
            apply: gamma_pixel,
        }),
    },
    EffectOp {
        entry_point: "invert",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 2,
            valid: |v| {
                v.first()
                    .is_some_and(|channel| (0.0..=4.0).contains(channel))
                    && v.get(1).is_some_and(|blend| (0.0..=1.0).contains(blend))
            },
            apply: invert_pixel,
        }),
    },
    EffectOp {
        entry_point: "levels",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 5,
            valid: |v| {
                v.get(0..4)
                    .is_some_and(|bounds| bounds.iter().all(|b| (0.0..=1.0).contains(b)))
                    && v.get(4).is_some_and(|gamma| *gamma > 0.0 && *gamma <= 12.0)
            },
            apply: levels_pixel,
        }),
    },
    EffectOp {
        entry_point: "posterize",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Pixel(PixelOp {
            len: 1,
            valid: |v| v.first().is_some_and(|n| *n >= 1.0 && n.is_finite()),
            apply: posterize_pixel,
        }),
    },
    EffectOp {
        entry_point: "strobe",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 4,
            valid: |v| {
                v.get(0..3)
                    .is_some_and(|c| c.iter().all(|v| (0.0..=1.0).contains(v)))
                    && v.get(3).is_some_and(|blend| (0.0..=1.0).contains(blend))
            },
            apply: strobe_pixel,
        }),
    },
    EffectOp {
        entry_point: "tint",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Pixel(PixelOp {
            len: 7,
            valid: |v| {
                v.get(0..6)
                    .is_some_and(|c| c.iter().all(|v| (0.0..=1.0).contains(v)))
                    && v.get(6).is_some_and(|amount| (0.0..=1.0).contains(amount))
            },
            apply: tint_pixel,
        }),
    },
    EffectOp {
        entry_point: "pixelate",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 3,
            valid: |v, _, _| {
                v.first()
                    .is_some_and(|block| block.is_finite() && *block >= 1.0)
                    && v.get(1)
                        .is_some_and(|block| block.is_finite() && *block >= 1.0)
                    && v.get(2)
                        .is_some_and(|softness| (0.0..=1.0).contains(softness))
            },
            apply: pixelate_sample,
        }),
    },
    EffectOp {
        entry_point: "detail_boost",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 2,
            valid: |v, _, _| {
                v.first().is_some_and(|gain| (0.0..=40.0).contains(gain))
                    && v.get(1)
                        .is_some_and(|threshold| (0.0..=1.0).contains(threshold))
            },
            apply: detail_boost_sample,
        }),
    },
    EffectOp {
        entry_point: "lens_blur",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 7,
            valid: spatial::valid_lens_blur,
            apply: spatial::lens_blur,
        }),
    },
    EffectOp {
        entry_point: "wrap_shift",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 3,
            valid: spatial::valid_wrap_shift,
            apply: spatial::wrap_shift,
        }),
    },
    EffectOp {
        entry_point: "vignette",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 1,
            valid: |v, _, _| v.first().is_some_and(|a| (-1.0..=1.0).contains(a)),
            apply: vignette_sample,
        }),
    },
    EffectOp {
        entry_point: "reduce_noise",
        wgsl: Wgsl::Entry,
        kernel: Kernel::Sample(SampleOp {
            len: 2,
            valid: denoise::valid_reduce_noise,
            apply: denoise::reduce_noise_sample,
        }),
    },
    EffectOp {
        entry_point: super::matte::ENTRY,
        wgsl: Wgsl::Reads(ReadImage::Instance),
        kernel: Kernel::Sample(SampleOp {
            len: 6,
            valid: super::matte::valid,
            apply: super::matte::sample,
        }),
    },
    EffectOp {
        entry_point: "grain",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 1,
            valid: |v, _, _| v.first().is_some_and(|a| (-1.0..=1.0).contains(a)),
            apply: grain_sample,
        }),
    },
    EffectOp {
        entry_point: "mirror",
        wgsl: Wgsl::Reads(ReadImage::Instance),
        kernel: Kernel::Sample(SampleOp {
            len: 4,
            valid: mirror_uniforms,
            apply: mirror,
        }),
    },
    EffectOp {
        entry_point: "streak_blur",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 3,
            valid: |v, _, _| {
                v.get(2)
                    .is_some_and(|n| (2.0..=64.0).contains(n) && n.fract() == 0.0)
            },
            apply: streak_sample,
        }),
    },
    EffectOp {
        entry_point: "edge_lines",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 2,
            valid: |v, _, _| {
                v.first().is_some_and(|invert| (0.0..=1.0).contains(invert))
                    && v.get(1).is_some_and(|blend| (0.0..=1.0).contains(blend))
            },
            apply: edge_lines_sample,
        }),
    },
    EffectOp {
        entry_point: "colour_relief",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 4,
            valid: |v, _, _| {
                v.get(2)
                    .is_some_and(|contrast| (0.0..=5.0).contains(contrast))
                    && v.get(3).is_some_and(|blend| (0.0..=1.0).contains(blend))
            },
            apply: colour_relief_sample,
        }),
    },
    EffectOp {
        entry_point: "twirl",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 4,
            valid: whole_image::valid_twirl,
            apply: whole_image::twirl,
        }),
    },
    EffectOp {
        entry_point: "spherize",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 3,
            valid: whole_image::valid_spherize,
            apply: whole_image::spherize,
        }),
    },
    EffectOp {
        entry_point: "magnify",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 6,
            valid: whole_image::valid_magnify,
            apply: whole_image::magnify,
        }),
    },
    EffectOp {
        entry_point: "wave_warp",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 4,
            valid: whole_image::valid_wave,
            apply: whole_image::wave_warp,
        }),
    },
    EffectOp {
        entry_point: "lens_distortion",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 4,
            valid: whole_image::valid_lens,
            apply: whole_image::lens_distortion,
        }),
    },
    EffectOp {
        entry_point: "turbulent_displace",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 8,
            valid: whole_image::valid_turbulent,
            apply: whole_image::turbulent_displace,
        }),
    },
    EffectOp {
        entry_point: "corner_pin",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 11,
            valid: whole_image::valid_corner,
            apply: whole_image::corner_pin,
        }),
    },
    EffectOp {
        entry_point: "basic_3d",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 6,
            valid: whole_image::valid_basic_3d,
            apply: whole_image::basic_3d,
        }),
    },
    EffectOp {
        entry_point: "brush_strokes",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 5,
            valid: whole_image::valid_brush,
            apply: whole_image::brush_strokes,
        }),
    },
    EffectOp {
        entry_point: "roughen_edges",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 8,
            valid: whole_image::valid_roughen,
            apply: whole_image::roughen_edges,
        }),
    },
    EffectOp {
        entry_point: "camera_shake",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 5,
            valid: whole_image::valid_shake,
            apply: whole_image::camera_shake,
        }),
    },
    EffectOp {
        entry_point: "sobel_edges",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 1,
            valid: whole_image::valid_sobel,
            apply: whole_image::sobel_edges,
        }),
    },
    EffectOp {
        entry_point: "glow_add",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 8,
            valid: whole_image::valid_glow,
            apply: whole_image::glow_add,
        }),
    },
    EffectOp {
        entry_point: "glint_rays",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 9,
            valid: whole_image::valid_glint,
            apply: whole_image::glint_rays,
        }),
    },
    EffectOp {
        entry_point: "radial_rays",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 9,
            valid: whole_image::valid_radial,
            apply: whole_image::radial_rays,
        }),
    },
    EffectOp {
        entry_point: "light_leaks",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 12,
            valid: whole_image::valid_leaks,
            apply: whole_image::light_leaks,
        }),
    },
    EffectOp {
        entry_point: "lens_flare",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 4,
            valid: whole_image::valid_flare,
            apply: whole_image::lens_flare,
        }),
    },
    EffectOp {
        entry_point: "rgb_split",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 7,
            valid: whole_image::valid_split,
            apply: whole_image::rgb_split,
        }),
    },
    EffectOp {
        entry_point: "drop_shadow",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 7,
            valid: whole_image::valid_drop,
            apply: whole_image::drop_shadow,
        }),
    },
    EffectOp {
        entry_point: "long_shadow",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 10,
            valid: whole_image::valid_long,
            apply: whole_image::long_shadow,
        }),
    },
    EffectOp {
        entry_point: "gradient",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 12,
            valid: whole_image::valid_gradient,
            apply: whole_image::gradient,
        }),
    },
    EffectOp {
        entry_point: "four_color_gradient",
        wgsl: Wgsl::Absent,
        kernel: Kernel::Sample(SampleOp {
            len: 20,
            valid: whole_image::valid_four,
            apply: whole_image::four_color_gradient,
        }),
    },
];
/// Premultiplied linear luma of a sample near the current texel.
fn sample_luma(pixel: [f32; 4]) -> f32 {
    luma([pixel[0], pixel[1], pixel[2]])
}
/// One fixed output-pixel Sobel over the current image with clamped borders.
fn edge_lines_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let Some(&[invert, blend]) = values.first_chunk::<2>() else {
        return inputs.texel_current(x, y);
    };
    let (w, h) = (inputs.current.width(), inputs.current.height());
    let l = |dx: i64, dy: i64| {
        let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as u32;
        let sy = (y as i64 + dy).clamp(0, h as i64 - 1) as u32;
        sample_luma(inputs.texel_current(sx, sy))
    };
    let gx = l(1, -1) + 2.0 * l(1, 0) + l(1, 1) - l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1);
    let gy = l(-1, 1) + 2.0 * l(0, 1) + l(1, 1) - l(-1, -1) - 2.0 * l(0, -1) - l(1, -1);
    let mut m = gx.hypot(gy).min(1.0);
    if invert <= 0.5 {
        m = 1.0 - m;
    }
    let centre = inputs.texel_current(x, y);
    let alpha = centre[3];
    let blend = blend.clamp(0.0, 1.0);
    let level = m * alpha * (1.0 - blend);
    [
        level + centre[0] * blend,
        level + centre[1] * blend,
        level + centre[2] * blend,
        alpha,
    ]
}
/// Straight linear RGB of one sample; transparent neighbours contribute zero.
fn straight_sample(pixel: [f32; 4]) -> [f32; 3] {
    if pixel[3] <= 1e-6 {
        [0.0; 3]
    } else {
        [
            pixel[0] / pixel[3],
            pixel[1] / pixel[3],
            pixel[2] / pixel[3],
        ]
    }
}
/// Colour-preserving relief: two clamped straight-luma neighbours shift the
/// encoded centre colour, then the encoded result mixes back toward the original.
fn colour_relief_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let Some(&[dx, dy, contrast, blend]) = values.first_chunk::<4>() else {
        return inputs.texel_current(x, y);
    };
    let centre = inputs.texel_current(x, y);
    let alpha = centre[3];
    if alpha <= 1e-6 {
        return centre;
    }
    let (w, h) = (inputs.current.width(), inputs.current.height());
    let plus = inputs.sample_current(
        (x as f32 + dx).clamp(0.0, (w - 1) as f32),
        (y as f32 + dy).clamp(0.0, (h - 1) as f32),
        image_ops::Edge::Clamp,
    );
    let minus = inputs.sample_current(
        (x as f32 - dx).clamp(0.0, (w - 1) as f32),
        (y as f32 - dy).clamp(0.0, (h - 1) as f32),
        image_ops::Edge::Clamp,
    );
    let difference = luma(straight_sample(plus)) - luma(straight_sample(minus));
    let encoded = encoded_color([centre[0] / alpha, centre[1] / alpha, centre[2] / alpha]);
    let relief = encoded.map(|v| (v + 2.0 * contrast * difference).clamp(0.0, 1.0));
    let mixed = zip3(relief, encoded, |r, e| r + (e - r) * blend.clamp(0.0, 1.0));
    premultiply(decoded_color(mixed), alpha)
}
/// Pixelate divides the picture into the authored number of blocks per axis and
/// reads the immutable input at block centres. Zero softness (or the sharp flag,
/// which forces it) takes the nearest tile colour; otherwise the four surrounding
/// tile centres blend with the block fraction, mixed toward that lattice by the
/// softness fraction. At most five bilinear samples per output pixel: the four
/// lattice corners plus the nearest tile colour.
fn pixelate_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let Some(&[bw, bh, softness]) = values.first_chunk::<3>() else {
        return inputs.texel_current(x, y);
    };
    let (w, h) = (inputs.input.width(), inputs.input.height());
    let tiles_x = (w as f32 / bw).ceil().max(1.0);
    let tiles_y = (h as f32 / bh).ceil().max(1.0);
    let tile = |coordinate: u32, block: f32, tiles: f32| {
        let raw = coordinate as f32 / block;
        let index = raw.floor().clamp(0.0, tiles - 1.0);
        (index, raw - raw.floor())
    };
    let (ix, _) = tile(x, bw, tiles_x);
    let (iy, _) = tile(y, bh, tiles_y);
    let centre = |i: f32, j: f32| {
        let px = ((i + 0.5) * bw - 0.5).clamp(0.0, (w - 1) as f32);
        let py = ((j + 0.5) * bh - 0.5).clamp(0.0, (h - 1) as f32);
        inputs.sample_input(px, py, image_ops::Edge::Clamp)
    };
    let nearest = centre(ix, iy);
    if softness <= 0.0 {
        return nearest;
    }
    // The source samples a virtual tile image in continuous coordinates with
    // pixel centres at half offsets, so map once into texel-index tile space and
    // clamp the four lattice indices at the picture edge.
    let lattice_at = |coordinate: u32, block: f32| {
        let raw = (coordinate as f32 + 0.5) / block - 0.5;
        (raw.floor(), raw - raw.floor())
    };
    let (i0, fx) = lattice_at(x, bw);
    let (j0, fy) = lattice_at(y, bh);
    let tile_at = |i: f32, j: f32| centre(i.clamp(0.0, tiles_x - 1.0), j.clamp(0.0, tiles_y - 1.0));
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| {
        [0, 1, 2, 3].map(|k| {
            let (a, b) = (
                a.get(k).copied().unwrap_or(0.0),
                b.get(k).copied().unwrap_or(0.0),
            );
            a + (b - a) * t
        })
    };
    let top = mix(tile_at(i0, j0), tile_at(i0 + 1.0, j0), fx);
    let bottom = mix(tile_at(i0, j0 + 1.0), tile_at(i0 + 1.0, j0 + 1.0), fx);
    let lattice = mix(top, bottom, fy);
    mix(nearest, lattice, softness)
}
/// Unsharp detail: the current image is the blurred copy a preceding gaussian
/// convolution produced, the immutable instance input is the original picture.
/// Per channel the difference is added back by `gain`, gated by `threshold` times
/// the original coverage so transparent pixels keep their alpha. Clamping to the
/// working-image contract happens once at the pass exit.
fn detail_boost_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let Some(&[gain, threshold]) = values.first_chunk::<2>() else {
        return inputs.texel_current(x, y);
    };
    let original = inputs.texel_input(x, y);
    let blurred = inputs.texel_current(x, y);
    let alpha = original[3];
    let mut out = original;
    for (channel, blurred) in out.iter_mut().zip(blurred).take(3) {
        let difference = *channel - blurred;
        let keep = threshold <= 0.0 || difference.abs() >= threshold * alpha;
        *channel += if keep { gain * difference } else { 0.0 };
    }
    out
}
/// A deterministic per-pixel noise value in 0..=1 from the output coordinate.
fn grain_noise(x: u32, y: u32) -> f32 {
    let mut hash = x
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263));
    hash = (hash ^ (hash >> 13)).wrapping_mul(1_274_126_177);
    hash ^= hash >> 16;
    (hash & 0xFFFF) as f32 / 65_535.0
}
/// Radial darkening or brightening in linear light; `amount` is -1..=1 and
/// the frame centre never moves.
fn vignette_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let amount = uniform(values, 0).clamp(-1.0, 1.0);
    let pixel = inputs.texel_current(x, y);
    let (width, height) = (inputs.current.width(), inputs.current.height());
    let dx = (x as f32 + 0.5) / width.max(1) as f32 * 2.0 - 1.0;
    let dy = (y as f32 + 0.5) / height.max(1) as f32 * 2.0 - 1.0;
    let radius = ((dx * dx + dy * dy) * 0.5).clamp(0.0, 1.0);
    let factor = (1.0 - amount * radius * 0.8).max(0.0);
    [
        pixel[0] * factor,
        pixel[1] * factor,
        pixel[2] * factor,
        pixel[3],
    ]
}
/// Luma grain mixed into straight linear colour; `amount` is -1..=1.
fn grain_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let amount = uniform(values, 0).clamp(-1.0, 1.0);
    let (c, alpha) = straight(inputs.texel_current(x, y));
    let delta = (grain_noise(x, y) - 0.5) * amount * 0.2;
    premultiply(c.map(|v| v + delta), alpha)
}
/// Clamp coordinates while still floating point, before the sampler's integer lookup.
fn streak_sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let Some(&[dx, dy, steps]) = values.first_chunk::<3>() else {
        return inputs.texel_current(x, y);
    };
    let mut sum = [0.0; 4];
    for s in 0..steps as u32 {
        let t = s as f32 / (steps - 1.0) - 0.5;
        let px = (x as f32 + dx * t).clamp(0.0, (inputs.current.width() - 1) as f32);
        let py = (y as f32 + dy * t).clamp(0.0, (inputs.current.height() - 1) as f32);
        let pixel = inputs.sample_current(px, py, image_ops::Edge::Clamp);
        for (out, value) in sum.iter_mut().zip(pixel) {
            *out += value / steps;
        }
    }
    sum
}
fn streak_blur(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    if geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    let a = frame.scalar("direction")?.to_radians();
    let length = frame.scalar("length")?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let dx = 2.0 * length * a.sin() * gx;
    let dy = -2.0 * length * a.cos() * gy;
    // Equal gains cancel direction analytically. Reconstructing this length
    // from rounded trig components can cross an integer and add a whole tap.
    let span = if gx == gy {
        2.0 * length * gx
    } else {
        dx.hypot(dy)
    };
    if !span.is_finite() || !((dx as f32).is_finite() && (dy as f32).is_finite()) {
        return Err(Error::Invalid("effect spatial extent"));
    }
    if span < 0.5 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "streak_blur",
        uniforms: vec![
            Uniform::Float(dx as f32),
            Uniform::Float(dy as f32),
            Uniform::Float(span.ceil().clamp(2.0, 64.0) as f32),
        ],
    }])
}
/// Find Edges is a fixed output-pixel Sobel: geometry gains deliberately do not
/// scale it. Blend 100 returns the original picture and emits no pass.
fn edge_lines(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let blend = frame.scalar("blend")?;
    if blend >= 100.0 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "edge_lines",
        uniforms: vec![
            Uniform::Float(if frame.flag("invert")? { 1.0 } else { 0.0 }),
            Uniform::Float((blend / 100.0) as f32),
        ],
    }])
}
/// Colour Relief offsets each axis by its own gain floored at .25; the source's
/// uniform scale is the uniform-gain case of this admitted nonuniform extension.
fn colour_relief(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    if geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    let relief = frame.scalar("relief")?;
    let contrast = frame.scalar("contrast")?;
    let blend = frame.scalar("blend")?;
    if relief == 0.0 || contrast == 0.0 || blend >= 100.0 {
        return Ok(Vec::new());
    }
    let angle = frame.scalar("direction")?.to_radians();
    let [gx, gy] = geometry.clip_pixel_scale();
    let dx = angle.cos() * relief * gx.max(0.25);
    let dy = -angle.sin() * relief * gy.max(0.25);
    if !dx.is_finite() || !dy.is_finite() || !((dx as f32).is_finite() && (dy as f32).is_finite()) {
        return Err(Error::Invalid("effect spatial extent"));
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "colour_relief",
        uniforms: vec![
            Uniform::Float(dx as f32),
            Uniform::Float(dy as f32),
            Uniform::Float((contrast / 100.0) as f32),
            Uniform::Float((blend / 100.0) as f32),
        ],
    }])
}
/// Mirror uniforms use an image-edge centre (admitted against geometry below)
/// and an explicitly unit normal. Components are in [-1,1]; squared length may
/// differ from 1 by at most eight f32 epsilons for representation roundoff.
fn mirror_uniforms(values: &[f32], width: u32, height: u32) -> bool {
    let Some(&[cx, cy, nx, ny]) = values.first_chunk::<4>() else {
        return false;
    };
    values.len() == 4
        && values.iter().all(|c| c.is_finite())
        && nx.abs() <= 1.0
        && ny.abs() <= 1.0
        && (nx * nx + ny * ny - 1.0).abs() <= 8.0 * f32::EPSILON
        && (0.0..=width as f32).contains(&cx)
        && (0.0..=height as f32).contains(&cy)
}
/// Reflect the picture about the line through `(cx, cy)` in output pixels with unit
/// normal `(nx, ny)`: pixels on the direction's positive side take the bilinear
/// sample of their reflection (transparent borders), the rest keep their value.
fn mirror(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let Some(&[cx, cy, nx, ny]) = values.first_chunk::<4>() else {
        return inputs.texel_current(x, y);
    };
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let distance = (px - cx) * nx + (py - cy) * ny;
    if distance > 0.0 {
        inputs.sample_input(
            px - 2.0 * distance * nx - 0.5,
            py - 2.0 * distance * ny - 0.5,
            image_ops::Edge::Transparent,
        )
    } else {
        inputs.texel_current(x, y)
    }
}
/// Clamp one sample result to the working-image contract (alpha in `0..=1`, colour
/// in `0..=alpha`, all finite) once, at the pass exit, so no op repeats it.
fn clamp_pixel(pixel: [f32; 4]) -> [f32; 4] {
    let alpha = if pixel[3].is_finite() {
        pixel[3].clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut out = pixel;
    for channel in out.iter_mut().take(3) {
        *channel = if channel.is_finite() {
            channel.clamp(0.0, alpha)
        } else {
            0.0
        };
    }
    if let Some(last) = out.last_mut() {
        *last = alpha;
    }
    out
}
impl EffectCompositor for CpuEffects {
    fn frame<'a>(
        &'a self,
        _: &'a dyn crate::ExecutionHooks,
    ) -> crate::Result<Option<Box<dyn crate::effects::EffectCompositor + 'a>>> {
        Ok(None)
    }
    fn release_idle(&self) -> crate::Result<()> {
        Ok(())
    }
    fn supports(&self, _: &str) -> bool {
        true
    }
    fn composite_layers(
        &self,
        width: u32,
        height: u32,
        layers: &[FrameLayer<'_>],
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        reference_layers(width, height, layers, hooks)
    }
    fn apply(
        &self,
        item: &Image,
        passes: &[EffectPass],
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        check(hooks)?;
        inputs.validate(item)?;
        for pass in passes {
            validate_pass(pass, item.width(), item.height())?;
        }
        let mut a = item.copy(hooks)?;
        let mut b = Image::new(item.width(), item.height(), [0.0; 4], hooks)?;
        for pass in passes {
            let (horizontal, weights) = match pass {
                EffectPass::Lut3d { table, strength } => {
                    crate::image::row_chunks(
                        &mut b.pixels,
                        a.width as usize,
                        hooks,
                        |start, rows| {
                            check(hooks)?;
                            let source = a
                                .pixels
                                .get(start..start + rows.len())
                                .ok_or(Error::Invalid("LUT pixels"))?;
                            for (out_row, in_row) in rows
                                .chunks_mut(a.width as usize)
                                .zip(source.chunks(a.width as usize))
                            {
                                check(hooks)?;
                                for (out, pixel) in out_row.iter_mut().zip(in_row) {
                                    *out = table.pixel(*pixel, *strength);
                                }
                            }
                            Ok(())
                        },
                    )?;
                    std::mem::swap(&mut a, &mut b);
                    continue;
                }
                EffectPass::Sample { entry_point, .. } => {
                    let op = sample_op(entry_point).ok_or(Error::Unsupported("effect pass"))?;
                    let values = pass.values();
                    let inputs = SampleInputs {
                        current: &a,
                        input: inputs.input,
                    };
                    let width = a.width as usize;
                    crate::image::row_chunks(&mut b.pixels, width, hooks, |start, rows| {
                        check(hooks)?;
                        for (row, pixels) in rows.chunks_mut(width).enumerate() {
                            check(hooks)?;
                            let y = (start / width + row) as u32;
                            for (x, pixel) in pixels.iter_mut().enumerate() {
                                *pixel = clamp_pixel((op.apply)(&inputs, x as u32, y, &values));
                            }
                        }
                        Ok(())
                    })?;
                    std::mem::swap(&mut a, &mut b);
                    continue;
                }
                EffectPass::Pixel { entry_point, .. } => {
                    let op = pixel_op(entry_point).ok_or(Error::Unsupported("effect pass"))?;
                    let values = pass.values();
                    crate::image::row_chunks(
                        &mut b.pixels,
                        a.width as usize,
                        hooks,
                        |start, rows| {
                            check(hooks)?;
                            let source = a
                                .pixels
                                .get(start..start + rows.len())
                                .ok_or(Error::Invalid("effect pixel"))?;
                            for (out, px) in rows.iter_mut().zip(source) {
                                *out = (op.apply)(*px, &values);
                            }
                            Ok(())
                        },
                    )?;
                    std::mem::swap(&mut a, &mut b);
                    continue;
                }
                EffectPass::Convolve {
                    horizontal,
                    weights,
                    ..
                } => (*horizontal, weights),
            };
            let w = a.width as usize;
            let h = a.height as usize;
            let radius = weights.len() as i64 / 2;
            crate::image::row_chunks(&mut b.pixels, w, hooks, |start, rows| {
                for (row, pixels) in rows.chunks_mut(w).enumerate() {
                    check(hooks)?;
                    let y = start / w + row;
                    for (x, output) in pixels.iter_mut().enumerate() {
                        let mut sum = [0.0; 4];
                        for (tap, weight) in weights.iter().enumerate() {
                            let delta = tap as i64 - radius;
                            let sx = if horizontal {
                                (x as i64 + delta).clamp(0, w as i64 - 1) as usize
                            } else {
                                x
                            };
                            let sy = if horizontal {
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
/// Admit a pass for positive output dimensions within Image's area bound.
/// Mirror centres lie in [0,width] x [0,height] in image-edge pixels, with an
/// explicitly unit normal. Thus |distance| <= width+height and each reflected
/// coordinate is bounded by 3*(width+height)+0.5, safely inside WGSL's i32 range
/// under Image's 2^26 pixel cap. Invalid geometry/uniforms are Unsupported.
pub fn validate_pass(pass: &EffectPass, width: u32, height: u32) -> Result<()> {
    Image::bytes(width, height)?;
    let valid = match pass {
        EffectPass::Lut3d { strength, .. } => {
            strength.is_finite() && (0.0..=1.0).contains(strength)
        }
        EffectPass::Pixel { entry_point, .. } => pixel_op(entry_point).is_some_and(|op| {
            let values = pass.values();
            values.len() == op.len && values.iter().all(|v| v.is_finite()) && (op.valid)(&values)
        }),
        EffectPass::Sample { entry_point, .. } => sample_op(entry_point).is_some_and(|op| {
            let values = pass.values();
            values.len() == op.len
                && values.iter().all(|v| v.is_finite())
                && (op.valid)(&values, width, height)
        }),
        EffectPass::Convolve {
            entry_point,
            weights,
            ..
        } => {
            op(entry_point).is_some_and(|op| matches!(op.kernel, Kernel::Convolve))
                && !weights.is_empty()
                && weights.len() <= 49
                && !weights.len().is_multiple_of(2)
                && weights.iter().all(|w| w.is_finite() && *w >= 0.0)
                && (weights.iter().sum::<f32>() - 1.0).abs() <= 1e-5
        }
    };
    if valid {
        Ok(())
    } else {
        Err(Error::Unsupported("effect pass"))
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "resolved values stay Copy and inline for the per-frame parameter map; boxing the curve would allocate on every effect evaluation"
)]
#[derive(Clone, Copy)]
enum ResolvedValue {
    Scalar(f64),
    Choice(u32),
    Bool(bool),
    Color([f64; 4]),
    Point([f64; 2]),
    Lut(Option<project::LutId>),
    Curve(project::ToneCurve),
}
impl From<EffectValue> for ResolvedValue {
    fn from(value: EffectValue) -> Self {
        match value {
            EffectValue::Scalar(v) => Self::Scalar(v),
            EffectValue::Choice(v) => Self::Choice(v),
            EffectValue::Bool(v) => Self::Bool(v),
            EffectValue::Color(v) => Self::Color(v.0.map(|v| f64::from(v) / 255.0)),
            EffectValue::Point(v) => Self::Point(v),
            EffectValue::Lut(v) => Self::Lut(v),
            EffectValue::Curve(v) => Self::Curve(v),
        }
    }
}

/// What an evaluator sees: its parameters at item-local time (keyframes
/// resolved, scalars clamped to the definition's hard range), the item-local
/// time itself for time-driven effects, the output size and the quality the
/// renderer asks for. Second-track inputs are not part of a frame.
pub struct EffectFrame {
    pub local: Tick,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    /// True for export and every strict production frame, false only for interactive
    /// preview. A coarse evaluation may lower sample counts, iteration counts or the
    /// number of scales; it never changes geometry, colour mapping, parameter meaning
    /// or which pixels are affected, and its output stays close to the fine one.
    pub fine: bool,
    params: BTreeMap<&'static str, ResolvedValue>,
}
impl EffectFrame {
    /// Resolve parameters at real item-local ticks; a content remap never changes this clock.
    /// Missing paths use static values; present zero-key composites keep their own bases.
    /// Wrong animation kinds/nonfinite samples refuse. Finite scalar/colour overshoot clamps.
    pub fn new(
        animations: &Animations,
        effect: &EffectInstance,
        local: Tick,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let definition = effect.definition()?;
        let mut params = BTreeMap::new();
        for param in definition.params {
            let stored = effect
                .value(param.id)
                .ok_or(Error::Invalid("effect parameter"))?;
            param.check(stored)?;
            let path = param
                .animatable
                .then(|| effect.param_path(param.id))
                .transpose()?;
            let animated = path
                .as_ref()
                .and_then(|path| animations.get(path).map(|p| (path, p)));
            let value = match (param.kind, animated) {
                (_, None) => ResolvedValue::from(*stored),
                (
                    EffectParamKind::Float { .. } | EffectParamKind::Angle,
                    Some((path, Param::Value(channel))),
                ) if matches!(channel.base(), ParamValue::Scalar(_)) => {
                    let Some(ParamValue::Scalar(v)) = animations.value_at(path, local)? else {
                        return Err(Error::Invalid("effect animation type"));
                    };
                    if !v.is_finite() {
                        return Err(Error::Invalid("effect animation value"));
                    }
                    let (min, max) = param
                        .scalar_range()
                        .ok_or(Error::Invalid("effect scalar range"))?;
                    ResolvedValue::Scalar(v.clamp(min, max))
                }
                (EffectParamKind::Color, Some((path, Param::Color(_)))) => {
                    let Some(ParamValue::Color(mut rgba)) = animations.value_at(path, local)?
                    else {
                        return Err(Error::Invalid("effect animation type"));
                    };
                    if rgba.iter().any(|v| !v.is_finite()) {
                        return Err(Error::Invalid("effect animation colour"));
                    }
                    for v in &mut rgba[..3] {
                        // The shared transfer is f32; only sampled RGB crosses this boundary.
                        *v = f64::from(crate::image::oetf(v.clamp(0.0, 1.0) as f32, 13));
                    }
                    rgba[3] = rgba[3].clamp(0.0, 1.0);
                    ResolvedValue::Color(rgba)
                }
                _ => return Err(Error::Invalid("effect animation type")),
            };
            params.insert(param.id, value);
        }
        Ok(Self {
            local,
            seconds: local.seconds(),
            width,
            height,
            fine: true,
            params,
        })
    }
    fn get(&self, id: &str) -> Result<&ResolvedValue> {
        self.params
            .get(id)
            .ok_or(Error::Invalid("effect parameter"))
    }
    pub fn scalar(&self, id: &str) -> Result<f64> {
        match self.get(id)? {
            ResolvedValue::Scalar(v) => Ok(*v),
            _ => Err(Error::Invalid("effect parameter")),
        }
    }
    pub fn choice(&self, id: &str) -> Result<u32> {
        match self.get(id)? {
            ResolvedValue::Choice(v) => Ok(*v),
            _ => Err(Error::Invalid("effect parameter")),
        }
    }
    pub fn flag(&self, id: &str) -> Result<bool> {
        match self.get(id)? {
            ResolvedValue::Bool(v) => Ok(*v),
            _ => Err(Error::Invalid("effect parameter")),
        }
    }
    /// Straight encoded sRGB RGBA, each component in 0..=1; wrong kind/id refuses.
    /// Animated RGB uses the shared f32 transfer once, without byte quantization.
    pub fn color(&self, id: &str) -> Result<[f64; 4]> {
        match self.get(id)? {
            ResolvedValue::Color(v) => Ok(*v),
            _ => Err(Error::Invalid("effect parameter")),
        }
    }
    pub fn point(&self, id: &str) -> Result<[f64; 2]> {
        match self.get(id)? {
            ResolvedValue::Point(v) => Ok(*v),
            _ => Err(Error::Invalid("effect parameter")),
        }
    }
    pub fn curve(&self, id: &str) -> Result<project::ToneCurve> {
        match self.get(id)? {
            ResolvedValue::Curve(v) => Ok(*v),
            _ => Err(Error::Invalid("effect parameter")),
        }
    }
}

/// Geometry of the canvas layer before the item's later transform.
/// Gains are working pixels per full-resolution clip pixel on each axis.
/// Invalid dimensions/area or nonpositive/nonfinite gains return Invalid.
#[derive(Clone, Copy, Debug)]
pub struct EffectGeometry {
    working: [u32; 2],
    clip_pixel_scale: [f64; 2],
}
impl EffectGeometry {
    pub fn new(working: [u32; 2], clip_pixel_scale: [f64; 2]) -> Result<Self> {
        Image::bytes(working[0], working[1])?;
        if clip_pixel_scale.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(Error::Invalid("effect spatial geometry"));
        }
        Ok(Self {
            working,
            clip_pixel_scale,
        })
    }
    pub fn working_size(&self) -> [u32; 2] {
        self.working
    }
    pub fn clip_pixel_scale(&self) -> [f64; 2] {
        self.clip_pixel_scale
    }
}

pub type EffectEvaluator = fn(&EffectFrame, &EffectGeometry) -> Result<Vec<EffectPass>>;

/// Remap the evolving content time (item-local ticks) in stack order. The frame's
/// `local`/`seconds` and resolved parameters always use the real item-local clock.
/// Returns item-local ticks, or a typed parameter/time failure; never resets time
/// on failure. Only content is remapped, independently of transform and audio.
pub type ContentTime = fn(&EffectFrame, Tick) -> Result<Tick>;

/// The evaluator of an effect kind; a kind without one is unsupported.
pub fn evaluator(kind: &str) -> Option<EffectEvaluator> {
    Some(match kind {
        "blur" => blur,
        "directional_blur" => streak_blur,
        "color_grade" => color_grade,
        "chroma_key" => chroma_key,
        "sharpen" => sharpen,
        "unsharp_mask" => spatial::detail_boost_eval,
        "bokeh_blur" => spatial::lens_blur_eval,
        "mirror" => spatial::mirror_eval,
        "offset" => spatial::wrap_shift_eval,
        "black_white" => black_white,
        "channel_mix" => channel_mix,
        "color_pass" => color_pass,
        "color_replace" => color_replace,
        "extract" => extract,
        "find_edges" => edge_lines,
        "color_emboss" => colour_relief,
        "gamma_correction" => gamma_correction,
        "invert" => invert,
        "levels" => levels,
        "mosaic" => mosaic,
        "posterize" => posterize,
        "strobe" => strobe,
        "tint" => tint,
        "hold_time" => hold_time,
        "twirl" => whole_image::twirl_eval,
        "spherize" => whole_image::spherize_eval,
        "magnify" => whole_image::magnify_eval,
        "wave_warp" => whole_image::wave_eval,
        "lens_distortion" => whole_image::lens_eval,
        "turbulent_displace" => whole_image::turbulent_eval,
        "corner_pin" => whole_image::corner_eval,
        "basic_3d" => whole_image::basic_3d_eval,
        "brush_strokes" => whole_image::brush_eval,
        "roughen_edges" => whole_image::roughen_eval,
        "camera_shake" => whole_image::shake_eval,
        "posterize_time" => whole_image::posterize_time_passes,
        "echo_glow" => whole_image::echo_glow_eval,
        "edge_glow" => whole_image::edge_glow_eval,
        "glint" => whole_image::glint_eval,
        "wonder_glow" => whole_image::wonder_glow_eval,
        "volumetric_rays" => whole_image::radial_eval,
        "light_leaks" => whole_image::leaks_eval,
        "lens_flare" => whole_image::flare_eval,
        "rgb_split" => whole_image::split_eval,
        "drop_shadow" => whole_image::drop_eval,
        "long_shadow" => whole_image::long_eval,
        "gradient" => whole_image::gradient_eval,
        "four_color_gradient" => whole_image::four_eval,
        "color_key" => whole_image::color_key_eval,
        "luma_key" => whole_image::luma_key_eval,
        _ => return None,
    })
}

/// The content-time remap of an effect kind; a kind without one samples the
/// content at the frame's own time.
pub fn content_remap(kind: &str) -> Option<ContentTime> {
    Some(match kind {
        "hold_time" => hold_time_content,
        "posterize_time" => whole_image::posterize_time_content,
        _ => return None,
    })
}

/// One pixel op reading flat uniforms.
fn pixel(entry_point: &'static str, uniforms: Vec<Uniform>) -> EffectPass {
    EffectPass::Pixel {
        entry_point,
        uniforms,
    }
}
/// A resolved straight sRGB colour; alpha is intentionally unused by these RGB ops.
fn color_uniform([r, g, b, _]: [f64; 4]) -> Uniform {
    Uniform::Color([r as f32, g as f32, b as f32])
}

fn blur(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    blur_passes(frame.scalar("intensity")?, frame.width, frame.height)
}
/// Sharpen is an unsharp mask: a gaussian of one clip pixel, at least .6 output
/// pixels, subtracted from the immutable input and added back by the authored
/// gain. The source's uniform scale is the equal-gain case; unequal gains take
/// the smaller axis like Blur, so the kernel stays within the tighter clip pixel.
fn sharpen(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    if geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    let amount = frame.scalar("amount")?;
    if amount == 0.0 {
        return Ok(Vec::new());
    }
    let [gx, gy] = geometry.clip_pixel_scale();
    let sigma = gx.min(gy).max(0.6) as f32;
    let mut passes = sigma_passes(sigma, frame.width, frame.height)?;
    passes.push(EffectPass::Sample {
        entry_point: "detail_boost",
        uniforms: vec![
            Uniform::Float((amount / 100.0) as f32),
            // Sharpen has no threshold and keeps every difference.
            Uniform::Float(0.0),
        ],
    });
    Ok(passes)
}
/// Basic grade, the shared tone set, sharpen or soften, lift, gamma, gain, RGB
/// curves, the luma curve, HSL bands, vignette and grain. A LUT follows the
/// whole instance in [`evaluate_effect`].
fn color_grade(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let mut passes = grade_passes(
        frame.scalar("brightness")?,
        frame.scalar("contrast")?,
        frame.scalar("saturation")?,
        frame.scalar("hue")?,
    )?;
    passes.extend(tone_passes(
        frame.scalar("temperature")?,
        frame.scalar("highlight")?,
        frame.scalar("shadow")?,
        frame.scalar("shine")?,
        frame.scalar("fade")?,
    )?);
    let sharpness = frame.scalar("sharpness")?;
    if sharpness != 0.0 {
        if geometry.working_size() != [frame.width, frame.height] {
            return Err(Error::Invalid("effect spatial geometry"));
        }
        let [gx, gy] = geometry.clip_pixel_scale();
        if sharpness > 0.0 {
            let sigma = gx.min(gy).max(0.6) as f32;
            passes.extend(sigma_passes(sigma, frame.width, frame.height)?);
            passes.push(EffectPass::Sample {
                entry_point: "detail_boost",
                uniforms: vec![
                    // 50 is one full unit of detail gain.
                    Uniform::Float((sharpness / 50.0) as f32),
                    Uniform::Float(0.0),
                ],
            });
        } else {
            // A negative control softens up to five output pixels.
            passes.extend(sigma_passes(
                (-sharpness / 10.0) as f32,
                frame.width,
                frame.height,
            )?);
        }
    }
    passes.extend(lift_passes(
        frame.scalar("lift_r")?,
        frame.scalar("lift_g")?,
        frame.scalar("lift_b")?,
    )?);
    passes.extend(gamma_passes(
        frame.scalar("gamma_r")?,
        frame.scalar("gamma_g")?,
        frame.scalar("gamma_b")?,
    )?);
    passes.extend(gain_passes(
        frame.scalar("gain_r")?,
        frame.scalar("gain_g")?,
        frame.scalar("gain_b")?,
    )?);
    for (id, entry) in [
        ("curve_red", "curve_red"),
        ("curve_green", "curve_green"),
        ("curve_blue", "curve_blue"),
        ("curve_luma", "curve_luma"),
    ] {
        let curve = frame.curve(id)?;
        if !curve.is_identity() {
            passes.push(pixel(entry, curve_uniforms(&curve)));
        }
    }
    let mut bands = [0.0; HSL_BAND_UNIFORMS];
    let keys = project::HSL_BANDS.iter().flat_map(|band| {
        project::HSL_CONTROLS
            .iter()
            .map(move |control| format!("hsl_{band}_{control}"))
    });
    for (slot, key) in bands.iter_mut().zip(keys) {
        *slot = frame.scalar(&key)?;
    }
    passes.extend(hsl_bands_passes(&bands)?);
    let vignette = frame.scalar("vignette")?;
    let grain = frame.scalar("grain")?;
    if (vignette != 0.0 || grain != 0.0) && geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    if vignette != 0.0 {
        passes.push(EffectPass::Sample {
            entry_point: "vignette",
            uniforms: vec![Uniform::Float((vignette / 50.0) as f32)],
        });
    }
    if grain != 0.0 {
        passes.push(EffectPass::Sample {
            entry_point: "grain",
            uniforms: vec![Uniform::Float((grain / 50.0) as f32)],
        });
    }
    Ok(passes)
}
fn chroma_key(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    chroma_key_passes(
        frame.color("key_color")?,
        frame.scalar("intensity")?,
        frame.scalar("shadow")?,
    )
}
/// Test kind paired with project's `test-kinds` feature: holds the item's content
/// at `rate` frames per second, as Low Frame Rate does.
fn hold_time_content(frame: &EffectFrame, content: Tick) -> Result<Tick> {
    let rate = frame.scalar("rate")?;
    let held = (content.seconds() * rate).floor() / rate;
    Ok(Tick::from_seconds_f64(held, Rounding::Floor)?)
}
/// The test kind's evaluator: an identity grade whose hue uniform carries the
/// quality flag, so a test can see which frame the renderer built.
fn hold_time(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![EffectPass::Pixel {
        entry_point: "grade",
        uniforms: [1.0, 1.0, 1.0, if frame.fine { 0.0 } else { 1e-3 }]
            .map(Uniform::Float)
            .to_vec(),
    }])
}

fn black_white(_: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel("black_white", Vec::new())])
}
fn channel_mix(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    const GAINS: [&str; 12] = [
        "rr", "rg", "rb", "rc", "gr", "gg", "gb", "gc", "br", "bg", "bb", "bc",
    ];
    const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut values = [0.0f32; 12];
    for (value, id) in values.iter_mut().zip(GAINS) {
        *value = (frame.scalar(id)? / 100.0) as f32;
    }
    if values == IDENTITY {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "channel_mix",
        values.map(Uniform::Float).to_vec(),
    )])
}
fn color_pass(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "color_pass",
        vec![
            color_uniform(frame.color("color")?),
            Uniform::Float((frame.scalar("similarity")? / 100.0) as f32),
            Uniform::Float(if frame.flag("reverse")? { 1.0 } else { 0.0 }),
        ],
    )])
}
fn color_replace(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "color_replace",
        vec![
            Uniform::Float((frame.scalar("similarity")? / 100.0 * 1.2) as f32),
            Uniform::Float(if frame.flag("solid")? { 1.0 } else { 0.0 }),
            color_uniform(frame.color("target")?),
            color_uniform(frame.color("replace")?),
        ],
    )])
}
fn extract(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let softness = frame.scalar("softness")?;
    Ok(vec![pixel(
        "extract",
        vec![
            Uniform::Float((frame.scalar("black")? / 255.0) as f32),
            Uniform::Float((frame.scalar("white")? / 255.0) as f32),
            // Zero softness is a hard cutoff; a tiny fade keeps the feathered
            // branch's limiting value without turning the edge grey.
            Uniform::Float(if softness <= 0.0 {
                0.0
            } else {
                (softness / 100.0 * 0.2 + 1e-4) as f32
            }),
            Uniform::Float(if frame.flag("invert")? { 1.0 } else { 0.0 }),
        ],
    )])
}
fn gamma_correction(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let gamma = (frame.scalar("gamma")? / 10.0) as f32;
    if (gamma - 1.0).abs() < 1e-6 {
        return Ok(Vec::new());
    }
    Ok(vec![pixel("gamma", vec![Uniform::Float(gamma)])])
}
fn invert(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "invert",
        vec![
            Uniform::Float(frame.choice("channel")? as f32),
            Uniform::Float((frame.scalar("blend")? / 100.0) as f32),
        ],
    )])
}
fn levels(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let in_black = frame.scalar("in_black")?;
    let in_white = frame.scalar("in_white")?;
    let out_black = frame.scalar("out_black")?;
    let out_white = frame.scalar("out_white")?;
    let gamma = frame.scalar("gamma")?;
    if in_black == 0.0
        && in_white == 255.0
        && out_black == 0.0
        && out_white == 255.0
        && gamma == 100.0
    {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "levels",
        vec![
            Uniform::Float((in_black / 255.0) as f32),
            Uniform::Float((in_white / 255.0) as f32),
            Uniform::Float((out_black / 255.0) as f32),
            Uniform::Float((out_white / 255.0) as f32),
            Uniform::Float((100.0 / gamma.max(1.0)) as f32),
        ],
    )])
}
/// Pixelate counts blocks per axis, so its block size is the output axis over the
/// truncated count, at least one pixel. Both axes covering one pixel per block is
/// an exact identity. The sharp flag forces nearest tile colours.
fn mosaic(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    if geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    let bw = (f64::from(frame.width) / frame.scalar("horizontal")?.trunc().max(1.0)).max(1.0);
    let bh = (f64::from(frame.height) / frame.scalar("vertical")?.trunc().max(1.0)).max(1.0);
    if bw == 1.0 && bh == 1.0 {
        return Ok(Vec::new());
    }
    let softness = if frame.flag("sharp")? {
        0.0
    } else {
        (frame.scalar("softness")? / 100.0) as f32
    };
    Ok(vec![EffectPass::Sample {
        entry_point: "pixelate",
        uniforms: vec![
            Uniform::Float(bw as f32),
            Uniform::Float(bh as f32),
            Uniform::Float(softness),
        ],
    }])
}
fn posterize(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "posterize",
        vec![Uniform::Float(
            (frame.scalar("levels")?.max(2.0) - 1.0) as f32,
        )],
    )])
}
fn strobe(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let period = frame.scalar("period")?.max(0.001);
    let duration = frame.scalar("duration")?;
    if frame.seconds.rem_euclid(period) >= duration {
        return Ok(Vec::new());
    }
    Ok(vec![pixel(
        "strobe",
        vec![
            color_uniform(frame.color("color")?),
            Uniform::Float((frame.scalar("blend")? / 100.0) as f32),
        ],
    )])
}
fn tint(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "tint",
        vec![
            color_uniform(frame.color("black")?),
            color_uniform(frame.color("white")?),
            Uniform::Float((frame.scalar("amount")? / 100.0) as f32),
        ],
    )])
}

/// The item-local time at which `item`'s content is sampled after its enabled
/// effects' content-time remaps, applied in stack order. Each remap resolves its
/// parameters at the real item-local time, while remapping the evolving content
/// time from the previous remap. Both coordinates are ticks. Only
/// the content is held: transform, crop, opacity and effects keep the real time.
/// Without a remap the time is returned unchanged, so handles outside the item's
/// extent keep sampling exactly; a remapped time is clamped into the item's range.
pub fn content_time(item: &Item, local: Tick, width: u32, height: u32) -> Result<Tick> {
    let mut at = local;
    let mut remapped = false;
    for effect in item.effects.iter().filter(|effect| effect.enabled) {
        let Some(remap) = content_remap(&effect.kind) else {
            continue;
        };
        let frame = EffectFrame::new(&item.animations, effect, local, width, height)?;
        at = remap(&frame, at)?;
        remapped = true;
    }
    if remapped {
        Ok(at.clamp(Tick::ZERO, item.duration()?))
    } else {
        Ok(local)
    }
}

/// The passes of one effect instance at item-local time `local`. `fine` is false
/// only for interactive preview; see [`EffectFrame::fine`].
pub fn evaluate_effect(
    animations: &Animations,
    effect: &EffectInstance,
    local: Tick,
    geometry: EffectGeometry,
    fine: bool,
    resources: &lut::LutResources,
) -> Result<Vec<EffectPass>> {
    let evaluate = evaluator(&effect.kind).ok_or(Error::Unsupported("effect kind"))?;
    let [width, height] = geometry.working_size();
    let mut frame = EffectFrame::new(animations, effect, local, width, height)?;
    frame.fine = fine;
    let mut passes = evaluate(&frame, &geometry)?;
    if effect.kind == "color_grade"
        && frame.scalar("lut_strength")? != 0.
        && let Some(ResolvedValue::Lut(Some(id))) = frame.params.get("lut")
    {
        let table = resources.get(*id)?;
        if !table.is_identity() {
            passes.push(EffectPass::Lut3d {
                table,
                strength: (frame.scalar("lut_strength")? / 100.) as f32,
            });
        }
    }
    Ok(passes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod effect_run_tests {
    use super::*;
    use crate::MemoryKind;
    use std::sync::Mutex;
    struct Hooks;
    struct Lease;
    impl crate::Reservation for Lease {}
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn reserve(&self, _: MemoryKind, _: u64) -> Result<Box<dyn crate::Reservation>> {
            Ok(Box::new(Lease))
        }
    }
    struct Split {
        mirror: bool,
        retry: bool,
        releases: std::sync::atomic::AtomicUsize,
        calls: Mutex<Vec<&'static str>>,
    }
    impl EffectCompositor for Split {
        fn frame<'a>(
            &'a self,
            _: &'a dyn crate::ExecutionHooks,
        ) -> crate::Result<Option<Box<dyn crate::effects::EffectCompositor + 'a>>> {
            Ok(None)
        }
        fn release_idle(&self) -> crate::Result<()> {
            self.releases
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn supports(&self, entry: &str) -> bool {
            (entry == "mirror") == self.mirror
        }
        fn apply(
            &self,
            current: &Image,
            passes: &[EffectPass],
            inputs: EffectInputs<'_>,
            hooks: &dyn ExecutionHooks,
        ) -> Result<Image> {
            self.calls
                .lock()
                .unwrap()
                .extend(passes.iter().map(EffectPass::entry_point));
            if passes[0].entry_point() == "mirror" {
                assert_ne!(
                    current.pixels(),
                    inputs.input.pixels(),
                    "prefix must visibly modify current"
                );
            }
            if self.retry {
                return Err(Error::Unsupported("test retry"));
            }
            CpuEffects.apply(current, passes, inputs, hooks)
        }
    }
    #[test]
    fn instance_input_survives_both_run_boundaries_retry_and_successive_instances() {
        let source = Image::from_working_pixels(
            8,
            4,
            (0..32).map(|i| [((i % 7) + 1) as f32 / 16.0, 0.1, 0.2, 1.0]),
            &Hooks,
        )
        .unwrap();
        let mut passes = grade_passes(-50.0, 0.0, 0.0, 0.0).unwrap();
        passes.push(EffectPass::Sample {
            entry_point: "mirror",
            uniforms: [4.0, 2.0, 1.0, 0.0].map(Uniform::Float).to_vec(),
        });
        let reference = |current: &Image| {
            CpuEffects
                .apply(
                    current,
                    &passes,
                    EffectInputs {
                        input: current,
                        coverage: None,
                        below: &source,
                    },
                    &Hooks,
                )
                .unwrap()
        };
        let once = reference(&source);
        let twice = reference(&once);
        for (mirror, retry) in [(false, false), (true, false), (true, true)] {
            let adapter = Split {
                mirror,
                retry,
                releases: Default::default(),
                calls: Mutex::new(vec![]),
            };
            let first = apply_effect(
                source.copy(&Hooks).unwrap(),
                &passes,
                &source,
                &adapter,
                &Hooks,
            )
            .unwrap();
            assert_eq!(first.pixels(), once.pixels());
            let second = apply_effect(first, &passes, &source, &adapter, &Hooks).unwrap();
            assert_eq!(second.pixels(), twice.pixels());
            assert_eq!(
                adapter.releases.load(std::sync::atomic::Ordering::SeqCst),
                if retry { 4 } else { 2 }
            );
            let expected = if mirror { "mirror" } else { "grade" };
            assert_eq!(*adapter.calls.lock().unwrap(), [expected, expected]);
        }
    }
}
