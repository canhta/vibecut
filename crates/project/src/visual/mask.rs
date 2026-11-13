//! Persisted masks; coverage and colour composition belong to render.
use crate::{AnimationPath, Color, Error, Result};
use captions::NfcText;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    None,
    #[default]
    Add,
    Subtract,
    Intersect,
    Lighten,
    Darken,
    Difference,
}
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    Inside,
    #[default]
    Center,
    Outside,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskStroke {
    pub color: Color,
    pub width: f64,
    pub align: StrokeAlign,
}
impl Default for MaskStroke {
    fn default() -> Self {
        Self {
            color: Color([255; 4]),
            width: 0.,
            align: StrokeAlign::Center,
        }
    }
}
/// Positions in normalized mask-box coordinates; 0..1 spans its evaluated
/// width and height. Tangents are offsets from the anchor in the same
/// coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskVertex {
    pub p: [f64; 2],
    pub t_in: [f64; 2],
    pub t_out: [f64; 2],
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskPath {
    pub vertices: Vec<MaskVertex>,
    pub closed: bool,
}
/// A freeform path the point editor can modify: closed, and with at least the
/// three anchors that bound an area. Saved open or short paths stay readable.
impl MaskPath {
    /// Unit ellipse seed: four anchors on the box edges with mirrored handles
    /// `k/2` long, `k = 4/3 * tan(pi/8)`. Anchors are top, right, bottom, left.
    pub fn ellipse() -> Self {
        let k = 0.5 * 4.0 / 3.0 * (std::f64::consts::PI / 8.0).tan();
        Self {
            vertices: vec![
                MaskVertex {
                    p: [0.5, 0.0],
                    t_in: [-k, 0.0],
                    t_out: [k, 0.0],
                },
                MaskVertex {
                    p: [1.0, 0.5],
                    t_in: [0.0, -k],
                    t_out: [0.0, k],
                },
                MaskVertex {
                    p: [0.5, 1.0],
                    t_in: [k, 0.0],
                    t_out: [-k, 0.0],
                },
                MaskVertex {
                    p: [0.0, 0.5],
                    t_in: [0.0, k],
                    t_out: [0.0, -k],
                },
            ],
            closed: true,
        }
    }
    /// Unit rectangle with straight edges.
    pub fn rectangle() -> Self {
        Self {
            vertices: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                .into_iter()
                .map(|p| MaskVertex {
                    p,
                    t_in: [0.0; 2],
                    t_out: [0.0; 2],
                })
                .collect(),
            closed: true,
        }
    }
    /// Closed straight polygon from unit-box anchors; 3..=4096 finite points
    /// with |coordinate| <= 100, matching the persisted path bounds.
    pub fn polygon(points: &[[f64; 2]]) -> Result<Self> {
        if !(3..=4096).contains(&points.len())
            || points
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 100.0)
        {
            return Err(Error::Invalid("mask polygon"));
        }
        Ok(Self {
            vertices: points
                .iter()
                .map(|p| MaskVertex {
                    p: *p,
                    t_in: [0.0; 2],
                    t_out: [0.0; 2],
                })
                .collect(),
            closed: true,
        })
    }
    /// Whether the point editor may modify this path.
    pub fn editable(&self) -> Result<()> {
        if !self.closed || !(3..=4096).contains(&self.vertices.len()) {
            return Err(Error::Invalid("mask path not editable"));
        }
        Ok(())
    }
    /// Split one cubic segment at `t`, keeping the curve exactly. `segment` i
    /// joins anchor i to `(i + 1) % len`; `t` is 2^-20..=1-2^-20.
    pub fn split(&self, segment: usize, t: f64) -> Result<Self> {
        self.editable()?;
        let count = self.vertices.len();
        if segment >= count
            || !t.is_finite()
            || !(1.0 / 1_048_576.0..=1.0 - 1.0 / 1_048_576.0).contains(&t)
        {
            return Err(Error::Invalid("mask path split"));
        }
        let next = (segment + 1) % count;
        let split = || Error::Invalid("mask path split");
        let a = self.vertices.get(segment).cloned().ok_or_else(split)?;
        let b = self.vertices.get(next).cloned().ok_or_else(split)?;
        let p0 = a.p;
        let c1 = [a.p[0] + a.t_out[0], a.p[1] + a.t_out[1]];
        let c2 = [b.p[0] + b.t_in[0], b.p[1] + b.t_in[1]];
        let p3 = b.p;
        let lerp = |x: [f64; 2], y: [f64; 2]| [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t];
        let l0 = lerp(p0, c1);
        let l1 = lerp(c1, c2);
        let l2 = lerp(c2, p3);
        let m0 = lerp(l0, l1);
        let m1 = lerp(l1, l2);
        let q = lerp(m0, m1);
        let sub = |x: [f64; 2], y: [f64; 2]| [x[0] - y[0], x[1] - y[1]];
        let vertex = MaskVertex {
            p: q,
            t_in: sub(m0, q),
            t_out: sub(m1, q),
        };
        let mut out = self.clone();
        out.vertices.get_mut(segment).ok_or_else(split)?.t_out = sub(l0, p0);
        out.vertices.get_mut(next).ok_or_else(split)?.t_in = sub(l2, p3);
        out.vertices.insert(segment + 1, vertex);
        if out.vertices.len() > 4096 {
            return Err(Error::TooLarge("mask vertices"));
        }
        if out
            .vertices
            .iter()
            .flat_map(|v| v.p.iter().chain(&v.t_in).chain(&v.t_out))
            .any(|n| !n.is_finite() || n.abs() > 100.0)
        {
            return Err(Error::Invalid("mask vertex"));
        }
        Ok(out)
    }
    /// Insert copied anchors, in order, directly after anchor `after`. Positions and
    /// tangent offsets are kept as given; the path stays closed and at most 4096 anchors.
    pub fn with_inserted(&self, after: usize, vertices: &[MaskVertex]) -> Result<Self> {
        self.editable()?;
        if after >= self.vertices.len() || vertices.is_empty() {
            return Err(Error::Invalid("mask point paste"));
        }
        if self.vertices.len() + vertices.len() > 4096 {
            return Err(Error::TooLarge("mask vertices"));
        }
        if vertices
            .iter()
            .flat_map(|v| v.p.iter().chain(&v.t_in).chain(&v.t_out))
            .any(|n| !n.is_finite() || n.abs() > 100.0)
        {
            return Err(Error::Invalid("mask vertex"));
        }
        let mut out = self.clone();
        let tail = out.vertices.split_off(after + 1);
        out.vertices.extend(vertices.iter().cloned());
        out.vertices.extend(tail);
        Ok(out)
    }
    /// Remove anchors by index, keeping at least three. Survivors keep their
    /// positions and tangent offsets; neighbors reconnect by their handles.
    pub fn without(&self, points: &[usize]) -> Result<Self> {
        self.editable()?;
        if points.is_empty() || points.len() > self.vertices.len() {
            return Err(Error::Invalid("mask point selection"));
        }
        let mut remove = points.to_vec();
        remove.sort_unstable();
        remove.dedup();
        if remove.len() != points.len() || remove.iter().any(|i| *i >= self.vertices.len()) {
            return Err(Error::Invalid("mask point selection"));
        }
        if self.vertices.len() - remove.len() < 3 {
            return Err(Error::Invalid("mask path minimum"));
        }
        let mut out = self.clone();
        for index in remove.into_iter().rev() {
            out.vertices.remove(index);
        }
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaskShape {
    Split,
    CinematicBars,
    Rectangle,
    Ellipse,
    Heart,
    Diamond,
    Star,
    /// Text coverage; the typography fields mean what they do on `TextParams`.
    Text {
        content: NfcText,
        font_size: f32,
        font: NfcText,
        weight: u16,
        italic: bool,
        letter_spacing: f32,
        line_height: f32,
    },
    Path {
        path: MaskPath,
    },
}
impl MaskShape {
    /// Text coverage in the default typography (regular Inter, natural spacing).
    pub fn text(content: impl Into<NfcText>, font_size: f32) -> Self {
        Self::Text {
            content: content.into(),
            font_size,
            font: "Inter".into(),
            weight: super::typography::DEFAULT_WEIGHT,
            italic: false,
            letter_spacing: 0.0,
            line_height: 0.0,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::CinematicBars => "cinematic-bars",
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Heart => "heart",
            Self::Diamond => "diamond",
            Self::Star => "star",
            Self::Text { .. } => "text",
            Self::Path { .. } => "path",
        }
    }
}
/// Center offsets and size are fractions of the placed item. Rotation is degrees;
/// feather, expansion, stroke width and text size are project-canvas pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub id: String,
    pub enabled: bool,
    pub mode: MaskMode,
    pub shape: MaskShape,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub scale: f64,
    pub feather: f64,
    pub expansion: f64,
    pub opacity: f64,
    pub inverted: bool,
    pub stroke: MaskStroke,
}
impl Mask {
    pub fn new(id: impl Into<String>, shape: MaskShape) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            mode: MaskMode::Add,
            shape,
            center_x: 0.,
            center_y: 0.,
            width: 0.6,
            height: 0.6,
            rotation: 0.,
            scale: 1.,
            feather: 0.,
            expansion: 0.,
            opacity: 1.,
            inverted: false,
            stroke: MaskStroke::default(),
        }
    }
    pub fn path(&self, key: &str) -> Result<AnimationPath> {
        AnimationPath::new(format!("masks.{}.params.{key}", self.id))
    }
    pub fn active(&self) -> bool {
        self.enabled
            && self.mode != MaskMode::None
            && !matches!(&self.shape,MaskShape::Text{content,..} if content.as_str().trim().is_empty())
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.path("feather")?;
        self.validate_scalars()?;
        match &self.shape {
            MaskShape::Text {
                content,
                font_size,
                font,
                weight,
                letter_spacing,
                line_height,
                ..
            } if content.as_str().len() > 4096
                || !font_size.is_finite()
                || !(1.0..=4096.0).contains(font_size)
                || !super::typography::valid(
                    font.as_str(),
                    *weight,
                    *letter_spacing,
                    *line_height,
                ) =>
            {
                return Err(Error::Invalid("text mask"));
            }
            MaskShape::Path { path } => {
                if path.vertices.len() > 4096 {
                    return Err(Error::TooLarge("mask vertices"));
                }
                for v in &path.vertices {
                    if v.p
                        .iter()
                        .chain(&v.t_in)
                        .chain(&v.t_out)
                        .any(|n| !n.is_finite() || n.abs() > 100.)
                    {
                        return Err(Error::Invalid("mask vertex"));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
macro_rules! mask_scalars {
    ($($key:literal => $field:ident $(.$member:ident)? : ($lo:expr,$hi:expr)),* $(,)?)=>{
        pub const MASK_SCALARS:&[&str]=&[$($key),*];
        impl Mask {
            fn validate_scalars(&self)->Result<()> {
                $(let value=self.$field$(.$member)?;
                if !value.is_finite() || !($lo..=$hi).contains(&value) {return Err(Error::Invalid("mask parameter"));})*
                Ok(())
            }
            /// Resolve item-local scalar overrides through the one animation
            /// evaluator; animated overshoot clamps to each static parameter range.
            pub fn at(&self,animations:&crate::Animations,local:time::Tick)->Result<Self> {
                let mut mask=self.clone();
                $(match animations.value_at(&self.path($key)?,local)? {
                    Some(crate::ParamValue::Scalar(value))=>mask.$field$(.$member)?=value.clamp($lo,$hi),
                    Some(_)=>return Err(Error::Invalid("mask animation must be scalar")),
                    None=>{}
                })*
                Ok(mask)
            }
        }
    }
}
mask_scalars! {
    "center_x"=>center_x:(-100.,100.),"center_y"=>center_y:(-100.,100.),
    "width"=>width:(0.000001,100.),"height"=>height:(0.000001,100.),
    "rotation"=>rotation:(-1e6,1e6),"scale"=>scale:(0.001,500.),
    "feather"=>feather:(0.,10000.),"expansion"=>expansion:(-10000.,10000.),
    "opacity"=>opacity:(0.,1.),"stroke_width"=>stroke.width:(0.,10000.),
}
