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
/// Positions/tangent offsets in normalized item coordinates (0..1 spans the item).
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
