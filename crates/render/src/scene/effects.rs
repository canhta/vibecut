//! Device-free evaluated passes, shared by CPU and app GPU adapters.
use crate::{Error, ExecutionHooks, Image, Result, check};
use project::{Animations, EffectInstance, EffectParamKind, EffectValue, Item, Param, ParamValue};
use std::{borrow::Cow, collections::BTreeMap};
use time::{Rounding, Tick};

mod image_ops;

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
            entry_point: "gaussian",
            horizontal,
            weights: weights.clone(),
        })
        .collect())
}
/// Linear-light mid grey that contrast pivots around.
pub const GRADE_PIVOT: f32 = 0.18;
/// BT.709 luma coefficients, also the key's luma for the shadow stage.
const KEY_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// BT.709 chroma scaling: 2*(1-Kb) and 2*(1-Kr) from the same coefficients.
const KEY_CB_SCALE: f32 = 1.8556;
const KEY_CR_SCALE: f32 = 1.5748;
/// Chroma-distance tolerance at intensity 0 and its span at intensity 100: the
/// implementation reference maps its 0..100 tolerance to `0.08 + 0.5*t`.
/// Intensity 0 emits no pass, matching the observed reference.
pub const KEY_TOLERANCE_MIN: f32 = 0.08;
pub const KEY_TOLERANCE_SPAN: f32 = 0.5;
/// Matte gain spans 1..3 across intensity, like the reference's
/// `1 + 2*transparency`.
pub const KEY_GAIN: f32 = 2.0;
/// The reference's shadow stage doubles the luma gap before adding it to the matte.
pub const KEY_SHADOW_GAIN: f32 = 2.0;
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
/// A pixel-local op: its shader entry, flat uniform count, uniform check and
/// the CPU reference the shader mirrors.
struct PixelOp {
    entry_point: &'static str,
    len: usize,
    valid: fn(&[f32]) -> bool,
    apply: fn([f32; 4], &[f32]) -> [f32; 4],
}
static PIXEL_OPS: [PixelOp; 13] = [
    PixelOp {
        entry_point: "grade",
        len: 4,
        valid: |v| v.get(1).is_some_and(|exponent| *exponent > 0.0),
        apply: grade_pixel,
    },
    PixelOp {
        entry_point: "chroma_key",
        len: 5,
        valid: |v| v.iter().all(|c| (0.0..=1.0).contains(c)),
        apply: chroma_key_pixel,
    },
    PixelOp {
        entry_point: "black_white",
        len: 0,
        valid: |_| true,
        apply: black_white_pixel,
    },
    PixelOp {
        entry_point: "channel_mix",
        len: 12,
        valid: |v| v.iter().all(|g| (-2.0..=2.0).contains(g)),
        apply: channel_mix_pixel,
    },
    PixelOp {
        entry_point: "color_pass",
        len: 5,
        valid: |v| {
            v.get(..3)
                .is_some_and(|key| key.iter().all(|c| (0.0..=1.0).contains(c)))
                && v.get(3).is_some_and(|sim| (0.0..=1.2).contains(sim))
                && v.get(4)
                    .is_some_and(|reverse| (0.0..=1.0).contains(reverse))
        },
        apply: color_pass_pixel,
    },
    PixelOp {
        entry_point: "color_replace",
        len: 8,
        valid: |v| {
            v.get(0).is_some_and(|sim| (0.0..=1.2).contains(sim))
                && v.get(1).is_some_and(|solid| (0.0..=1.0).contains(solid))
                && v.get(2..8)
                    .is_some_and(|c| c.iter().all(|v| (0.0..=1.0).contains(v)))
        },
        apply: color_replace_pixel,
    },
    PixelOp {
        entry_point: "extract",
        len: 4,
        valid: |v| {
            v.get(0).is_some_and(|lo| (0.0..=1.0).contains(lo))
                && v.get(1).is_some_and(|hi| (0.0..=1.0).contains(hi))
                && v.get(2).is_some_and(|soft| (0.0..=0.2001).contains(soft))
                && v.get(3).is_some_and(|invert| (0.0..=1.0).contains(invert))
        },
        apply: extract_pixel,
    },
    PixelOp {
        entry_point: "gamma",
        len: 1,
        valid: |v| v.first().is_some_and(|g| *g > 0.0 && *g <= 4.0),
        apply: gamma_pixel,
    },
    PixelOp {
        entry_point: "invert",
        len: 2,
        valid: |v| {
            v.first()
                .is_some_and(|channel| (0.0..=4.0).contains(channel))
                && v.get(1).is_some_and(|blend| (0.0..=1.0).contains(blend))
        },
        apply: invert_pixel,
    },
    PixelOp {
        entry_point: "levels",
        len: 5,
        valid: |v| {
            v.get(0..4)
                .is_some_and(|bounds| bounds.iter().all(|b| (0.0..=1.0).contains(b)))
                && v.get(4).is_some_and(|gamma| *gamma > 0.0 && *gamma <= 12.0)
        },
        apply: levels_pixel,
    },
    PixelOp {
        entry_point: "posterize",
        len: 1,
        valid: |v| v.first().is_some_and(|n| *n >= 1.0 && n.is_finite()),
        apply: posterize_pixel,
    },
    PixelOp {
        entry_point: "strobe",
        len: 4,
        valid: |v| {
            v.get(0..3)
                .is_some_and(|c| c.iter().all(|v| (0.0..=1.0).contains(v)))
                && v.get(3).is_some_and(|blend| (0.0..=1.0).contains(blend))
        },
        apply: strobe_pixel,
    },
    PixelOp {
        entry_point: "tint",
        len: 7,
        valid: |v| {
            v.get(0..6)
                .is_some_and(|c| c.iter().all(|v| (0.0..=1.0).contains(v)))
                && v.get(6).is_some_and(|amount| (0.0..=1.0).contains(amount))
        },
        apply: tint_pixel,
    },
];

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
pub fn black_white_pixel(pixel: [f32; 4], _: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    premultiply([luma(c); 3], alpha)
}
/// Encoded-space channel inversion mixed back by `blend`; the Alpha channel
/// inverts coverage instead.
pub fn invert_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
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
    let mixed = [0, 1, 2].map(|k| {
        if channel == 0 || channel == k as u32 + 1 {
            inverted[k] + (e[k] - inverted[k]) * blend
        } else {
            e[k]
        }
    });
    premultiply(decoded_color(mixed), alpha)
}
/// Encoded-space quantisation into `n + 1` levels.
pub fn posterize_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let n = uniform(values, 0).max(1.0);
    let (c, alpha) = straight(pixel);
    let quantized = encoded_color(c).map(|v| (v * n).round() / n);
    premultiply(decoded_color(quantized), alpha)
}
/// Encoded-space input/output remap with a gamma power.
pub fn levels_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
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
pub fn gamma_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let gamma = uniform(values, 0).max(1e-3);
    let (c, alpha) = straight(pixel);
    let mapped = encoded_color(c).map(|v| v.max(0.0).powf(gamma));
    premultiply(decoded_color(mapped), alpha)
}
/// Encoded luma ramps between the two mapped colours, then mixes by amount.
pub fn tint_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let black = uniform_color(values, 0);
    let white = uniform_color(values, 3);
    let amount = uniform(values, 6).clamp(0.0, 1.0);
    let (c, alpha) = straight(pixel);
    let l = crate::image::oetf(luma(c).max(0.0), 13);
    let target = [0, 1, 2].map(|k| black[k] + (white[k] - black[k]) * l);
    let e = encoded_color(c);
    let mixed = [0, 1, 2].map(|k| e[k] + (target[k] - e[k]) * amount);
    premultiply(decoded_color(mixed), alpha)
}
/// Keeps colours within `similarity` of the key and greys the rest.
pub fn color_pass_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
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
pub fn color_replace_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
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
    let mixed = [0, 1, 2].map(|k| e[k] + (to[k] - e[k]) * weight);
    premultiply(decoded_color(mixed), alpha)
}
/// Encoded 3x4 channel matrix; the last value of each row is a constant.
pub fn channel_mix_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let (c, alpha) = straight(pixel);
    let e = encoded_color(c);
    let mixed = [0, 1, 2].map(|k| {
        let row = k * 4;
        (uniform(values, row) * e[0]
            + uniform(values, row + 1) * e[1]
            + uniform(values, row + 2) * e[2]
            + uniform(values, row + 3))
        .clamp(0.0, 1.0)
    });
    premultiply(decoded_color(mixed), alpha)
}
/// Black-and-white threshold on encoded luma with an optional feathered band.
pub fn extract_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
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
pub fn strobe_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let strobe = decoded_color(uniform_color(values, 0));
    let blend = uniform(values, 3).clamp(0.0, 1.0);
    let (c, alpha) = straight(pixel);
    let mixed = [0, 1, 2].map(|k| strobe[k] + (c[k] - strobe[k]) * blend);
    premultiply(mixed, alpha)
}
fn pixel_op(entry_point: &str) -> Option<&'static PixelOp> {
    PIXEL_OPS.iter().find(|op| op.entry_point == entry_point)
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
    #[allow(dead_code, reason = "first used by the 264 and 265 ops")]
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
/// A sample op: entry name, flat uniform count, uniform check and the CPU reference
/// a shader entry mirrors (one output pixel from its coordinate, the uniforms and
/// [`SampleInputs`]).
struct SampleOp {
    entry_point: &'static str,
    len: usize,
    valid: fn(&[f32], u32, u32) -> bool,
    apply: fn(&SampleInputs<'_>, u32, u32, &[f32]) -> [f32; 4],
}
static SAMPLE_OPS: [SampleOp; 1] = [SampleOp {
    entry_point: "mirror",
    len: 4,
    valid: mirror_uniforms,
    apply: mirror,
}];
fn sample_op(entry_point: &str) -> Option<&'static SampleOp> {
    SAMPLE_OPS.iter().find(|op| op.entry_point == entry_point)
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
            *entry_point == "gaussian"
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

#[derive(Clone, Copy)]
enum ResolvedValue {
    Scalar(f64),
    Choice(u32),
    Bool(bool),
    Color([f64; 4]),
    Point([f64; 2]),
}
impl From<EffectValue> for ResolvedValue {
    fn from(value: EffectValue) -> Self {
        match value {
            EffectValue::Scalar(v) => Self::Scalar(v),
            EffectValue::Choice(v) => Self::Choice(v),
            EffectValue::Bool(v) => Self::Bool(v),
            EffectValue::Color(v) => Self::Color(v.0.map(|v| f64::from(v) / 255.0)),
            EffectValue::Point(v) => Self::Point(v),
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
}

pub type EffectEvaluator = fn(&EffectFrame) -> Result<Vec<EffectPass>>;

/// Remap the evolving content time (item-local ticks) in stack order. The frame's
/// `local`/`seconds` and resolved parameters always use the real item-local clock.
/// Returns item-local ticks, or a typed parameter/time failure; never resets time
/// on failure. Only content is remapped, independently of transform and audio.
pub type ContentTime = fn(&EffectFrame, Tick) -> Result<Tick>;

/// The evaluator of an effect kind; a kind without one is unsupported.
pub fn evaluator(kind: &str) -> Option<EffectEvaluator> {
    Some(match kind {
        "blur" => blur,
        "color_grade" => color_grade,
        "chroma_key" => chroma_key,
        "black_white" => black_white,
        "channel_mix" => channel_mix,
        "color_pass" => color_pass,
        "color_replace" => color_replace,
        "extract" => extract,
        "gamma_correction" => gamma_correction,
        "invert" => invert,
        "levels" => levels,
        "posterize" => posterize,
        "strobe" => strobe,
        "tint" => tint,
        "hold_time" => hold_time,
        _ => return None,
    })
}

/// The content-time remap of an effect kind; a kind without one samples the
/// content at the frame's own time.
pub fn content_remap(kind: &str) -> Option<ContentTime> {
    Some(match kind {
        "hold_time" => hold_time_content,
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

fn blur(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    blur_passes(frame.scalar("intensity")?, frame.width, frame.height)
}
fn color_grade(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    grade_passes(
        frame.scalar("brightness")?,
        frame.scalar("contrast")?,
        frame.scalar("saturation")?,
        frame.scalar("hue")?,
    )
}
fn chroma_key(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
fn hold_time(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    Ok(vec![EffectPass::Pixel {
        entry_point: "grade",
        uniforms: [1.0, 1.0, 1.0, if frame.fine { 0.0 } else { 1e-3 }]
            .map(Uniform::Float)
            .to_vec(),
    }])
}

fn black_white(_: &EffectFrame) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel("black_white", Vec::new())])
}
fn channel_mix(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
fn color_pass(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "color_pass",
        vec![
            color_uniform(frame.color("color")?),
            Uniform::Float((frame.scalar("similarity")? / 100.0) as f32),
            Uniform::Float(if frame.flag("reverse")? { 1.0 } else { 0.0 }),
        ],
    )])
}
fn color_replace(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
fn extract(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
fn gamma_correction(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    let gamma = (frame.scalar("gamma")? / 10.0) as f32;
    if (gamma - 1.0).abs() < 1e-6 {
        return Ok(Vec::new());
    }
    Ok(vec![pixel("gamma", vec![Uniform::Float(gamma)])])
}
fn invert(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "invert",
        vec![
            Uniform::Float(frame.choice("channel")? as f32),
            Uniform::Float((frame.scalar("blend")? / 100.0) as f32),
        ],
    )])
}
fn levels(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
fn posterize(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
    Ok(vec![pixel(
        "posterize",
        vec![Uniform::Float(
            (frame.scalar("levels")?.max(2.0) - 1.0) as f32,
        )],
    )])
}
fn strobe(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
fn tint(frame: &EffectFrame) -> Result<Vec<EffectPass>> {
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
    width: u32,
    height: u32,
    fine: bool,
) -> Result<Vec<EffectPass>> {
    let evaluate = evaluator(&effect.kind).ok_or(Error::Unsupported("effect kind"))?;
    let mut frame = EffectFrame::new(animations, effect, local, width, height)?;
    frame.fine = fine;
    evaluate(&frame)
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
