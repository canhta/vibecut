//! Device-free evaluated passes, shared by CPU and app GPU adapters.
use crate::{Error, ExecutionHooks, Image, Result, check};
use project::{Animations, EffectInstance, EffectValue, Item, ParamValue};
use std::{borrow::Cow, collections::BTreeMap};
use time::Tick;

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
}
impl EffectPass {
    pub const fn entry_point(&self) -> &'static str {
        match self {
            Self::Convolve { entry_point, .. } | Self::Pixel { entry_point, .. } => entry_point,
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
            Self::Pixel { uniforms, .. } => {
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
pub fn chroma_key_passes(
    key_color: project::Color,
    intensity: f64,
    shadow: f64,
) -> Result<Vec<EffectPass>> {
    let range = project::CHROMA_KEY_MIN..=project::CHROMA_KEY_MAX;
    if [intensity, shadow]
        .iter()
        .any(|v| !v.is_finite() || !range.contains(v))
    {
        return Err(Error::Invalid("chroma key control"));
    }
    if intensity == 0.0 {
        return Ok(Vec::new());
    }
    // Project colours are straight sRGB bytes; the pass compares sRGB-encoded
    // pixels, so the key needs no transfer conversion here.
    let encoded = |byte: u8| f32::from(byte) / 255.0;
    Ok(vec![EffectPass::Pixel {
        entry_point: "chroma_key",
        uniforms: vec![
            Uniform::Color([
                encoded(key_color.0[0]),
                encoded(key_color.0[1]),
                encoded(key_color.0[2]),
            ]),
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
static PIXEL_OPS: [PixelOp; 2] = [
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
];
fn pixel_op(entry_point: &str) -> Option<&'static PixelOp> {
    PIXEL_OPS.iter().find(|op| op.entry_point == entry_point)
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
            let (horizontal, weights) = match pass {
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
pub fn validate_pass(pass: &EffectPass) -> Result<()> {
    let valid = match pass {
        EffectPass::Pixel { entry_point, .. } => pixel_op(entry_point).is_some_and(|op| {
            let values = pass.values();
            values.len() == op.len && values.iter().all(|v| v.is_finite()) && (op.valid)(&values)
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

/// What an evaluator sees: its parameters at item-local time (keyframes
/// resolved, scalars clamped to the definition's hard range), the item-local
/// time itself for time-driven effects, and the output size. Second-track
/// inputs are not part of a frame.
pub struct EffectFrame {
    pub local: Tick,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    params: BTreeMap<&'static str, EffectValue>,
}
impl EffectFrame {
    /// Resolve `effect`'s parameters against `animations` at item-local time `local`.
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
            let value = match (param.animatable, param.scalar_range()) {
                (true, Some((min, max))) => {
                    match animations.value_at(&effect.param_path(param.id)?, local)? {
                        Some(ParamValue::Scalar(v)) => EffectValue::Scalar(v.clamp(min, max)),
                        Some(_) => return Err(Error::Invalid("effect animation type")),
                        None => *stored,
                    }
                }
                _ => *stored,
            };
            params.insert(param.id, value);
        }
        Ok(Self {
            local,
            seconds: local.seconds(),
            width,
            height,
            params,
        })
    }
    fn get(&self, id: &str) -> Result<&EffectValue> {
        self.params
            .get(id)
            .ok_or(Error::Invalid("effect parameter"))
    }
    pub fn scalar(&self, id: &str) -> Result<f64> {
        self.get(id)?
            .scalar()
            .ok_or(Error::Invalid("effect parameter"))
    }
    pub fn choice(&self, id: &str) -> Result<u32> {
        self.get(id)?
            .choice()
            .ok_or(Error::Invalid("effect parameter"))
    }
    pub fn flag(&self, id: &str) -> Result<bool> {
        self.get(id)?
            .flag()
            .ok_or(Error::Invalid("effect parameter"))
    }
    pub fn color(&self, id: &str) -> Result<project::Color> {
        self.get(id)?
            .color()
            .ok_or(Error::Invalid("effect parameter"))
    }
    pub fn point(&self, id: &str) -> Result<[f64; 2]> {
        self.get(id)?
            .point()
            .ok_or(Error::Invalid("effect parameter"))
    }
}

pub type EffectEvaluator = fn(&EffectFrame) -> Result<Vec<EffectPass>>;

/// The evaluator of an effect kind; a kind without one is unsupported.
pub fn evaluator(kind: &str) -> Option<EffectEvaluator> {
    Some(match kind {
        "blur" => blur,
        "color_grade" => color_grade,
        "chroma_key" => chroma_key,
        _ => return None,
    })
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

/// The passes of one effect instance at item-local time `local`.
pub fn evaluate_effect(
    animations: &Animations,
    effect: &EffectInstance,
    local: Tick,
    width: u32,
    height: u32,
) -> Result<Vec<EffectPass>> {
    let evaluate = evaluator(&effect.kind).ok_or(Error::Unsupported("effect kind"))?;
    evaluate(&EffectFrame::new(animations, effect, local, width, height)?)
}

/// The passes of every enabled effect of `item`, in stack order.
pub fn evaluate(item: &Item, local: Tick, width: u32, height: u32) -> Result<Vec<EffectPass>> {
    let mut passes = Vec::new();
    for effect in item.effects.iter().filter(|e| e.enabled) {
        passes.extend(evaluate_effect(
            &item.animations,
            effect,
            local,
            width,
            height,
        )?);
    }
    Ok(passes)
}
