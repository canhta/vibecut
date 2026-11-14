//! Declares every item parameter once and generates the patch enum, its checks and its table.
use crate::session::edit::parameters::ApplyCx;
use crate::session::{EditError, Result};
use captions::NfcText;
use project::{
    AnimationPath, BuiltinPath, Color, GraphicParams, Item, ItemParams, Param, ParamValue,
    StrokeAlign, TextBackground, TextParams, TransformField,
};
use serde::{Deserialize, Serialize};

/// The item kinds a parameter can address. Graphic shapes are separate because some
/// parameters exist on only some of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamTarget {
    Video,
    Audio,
    Text,
    Caption,
    Overlay,
    Sticker,
    Effect,
    Rectangle,
    Ellipse,
    Polygon,
    Star,
}
impl ParamTarget {
    /// The target a piece of item content falls under.
    pub fn of(content: &ItemParams) -> Self {
        match content {
            ItemParams::Video(_) => Self::Video,
            ItemParams::Audio(_) => Self::Audio,
            ItemParams::Text(_) => Self::Text,
            ItemParams::Caption(_) => Self::Caption,
            ItemParams::Overlay(_) => Self::Overlay,
            ItemParams::Sticker(_) => Self::Sticker,
            ItemParams::Effect(_) => Self::Effect,
            ItemParams::Graphic(GraphicParams::Rectangle { .. }) => Self::Rectangle,
            ItemParams::Graphic(GraphicParams::Ellipse { .. }) => Self::Ellipse,
            ItemParams::Graphic(GraphicParams::Polygon { .. }) => Self::Polygon,
            ItemParams::Graphic(GraphicParams::Star { .. }) => Self::Star,
        }
    }
}

/// What a parameter admits. The patch checks and the inspector read the same value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamBounds {
    /// Every value of the type is admitted: switches, choices and colours.
    Free,
    /// A finite number within `min..=max`. Typed text beyond the range is pulled in
    /// when `clamp` is set and refused otherwise; the patch itself always refuses.
    Range { min: f64, max: f64, clamp: bool },
    /// Text of at most `max_bytes`; a blank text is admitted only when `blank` is set.
    Text { max_bytes: usize, blank: bool },
    /// A compound value or `None`; the value type decides its own validity.
    Shape,
}
impl ParamBounds {
    const fn range(min: f64, max: f64) -> Self {
        Self::Range {
            min,
            max,
            clamp: false,
        }
    }
    const fn transform(field: TransformField) -> Self {
        let (min, max) = field.bounds();
        Self::Range {
            min,
            max,
            clamp: true,
        }
    }
    /// The numeric range, when the parameter is a bounded number.
    pub const fn limits(&self) -> Option<(f64, f64)> {
        match self {
            Self::Range { min, max, .. } => Some((*min, *max)),
            _ => None,
        }
    }
    fn admits_number(&self, value: f64) -> bool {
        match self {
            Self::Range { min, max, .. } => value.is_finite() && (*min..=*max).contains(&value),
            _ => value.is_finite(),
        }
    }
}

/// The unit a parameter's value is authored in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamUnit {
    None,
    /// Canvas pixels, or font pixels for type.
    Pixels,
    Degrees,
    /// A multiplier the inspector shows as a percentage: opacity 0..=1, scale 0.01..=10.
    Fraction,
    /// A 0..=100 amount that is already a percentage.
    Percent,
    Decibels,
}
impl ParamUnit {
    /// The suffix the inspector prints beside the value; pixels print none.
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Fraction | Self::Percent => "%",
            Self::Decibels => "dB",
            Self::Degrees => "°",
            Self::None | Self::Pixels => "",
        }
    }
    /// The factor from the authored value to the shown number.
    pub const fn shown_scale(self) -> f64 {
        match self {
            Self::Fraction => 100.0,
            _ => 1.0,
        }
    }
}

/// One row of the read-only parameter table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemParam {
    /// The wire name: the patch `field`, and the only name an adapter uses.
    pub name: &'static str,
    pub targets: &'static [ParamTarget],
    pub bounds: ParamBounds,
    pub unit: ParamUnit,
    /// The animation channel the value keys on, for an animatable parameter.
    pub path: Option<&'static str>,
}
impl ItemParam {
    pub fn applies_to(&self, content: &ItemParams) -> bool {
        self.targets.contains(&ParamTarget::of(content))
    }
}

/// Why text did not become a patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParamParseError {
    #[error("unknown item field")]
    Unknown,
    #[error("invalid item field value")]
    Value,
}

/// A payload type's own admission of a value under the declared bounds.
trait Admit {
    fn admits(&self, bounds: &ParamBounds) -> bool;
}
macro_rules! admit_always {
    ($($ty:ty),* $(,)?) => {$(
        impl Admit for $ty {
            fn admits(&self, _: &ParamBounds) -> bool {
                true
            }
        }
    )*};
}
admit_always!(
    bool,
    Color,
    project::BlendMode,
    project::FitMode,
    project::TextAlign,
    project::TextCase,
    project::TextDecoration,
    StrokeAlign,
);
impl Admit for f64 {
    fn admits(&self, bounds: &ParamBounds) -> bool {
        bounds.admits_number(*self)
    }
}
impl Admit for f32 {
    fn admits(&self, bounds: &ParamBounds) -> bool {
        bounds.admits_number(f64::from(*self))
    }
}
impl Admit for u16 {
    fn admits(&self, bounds: &ParamBounds) -> bool {
        bounds.admits_number(f64::from(*self))
    }
}
impl Admit for NfcText {
    fn admits(&self, bounds: &ParamBounds) -> bool {
        match bounds {
            ParamBounds::Text { max_bytes, blank } => {
                self.as_str().len() <= *max_bytes && (*blank || !self.as_str().trim().is_empty())
            }
            _ => true,
        }
    }
}
impl Admit for Option<project::Crop> {
    fn admits(&self, _: &ParamBounds) -> bool {
        self.as_ref().is_none_or(|crop| crop.valid())
    }
}
impl Admit for Option<project::StabilizeLevel> {
    fn admits(&self, _: &ParamBounds) -> bool {
        true
    }
}
impl Admit for Option<project::BackgroundRemoval> {
    fn admits(&self, _: &ParamBounds) -> bool {
        self.is_none_or(project::BackgroundRemoval::valid)
    }
}
impl Admit for Vec<project::MatteStroke> {
    fn admits(&self, _: &ParamBounds) -> bool {
        project::MatteStroke::valid_list(self)
    }
}
impl Admit for Option<TextBackground> {
    fn admits(&self, _: &ParamBounds) -> bool {
        self.as_ref().is_none_or(TextBackground::valid)
    }
}
impl Admit for Option<project::TextStroke> {
    fn admits(&self, _: &ParamBounds) -> bool {
        self.as_ref().is_none_or(project::TextStroke::valid)
    }
}
impl Admit for Option<project::TextShadow> {
    fn admits(&self, _: &ParamBounds) -> bool {
        self.as_ref().is_none_or(project::TextShadow::valid)
    }
}

/// A payload type's reading of the text an inspector field or a script holds.
trait ParamText: Sized {
    fn from_text(text: &str, bounds: &ParamBounds) -> Option<Self>;
}
/// A numeric payload read from text, with the conversion a clamped read needs.
trait Number: Copy + std::str::FromStr {
    fn get(self) -> f64;
    fn put(value: f64) -> Self;
}
impl Number for f64 {
    fn get(self) -> f64 {
        self
    }
    fn put(value: f64) -> Self {
        value
    }
}
impl Number for f32 {
    fn get(self) -> f64 {
        f64::from(self)
    }
    fn put(value: f64) -> Self {
        value as f32
    }
}
impl Number for u16 {
    fn get(self) -> f64 {
        f64::from(self)
    }
    fn put(value: f64) -> Self {
        value as u16
    }
}
fn read_number<N: Number>(text: &str, bounds: &ParamBounds) -> Option<N> {
    let value: N = text.trim().parse().ok()?;
    match bounds {
        ParamBounds::Range {
            min,
            max,
            clamp: true,
        } => {
            let number = value.get();
            number.is_finite().then(|| N::put(number.clamp(*min, *max)))
        }
        _ => Some(value),
    }
}
macro_rules! text_number {
    ($($ty:ty),* $(,)?) => {$(
        impl ParamText for $ty {
            fn from_text(text: &str, bounds: &ParamBounds) -> Option<Self> {
                read_number(text, bounds)
            }
        }
    )*};
}
text_number!(f64, f32, u16);
impl ParamText for bool {
    fn from_text(text: &str, _: &ParamBounds) -> Option<Self> {
        match text.trim() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }
}
macro_rules! text_word {
    ($($ty:ty),* $(,)?) => {$(
        impl ParamText for $ty {
            fn from_text(text: &str, _: &ParamBounds) -> Option<Self> {
                serde_json::from_value(serde_json::Value::String(text.trim().to_owned())).ok()
            }
        }
    )*};
}
text_word!(
    project::BlendMode,
    project::FitMode,
    project::TextAlign,
    project::TextCase,
    project::TextDecoration,
    StrokeAlign,
);
impl ParamText for Color {
    fn from_text(text: &str, _: &ParamBounds) -> Option<Self> {
        render::color::parse_hex(text).ok()
    }
}
impl ParamText for NfcText {
    fn from_text(text: &str, bounds: &ParamBounds) -> Option<Self> {
        let text = match bounds {
            ParamBounds::Text { blank: false, .. } => text.trim(),
            _ => text,
        };
        let value = Self::from(text);
        value.admits(bounds).then_some(value)
    }
}
/// A shape that `none` clears and a colour starts with its defaults.
macro_rules! text_shape {
    ($($ty:ty),* $(,)?) => {$(
        impl ParamText for Option<$ty> {
            fn from_text(text: &str, bounds: &ParamBounds) -> Option<Self> {
                let text = text.trim();
                if text.eq_ignore_ascii_case("none") {
                    return Some(None);
                }
                let color = Color::from_text(text, bounds)?;
                Some(Some(<$ty>::from_color(color)))
            }
        }
    )*};
}
trait FromColor {
    fn from_color(color: Color) -> Self;
}
macro_rules! from_color {
    ($($ty:ty),* $(,)?) => {$(
        impl FromColor for $ty {
            fn from_color(color: Color) -> Self {
                Self {
                    color,
                    ..Self::default()
                }
            }
        }
    )*};
}
from_color!(TextBackground, project::TextStroke, project::TextShadow);
text_shape!(TextBackground, project::TextStroke, project::TextShadow);
/// `none` or `off` switches stabilization off; a level word switches it on at that level.
impl ParamText for Option<project::StabilizeLevel> {
    fn from_text(text: &str, _: &ParamBounds) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("none") || text.eq_ignore_ascii_case("off") {
            return Some(None);
        }
        serde_json::from_value(serde_json::Value::String(text.to_owned()))
            .ok()
            .map(Some)
    }
}
/// `none` or `off` switches background removal off, `on` switches it on with the model's own
/// edge, and a number switches it on with that edge feather.
impl ParamText for Option<project::BackgroundRemoval> {
    fn from_text(text: &str, _: &ParamBounds) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("none") || text.eq_ignore_ascii_case("off") {
            return Some(None);
        }
        if text.eq_ignore_ascii_case("on") {
            return Some(Some(project::BackgroundRemoval::default()));
        }
        let removal = project::BackgroundRemoval {
            feather: text.parse().ok()?,
        };
        removal.valid().then_some(Some(removal))
    }
}
/// The whole stroke list as JSON, or `none` for an empty list; a drag on the player is the usual
/// writer, so this is for scripts.
impl ParamText for Vec<project::MatteStroke> {
    fn from_text(text: &str, _: &ParamBounds) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("none") {
            return Some(Vec::new());
        }
        serde_json::from_str(text).ok()
    }
}
/// A crop is placed by dragging, not typed, so no text reads as one.
impl ParamText for Option<project::Crop> {
    fn from_text(_: &str, _: &ParamBounds) -> Option<Self> {
        None
    }
}

fn animate(target: &mut Item, path: BuiltinPath, value: ParamValue) -> Result<()> {
    target
        .animations
        .set(AnimationPath::from(path), Param::new(value)?)?;
    Ok(())
}
fn text(target: &mut Item) -> Option<&mut TextParams> {
    match &mut target.content {
        ItemParams::Text(params) => Some(params),
        _ => None,
    }
}
/// The background box, created with its defaults on the first edit of one of its parts.
fn text_box(target: &mut Item) -> Option<&mut TextBackground> {
    text(target).map(|params| params.background.get_or_insert_with(Default::default))
}
fn graphic(target: &mut Item) -> Option<&mut GraphicParams> {
    match &mut target.content {
        ItemParams::Graphic(shape) => Some(shape),
        _ => None,
    }
}
fn stroke_width(shape: &mut GraphicParams) -> &mut f64 {
    match shape {
        GraphicParams::Rectangle { stroke_width, .. }
        | GraphicParams::Ellipse { stroke_width, .. }
        | GraphicParams::Polygon { stroke_width, .. }
        | GraphicParams::Star { stroke_width, .. } => stroke_width,
    }
}
fn stroke_align(shape: &mut GraphicParams) -> &mut StrokeAlign {
    match shape {
        GraphicParams::Rectangle { stroke_align, .. }
        | GraphicParams::Ellipse { stroke_align, .. }
        | GraphicParams::Polygon { stroke_align, .. }
        | GraphicParams::Star { stroke_align, .. } => stroke_align,
    }
}
fn corner_radius(shape: &mut GraphicParams) -> Option<&mut f64> {
    match shape {
        GraphicParams::Rectangle { corner_radius, .. }
        | GraphicParams::Polygon { corner_radius, .. } => Some(corner_radius),
        _ => None,
    }
}

const fn same(mut a: &[u8], mut b: &[u8]) -> bool {
    loop {
        match (a, b) {
            ([], []) => return true,
            ([x, rest_a @ ..], [y, rest_b @ ..]) if *x == *y => {
                a = rest_a;
                b = rest_b;
            }
            _ => return false,
        }
    }
}
/// The row of a wire name; a name that is not a field has none.
pub const fn item_param(name: &str) -> Option<&'static ItemParam> {
    let mut rows = ITEM_PARAMS;
    while let [row, rest @ ..] = rows {
        if same(row.name.as_bytes(), name.as_bytes()) {
            return Some(row);
        }
        rows = rest;
    }
    None
}

use ParamTarget as T;
/// Everything that takes a placement: all kinds except audio. An adjustment layer draws no
/// picture, but its transform, opacity and blend place the region it adjusts.
const PICTURE: &[T] = &[
    T::Video,
    T::Text,
    T::Caption,
    T::Overlay,
    T::Sticker,
    T::Effect,
    T::Rectangle,
    T::Ellipse,
    T::Polygon,
    T::Star,
];
const AUDIBLE: &[T] = &[T::Audio, T::Video];
const VIDEO: &[T] = &[T::Video];
const TEXT: &[T] = &[T::Text];
const FILLED: &[T] = &[T::Overlay, T::Rectangle, T::Ellipse, T::Polygon, T::Star];
const SHAPE: &[T] = &[T::Rectangle, T::Ellipse, T::Polygon, T::Star];
const ROUNDED: &[T] = &[T::Rectangle, T::Polygon];
const POLYGON: &[T] = &[T::Polygon];
const STAR: &[T] = &[T::Star];

const VOLUME: ParamBounds = ParamBounds::range(
    *project::VOLUME_DB_RANGE.start() as f64,
    *project::VOLUME_DB_RANGE.end() as f64,
);
const SPACING: ParamBounds = ParamBounds::range(
    -(project::MAX_LETTER_SPACING as f64),
    project::MAX_LETTER_SPACING as f64,
);
const EXTENT: ParamBounds = ParamBounds::range(0.0, TextBackground::MAX_EXTENT as f64);
const OFFSET: ParamBounds = ParamBounds::range(
    -(TextBackground::MAX_OFFSET as f64),
    TextBackground::MAX_OFFSET as f64,
);

/// Declares each parameter once. An entry names its wire name, the kinds it addresses,
/// its bounds and unit, the channel it keys on, and how it writes into an item; the
/// patch variant, compatibility, validation, application, parsing and the table follow.
macro_rules! item_params {
    ($(
        $(#[$meta:meta])*
        $variant:ident($ty:ty) = $name:literal {
            targets: $targets:expr,
            bounds: $bounds:expr,
            unit: $unit:expr,
            path: $path:expr,
            $(also: |$av:ident| $also:expr,)?
            apply: |$t:ident, $v:ident, $cx:ident| $apply:expr $(,)?
        }
    ),* $(,)?) => {
        /// A shared field edit; incompatible item kinds are left unchanged.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "field", content = "value", deny_unknown_fields)]
        pub enum ItemParamPatch {
            $(
                $(#[$meta])*
                #[serde(rename = $name)]
                $variant($ty),
            )*
        }

        /// Every item parameter, in declaration order.
        pub const ITEM_PARAMS: &[ItemParam] = &[$(
            ItemParam {
                name: $name,
                targets: $targets,
                bounds: $bounds,
                unit: $unit,
                path: $path,
            },
        )*];

        impl ItemParamPatch {
            /// The wire name of the field this patch sets.
            pub fn field(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $name,)*
                }
            }
            /// This field's row in the parameter table.
            pub fn row(&self) -> &'static ItemParam {
                match self {
                    $(Self::$variant(_) => {
                        const ROW: ItemParam = ItemParam {
                            name: $name,
                            targets: $targets,
                            bounds: $bounds,
                            unit: $unit,
                            path: $path,
                        };
                        &ROW
                    })*
                }
            }
            /// Whether the field addresses this content.
            pub fn compatible(&self, content: &ItemParams) -> bool {
                self.row().applies_to(content)
            }
            pub(crate) fn validate(&self) -> Result<()> {
                let admitted = match self {
                    $(Self::$variant(value) => {
                        Admit::admits(value, &$bounds)
                            $(&& {
                                let $av = value;
                                $also
                            })?
                    })*
                };
                if admitted {
                    Ok(())
                } else {
                    Err(EditError::Invalid("shared field value"))
                }
            }
            pub(in crate::session) fn apply(
                &self,
                target: &mut Item,
                cx: &mut ApplyCx<'_>,
            ) -> Result<()> {
                match self {
                    $(Self::$variant($v) => {
                        let $t = &mut *target;
                        let $cx = &mut *cx;
                        let _ = (&$t, &$cx);
                        $apply
                    })*
                }
            }
            /// Reads the text of an inspector field or a script as the patch of `field`.
            pub fn parse(field: &str, text: &str) -> std::result::Result<Self, ParamParseError> {
                match field {
                    $($name => <$ty as ParamText>::from_text(text, &$bounds)
                        .map(Self::$variant)
                        .ok_or(ParamParseError::Value),)*
                    _ => Err(ParamParseError::Unknown),
                }
            }
        }
    };
}

item_params! {
    BlendMode(project::BlendMode) = "blendMode" {
        targets: PICTURE,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            t.blend_mode = *v;
            Ok(())
        },
    },
    Opacity(f64) = "opacity" {
        targets: PICTURE,
        bounds: ParamBounds::range(0.0, 1.0),
        unit: ParamUnit::Fraction,
        path: Some("opacity"),
        apply: |t, v, cx| animate(t, BuiltinPath::Opacity, ParamValue::Scalar(*v)),
    },
    AudioGain(f32) = "volume" {
        targets: AUDIBLE,
        bounds: VOLUME,
        unit: ParamUnit::Decibels,
        path: Some("volume"),
        also: |gain| project::valid_volume(*gain),
        apply: |t, v, cx| {
            match &mut t.content {
                ItemParams::Audio(p) => p.volume = *v,
                ItemParams::Video(p) => p.volume = *v,
                _ => {}
            }
            animate(t, BuiltinPath::Volume, ParamValue::Scalar(f64::from(*v)))
        },
    },
    AudioMuted(bool) = "muted" {
        targets: AUDIBLE,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            match &mut t.content {
                ItemParams::Audio(p) => p.muted = *v,
                ItemParams::Video(p) => p.muted = *v,
                _ => {}
            }
            animate(t, BuiltinPath::Muted, ParamValue::Bool(*v))
        },
    },
    TextContent(NfcText) = "text_content" {
        targets: TEXT,
        bounds: ParamBounds::Text { max_bytes: 1 << 20, blank: true },
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            cx.charge_text(v.as_str().len())?;
            if let Some(p) = text(t) {
                p.content = v.clone();
            }
            Ok(())
        },
    },
    TextFont(NfcText) = "text_font" {
        targets: TEXT,
        bounds: ParamBounds::Text { max_bytes: 256, blank: false },
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            cx.charge_text(v.as_str().len())?;
            if let Some(p) = text(t) {
                p.font = v.clone();
            }
            Ok(())
        },
    },
    TextWeight(u16) = "text_weight" {
        targets: TEXT,
        bounds: ParamBounds::range(100.0, 900.0),
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.weight = *v;
            }
            Ok(())
        },
    },
    TextItalic(bool) = "text_italic" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.italic = *v;
            }
            Ok(())
        },
    },
    TextLetterSpacing(f32) = "text_letter_spacing" {
        targets: TEXT,
        bounds: SPACING,
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.letter_spacing = *v;
            }
            Ok(())
        },
    },
    TextLineHeight(f32) = "text_line_height" {
        targets: TEXT,
        bounds: ParamBounds::range(0.0, project::MAX_LINE_HEIGHT as f64),
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.line_height = *v;
            }
            Ok(())
        },
    },
    TextSize(f32) = "text_size" {
        targets: TEXT,
        bounds: ParamBounds::range(1.0, 4096.0),
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.font_size = *v;
            }
            Ok(())
        },
    },
    TextColor(Color) = "text_color" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.color = *v;
            }
            Ok(())
        },
    },
    TextBackground(Option<project::TextBackground>) = "text_background" {
        targets: TEXT,
        bounds: ParamBounds::Shape,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.background = v.clone();
            }
            Ok(())
        },
    },
    TextBackgroundColor(Color) = "text_background_color" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(b) = text_box(t) {
                b.color = *v;
            }
            Ok(())
        },
    },
    TextBackgroundPaddingX(f32) = "text_background_padding_x" {
        targets: TEXT,
        bounds: EXTENT,
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(b) = text_box(t) {
                b.padding_x = *v;
            }
            Ok(())
        },
    },
    TextBackgroundPaddingY(f32) = "text_background_padding_y" {
        targets: TEXT,
        bounds: EXTENT,
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(b) = text_box(t) {
                b.padding_y = *v;
            }
            Ok(())
        },
    },
    TextBackgroundOffsetX(f32) = "text_background_offset_x" {
        targets: TEXT,
        bounds: OFFSET,
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(b) = text_box(t) {
                b.offset_x = *v;
            }
            Ok(())
        },
    },
    TextBackgroundOffsetY(f32) = "text_background_offset_y" {
        targets: TEXT,
        bounds: OFFSET,
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(b) = text_box(t) {
                b.offset_y = *v;
            }
            Ok(())
        },
    },
    TextBackgroundCornerRadius(f32) = "text_background_corner_radius" {
        targets: TEXT,
        bounds: EXTENT,
        unit: ParamUnit::Pixels,
        path: None,
        apply: |t, v, cx| {
            if let Some(b) = text_box(t) {
                b.radius = *v;
            }
            Ok(())
        },
    },
    TextAlign(project::TextAlign) = "text_align" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.align = *v;
            }
            Ok(())
        },
    },
    TextDecoration(project::TextDecoration) = "text_decoration" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.decoration = *v;
            }
            Ok(())
        },
    },
    TextCase(project::TextCase) = "text_case" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.case = *v;
            }
            Ok(())
        },
    },
    TextStroke(Option<project::TextStroke>) = "text_stroke" {
        targets: TEXT,
        bounds: ParamBounds::Shape,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.stroke = v.clone();
            }
            Ok(())
        },
    },
    TextShadow(Option<project::TextShadow>) = "text_shadow" {
        targets: TEXT,
        bounds: ParamBounds::Shape,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.shadow = v.clone();
            }
            Ok(())
        },
    },
    TextGlow(bool) = "text_glow" {
        targets: TEXT,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(p) = text(t) {
                p.glow = *v;
            }
            Ok(())
        },
    },
    PositionX(f64) = "transform.positionX" {
        targets: PICTURE,
        bounds: ParamBounds::transform(TransformField::PositionX),
        unit: ParamUnit::Pixels,
        path: Some("transform.positionX"),
        apply: |t, v, cx| animate(t, BuiltinPath::TransformPositionX, ParamValue::Scalar(*v)),
    },
    PositionY(f64) = "transform.positionY" {
        targets: PICTURE,
        bounds: ParamBounds::transform(TransformField::PositionY),
        unit: ParamUnit::Pixels,
        path: Some("transform.positionY"),
        apply: |t, v, cx| animate(t, BuiltinPath::TransformPositionY, ParamValue::Scalar(*v)),
    },
    ScaleX(f64) = "transform.scaleX" {
        targets: PICTURE,
        bounds: ParamBounds::transform(TransformField::ScaleX),
        unit: ParamUnit::Fraction,
        path: Some("transform.scaleX"),
        apply: |t, v, cx| animate(t, BuiltinPath::TransformScaleX, ParamValue::Scalar(*v)),
    },
    ScaleY(f64) = "transform.scaleY" {
        targets: PICTURE,
        bounds: ParamBounds::transform(TransformField::ScaleY),
        unit: ParamUnit::Fraction,
        path: Some("transform.scaleY"),
        apply: |t, v, cx| animate(t, BuiltinPath::TransformScaleY, ParamValue::Scalar(*v)),
    },
    Rotate(f64) = "transform.rotate" {
        targets: PICTURE,
        bounds: ParamBounds::transform(TransformField::Rotate),
        unit: ParamUnit::Degrees,
        path: Some("transform.rotate"),
        apply: |t, v, cx| animate(t, BuiltinPath::TransformRotate, ParamValue::Scalar(*v)),
    },
    Crop(Option<project::Crop>) = "crop" {
        targets: VIDEO,
        bounds: ParamBounds::Shape,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let ItemParams::Video(p) = &mut t.content {
                p.crop = *v;
            }
            Ok(())
        },
    },
    /// Per-item noise-reduction switch of a video item.
    ReduceNoise(bool) = "reduce_noise" {
        targets: VIDEO,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let ItemParams::Video(p) = &mut t.content {
                p.reduce_noise = *v;
            }
            Ok(())
        },
    },
    /// Per-item stabilization of a video item: a level switches it on, `None` off. The corrective
    /// motion is not stored here; render looks it up by source.
    Stabilize(Option<project::StabilizeLevel>) = "stabilize" {
        targets: VIDEO,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let ItemParams::Video(p) = &mut t.content {
                p.stabilize = *v;
            }
            Ok(())
        },
    },
    /// Person background removal of a video item: a value switches it on with that edge
    /// feather, `None` off. The person matte is not stored here; render looks it up by source.
    BackgroundRemoval(Option<project::BackgroundRemoval>) = "background_removal" {
        targets: VIDEO,
        bounds: ParamBounds::Shape,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let ItemParams::Video(p) = &mut t.content {
                p.background_removal = *v;
            }
            Ok(())
        },
    },
    /// The edge feather of a video item whose background removal is on; refused while it is off.
    BackgroundRemovalFeather(f32) = "background_removal_feather" {
        targets: VIDEO,
        bounds: ParamBounds::Range {
            min: 0.0,
            max: project::BackgroundRemoval::MAX_FEATHER as f64,
            clamp: true,
        },
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            match &mut t.content {
                ItemParams::Video(project::VideoParams {
                    background_removal: Some(removal),
                    ..
                }) => {
                    removal.feather = *v;
                    Ok(())
                }
                _ => Err(EditError::Invalid("background removal is off")),
            }
        },
    },
    /// The refinement brush strokes of a video item's person matte, replaced as a whole list: one
    /// stroke is one patch and so one undo step. Kept while background removal is off.
    MatteStrokes(Vec<project::MatteStroke>) = "matte_strokes" {
        targets: VIDEO,
        bounds: ParamBounds::Shape,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let ItemParams::Video(p) = &mut t.content {
                p.matte_strokes = v.clone();
            }
            Ok(())
        },
    },
    /// How a video or image item's cropped source is placed on the canvas.
    Fit(project::FitMode) = "fit" {
        targets: VIDEO,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| cx.fit(t, *v),
    },
    GraphicFill(Color) = "graphic_fill" {
        targets: FILLED,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            match &mut t.content {
                ItemParams::Overlay(p) => p.color = *v,
                ItemParams::Graphic(
                    GraphicParams::Rectangle { fill, .. }
                    | GraphicParams::Ellipse { fill, .. }
                    | GraphicParams::Polygon { fill, .. }
                    | GraphicParams::Star { fill, .. },
                ) => *fill = *v,
                _ => {}
            }
            Ok(())
        },
    },
    GraphicStroke(Color) = "graphic_stroke" {
        targets: FILLED,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let ItemParams::Graphic(
                GraphicParams::Rectangle { stroke, .. }
                | GraphicParams::Ellipse { stroke, .. }
                | GraphicParams::Polygon { stroke, .. }
                | GraphicParams::Star { stroke, .. },
            ) = &mut t.content
            {
                *stroke = *v;
                return Ok(());
            }
            let [r, g, b, a] = v.0;
            t.animations.set(
                AnimationPath::new("params.strokeColor")?,
                Param::new(ParamValue::Text(
                    format!("#{r:02X}{g:02X}{b:02X}{a:02X}").into(),
                ))?,
            )?;
            Ok(())
        },
    },
    GraphicStrokeWidth(f64) = "graphic_stroke_width" {
        targets: SHAPE,
        bounds: ParamBounds::range(0.0, 64.0),
        unit: ParamUnit::Pixels,
        path: Some("params.strokeWidth"),
        apply: |t, v, cx| {
            if let Some(shape) = graphic(t) {
                *stroke_width(shape) = *v;
            }
            Ok(())
        },
    },
    GraphicStrokeAlign(StrokeAlign) = "graphic_stroke_align" {
        targets: SHAPE,
        bounds: ParamBounds::Free,
        unit: ParamUnit::None,
        path: None,
        apply: |t, v, cx| {
            if let Some(shape) = graphic(t) {
                *stroke_align(shape) = *v;
            }
            Ok(())
        },
    },
    GraphicCornerRadius(f64) = "graphic_corner_radius" {
        targets: ROUNDED,
        bounds: ParamBounds::range(0.0, 50.0),
        unit: ParamUnit::Percent,
        path: Some("params.cornerRadius"),
        apply: |t, v, cx| {
            if let Some(radius) = graphic(t).and_then(corner_radius) {
                *radius = *v;
            }
            Ok(())
        },
    },
    GraphicSides(f64) = "graphic_sides" {
        targets: POLYGON,
        bounds: ParamBounds::range(3.0, 12.0),
        unit: ParamUnit::None,
        path: Some("params.sides"),
        apply: |t, v, cx| {
            if let Some(GraphicParams::Polygon { sides, .. }) = graphic(t) {
                *sides = *v;
            }
            Ok(())
        },
    },
    GraphicPoints(f64) = "graphic_points" {
        targets: STAR,
        bounds: ParamBounds::range(3.0, 12.0),
        unit: ParamUnit::None,
        path: Some("params.points"),
        apply: |t, v, cx| {
            if let Some(GraphicParams::Star { points, .. }) = graphic(t) {
                *points = *v;
            }
            Ok(())
        },
    },
    GraphicDepth(f64) = "graphic_depth" {
        targets: STAR,
        bounds: ParamBounds::range(1.0, 99.0),
        unit: ParamUnit::Percent,
        path: Some("params.depth"),
        apply: |t, v, cx| {
            if let Some(GraphicParams::Star { depth, .. }) = graphic(t) {
                *depth = *v;
            }
            Ok(())
        },
    },
}
