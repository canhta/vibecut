//! Item identities, typed item parameters and item timing.
use super::project::{CaptionPresetId, text};
use super::settings::Color;
use super::track::TrackKind;
use crate::visual::DEFAULT_WEIGHT;
use crate::{Error, Result};
use captions::{CueTrack, NfcText};
use media::AssetId;
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick};

id!(ItemId);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoParams {
    pub source_audio_enabled: bool,
    pub volume: f32,
    pub muted: bool,
    /// Source-normalized region shown in place of the whole frame; absent keeps the full frame.
    #[serde(default)]
    pub crop: Option<crate::Crop>,
    #[serde(default)]
    pub fades: crate::Fades,
}
/// Remote-library provenance carried with an audio item. URLs are canonical and
/// bounded; no token, cache path or local absolute path belongs here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibrarySound {
    /// Provider identity; Freesound sound identifiers are positive.
    pub id: u64,
    pub name: NfcText,
    pub author: NfcText,
    pub licence: NfcText,
    /// Human-readable attribution shown in the panel and written into exports.
    pub attribution: NfcText,
    /// Public provider page; the item's offline media is a cached preview.
    pub source_url: String,
    /// Exact preview URL the cached bytes were fetched from. Changing it
    /// invalidates the content-addressed cache entry.
    pub audio_url: String,
    pub downloads: u64,
    /// Provider-reported duration for display before the cached file is probed.
    pub duration_ms: u64,
}
impl LibrarySound {
    pub(super) fn validate(&self) -> Result<()> {
        if self.id == 0 {
            return Err(Error::Invalid("library sound id"));
        }
        text(&self.name)?;
        text(&self.author)?;
        text(&self.licence)?;
        text(&self.attribution)?;
        if self.name.as_str().is_empty()
            || self.author.as_str().is_empty()
            || self.licence.as_str().is_empty()
            || self.attribution.as_str().is_empty()
        {
            return Err(Error::Invalid("library sound attribution"));
        }
        https(&self.source_url)?;
        https(&self.audio_url)?;
        if self.duration_ms > 86_400_000 {
            return Err(Error::Invalid("library sound duration"));
        }
        Ok(())
    }
}
fn https(value: &str) -> Result<()> {
    if value.len() > 8192
        || !value.starts_with("https://")
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ' || b == b'\\' || b == b'"')
    {
        return Err(Error::Invalid("library sound URL"));
    }
    Ok(())
}
/// Audio items either reference a project asset or a cached provider preview.
/// Ordinary uploads keep the default so no content field is duplicated.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "sourceType", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioSource {
    #[default]
    Upload,
    Library {
        sound: LibrarySound,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioParams {
    pub volume: f32,
    pub muted: bool,
    #[serde(default)]
    pub source: AudioSource,
    #[serde(default)]
    pub fades: crate::Fades,
}
/// Percentages of the canvas; `(50, 84.5)` reproduces the original lower-third box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextPosition {
    pub x: f32,
    pub y: f32,
}
impl Default for TextPosition {
    fn default() -> Self {
        Self { x: 50.0, y: 84.5 }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextBackground {
    pub color: Color,
    /// Horizontal padding in project canvas pixels; scaled with the output like the font size.
    pub padding_x: f32,
    /// Vertical padding in project canvas pixels.
    pub padding_y: f32,
    /// Shift of the box from the text bounds in project canvas pixels (y down).
    pub offset_x: f32,
    pub offset_y: f32,
    pub radius: f32,
}
impl Default for TextBackground {
    fn default() -> Self {
        Self {
            color: Color([0, 0, 0, 160]),
            padding_x: 12.0,
            padding_y: 12.0,
            offset_x: 0.0,
            offset_y: 0.0,
            radius: 4.0,
        }
    }
}
impl TextBackground {
    /// Largest padding or radius in project canvas pixels.
    pub const MAX_EXTENT: f32 = 128.0;
    /// Largest box offset magnitude in project canvas pixels.
    pub const MAX_OFFSET: f32 = 1024.0;
    /// Whether every geometry field is finite and within its documented range.
    pub fn valid(&self) -> bool {
        let extent = |v: f32| v.is_finite() && (0.0..=Self::MAX_EXTENT).contains(&v);
        let offset = |v: f32| v.is_finite() && v.abs() <= Self::MAX_OFFSET;
        extent(self.padding_x)
            && extent(self.padding_y)
            && extent(self.radius)
            && offset(self.offset_x)
            && offset(self.offset_y)
    }
}
/// Horizontal alignment of lines inside the text box.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}
/// Line drawn across the text in its fill colour.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextParams {
    pub content: NfcText,
    /// Font family name, resolved by the text crate; an unavailable family renders in the
    /// bundled default.
    pub font: NfcText,
    pub font_size: f32,
    /// CSS weight 100..=900 (400 regular, 700 bold).
    pub weight: u16,
    pub italic: bool,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub decoration: TextDecoration,
    /// Extra advance after every grapheme cluster, in project canvas pixels (scaled with
    /// the output like the font size); negative tightens.
    pub letter_spacing: f32,
    /// Baseline distance in project canvas pixels; zero uses the font's natural height.
    pub line_height: f32,
    pub color: Color,
    pub background: Option<TextBackground>,
    pub position: TextPosition,
}
impl Default for TextParams {
    fn default() -> Self {
        Self::new("")
    }
}
impl TextParams {
    /// Baseline title style shared by insertion and fixtures.
    pub fn new(content: impl Into<NfcText>) -> Self {
        Self {
            content: content.into(),
            font: "Inter".into(),
            font_size: 48.0,
            weight: DEFAULT_WEIGHT,
            italic: false,
            align: TextAlign::default(),
            decoration: TextDecoration::default(),
            letter_spacing: 0.0,
            line_height: 0.0,
            color: Color([255; 4]),
            background: None,
            position: TextPosition::default(),
        }
    }
}
/// Reusable text appearance copied from one item into ordinary text parameters.
/// Content is never part of a preset, so applying one keeps every item's words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    pub font: NfcText,
    pub font_size: f32,
    pub weight: u16,
    pub italic: bool,
    pub align: TextAlign,
    pub decoration: TextDecoration,
    pub letter_spacing: f32,
    pub line_height: f32,
    pub color: Color,
    pub background: Option<TextBackground>,
    pub position: TextPosition,
}
impl TextStyle {
    /// The style of `params`, leaving its content behind.
    pub fn from_params(params: &TextParams) -> Self {
        Self {
            font: params.font.clone(),
            font_size: params.font_size,
            weight: params.weight,
            italic: params.italic,
            align: params.align,
            decoration: params.decoration,
            letter_spacing: params.letter_spacing,
            line_height: params.line_height,
            color: params.color,
            background: params.background.clone(),
            position: params.position,
        }
    }
    /// Copy this style over `params`, keeping its content.
    pub fn apply_to(&self, params: &mut TextParams) {
        params.font = self.font.clone();
        params.font_size = self.font_size;
        params.weight = self.weight;
        params.italic = self.italic;
        params.align = self.align;
        params.decoration = self.decoration;
        params.letter_spacing = self.letter_spacing;
        params.line_height = self.line_height;
        params.color = self.color;
        params.background = self.background.clone();
        params.position = self.position;
    }
    /// Whether every stored field satisfies the text-item bounds.
    pub fn valid(&self) -> bool {
        crate::visual::valid(
            self.font.as_str(),
            self.weight,
            self.letter_spacing,
            self.line_height,
        ) && self.font_size.is_finite()
            && (1.0..=4096.0).contains(&self.font_size)
            && self.position.x.is_finite()
            && self.position.y.is_finite()
            && (0.0..=100.0).contains(&self.position.x)
            && (0.0..=100.0).contains(&self.position.y)
            && self.background.as_ref().is_none_or(TextBackground::valid)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionParams {
    /// Explicit source-word emphasis; new and imported caption items start disabled.
    pub word_highlight: bool,
    /// Live saved-preset selection; absent keeps imported cue styles and project settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<CaptionPresetId>,
    /// Sparse patch over the selected preset; requires a selection.
    #[serde(default, skip_serializing_if = "captions::CaptionOverrides::is_empty")]
    pub overrides: captions::CaptionOverrides,
    pub cues: CueTrack,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverlayParams {
    pub color: Color,
}
/// Generated geometry in a 512-pixel author square. Saved appearance is explicit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "definitionId",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum GraphicParams {
    Rectangle {
        fill: Color,
        stroke: Color,
        #[serde(rename = "strokeWidth")]
        stroke_width: f64,
        #[serde(rename = "strokeAlign")]
        stroke_align: crate::StrokeAlign,
        #[serde(rename = "cornerRadius")]
        corner_radius: f64,
    },
    Ellipse {
        fill: Color,
        stroke: Color,
        #[serde(rename = "strokeWidth")]
        stroke_width: f64,
        #[serde(rename = "strokeAlign")]
        stroke_align: crate::StrokeAlign,
    },
}
impl GraphicParams {
    pub fn rectangle() -> Self {
        Self::Rectangle {
            fill: Color([255; 4]),
            stroke: Color([0, 0, 0, 255]),
            stroke_width: 0.,
            stroke_align: crate::StrokeAlign::Center,
            corner_radius: 0.,
        }
    }
    pub fn ellipse() -> Self {
        Self::Ellipse {
            fill: Color([255; 4]),
            stroke: Color([0, 0, 0, 255]),
            stroke_width: 0.,
            stroke_align: crate::StrokeAlign::Center,
        }
    }
    /// Stored fallbacks; present animation parameters remain authoritative, even without keys.
    pub fn appearance(&self) -> (Color, Color, f64, crate::StrokeAlign, f64) {
        match *self {
            Self::Rectangle {
                fill,
                stroke,
                stroke_width,
                stroke_align,
                corner_radius,
            } => (fill, stroke, stroke_width, stroke_align, corner_radius),
            Self::Ellipse {
                fill,
                stroke,
                stroke_width,
                stroke_align,
            } => (fill, stroke, stroke_width, stroke_align, 0.),
        }
    }
    pub(crate) fn validate(&self, animations: &crate::Animations) -> Result<()> {
        let (_, _, width, _, radius) = self.appearance();
        if !width.is_finite()
            || !(0. ..=64.).contains(&width)
            || !radius.is_finite()
            || !(0. ..=50.).contains(&radius)
        {
            return Err(Error::Invalid("graphic appearance"));
        }
        for (path, param) in animations.iter() {
            let Some(name) = path.as_str().strip_prefix("params.") else {
                continue;
            };
            match (name, param) {
                ("fill" | "stroke", crate::Param::Color(_)) => {}
                ("strokeWidth" | "cornerRadius" | "strokeAlign", crate::Param::Value(channel)) => {
                    if name == "cornerRadius" && !matches!(self, Self::Rectangle { .. }) {
                        return Err(Error::Invalid("ellipse corner radius"));
                    }
                    let valid = |v: &crate::ParamValue| match (name, v) {
                        ("strokeWidth", crate::ParamValue::Scalar(v)) => {
                            v.is_finite() && (0. ..=64.).contains(v)
                        }
                        ("cornerRadius", crate::ParamValue::Scalar(v)) => {
                            v.is_finite() && (0. ..=50.).contains(v)
                        }
                        ("strokeAlign", crate::ParamValue::Text(v)) => {
                            matches!(v.as_str(), "inside" | "center" | "outside")
                        }
                        _ => false,
                    };
                    if !valid(channel.base())
                        || channel.keys().iter().any(|key| !valid(&key.value))
                        || (name == "strokeAlign"
                            && (channel.extrapolation()
                                != (crate::Extrapolation::Hold, crate::Extrapolation::Hold)
                                || channel
                                    .keys()
                                    .iter()
                                    .any(|key| key.segment != crate::Segment::Step)))
                    {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                _ => return Err(Error::Invalid("graphic animation path or type")),
            }
        }
        Ok(())
    }
}
/// Typed parameters determine item kind; no unvalidated arbitrary parameter bag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ItemParams {
    Video(VideoParams),
    Audio(AudioParams),
    Text(TextParams),
    Caption(CaptionParams),
    Overlay(OverlayParams),
    Graphic(GraphicParams),
}
impl ItemParams {
    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Text(_) => TrackKind::Text,
            Self::Caption(_) => TrackKind::Text,
            Self::Overlay(_) | Self::Graphic(_) => TrackKind::Video,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    #[serde(rename = "blendMode")]
    pub blend_mode: crate::BlendMode,
    pub masks: Vec<crate::Mask>,
    pub effects: Vec<crate::EffectInstance>,
    pub animations: crate::Animations,
    pub id: ItemId,
    /// Visual item hidden in the preview and export while its audio stays audible.
    #[serde(default)]
    pub hidden: bool,
    pub source_asset: Option<AssetId>,
    /// Source ticks for media/captions; local content ticks for generated items.
    pub source_range: SourceRange,
    /// Output timeline ticks. Playback uses the retained retime slice when present; see [`Item::duration`].
    pub start: Tick,
    pub retime: crate::Retime,
    pub content: ItemParams,
}
impl Item {
    pub fn end(&self) -> Result<Tick> {
        self.start
            .checked_add(self.duration()?)
            .ok_or(Error::Invalid("item end overflow"))
    }

    /// Native ASR words of a caption item, each clipped to its cue and the item's
    /// source window, with source and output ranges. Other item kinds carry none.
    ///
    /// # Errors
    ///
    /// Returns an error when the retained retime mapping or a range is unrepresentable.
    pub fn word_spans(&self) -> Result<Vec<WordSpan>> {
        let ItemParams::Caption(params) = &self.content else {
            return Ok(Vec::new());
        };
        let mut spans = Vec::new();
        for cue in &params.cues.cues {
            for (index, word) in cue.word_ranges.iter().enumerate() {
                let start = word
                    .start()
                    .max(self.source_range.start())
                    .max(cue.range.start());
                let end = word.end().min(self.source_range.end()).min(cue.range.end());
                if start >= end {
                    continue;
                }
                let source = SourceRange::new(start, end)?;
                let output =
                    SourceRange::new(self.output_time_of(start)?, self.output_time_of(end)?)?;
                if output.start() >= output.end() {
                    continue;
                }
                spans.push(WordSpan {
                    item: self.id,
                    cue: cue.id.clone(),
                    index,
                    source,
                    output,
                });
            }
        }
        Ok(spans)
    }
}

/// One native ASR word retained by a caption item, in source and output time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordSpan {
    pub item: ItemId,
    pub cue: NfcText,
    pub index: usize,
    pub source: SourceRange,
    pub output: SourceRange,
}

/// Clip/master dB range and hundredth-dB editing grid. Curves interpolate continuously.
pub fn valid_volume(value: f32) -> bool {
    value.is_finite()
        && (-60.0..=20.0).contains(&value)
        && (value * 100.0 - (value * 100.0).round()).abs() < 0.001
}

/// Safe captured-state diagnostics. No cue text, arbitrary cue ID or source path is exposed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionWordWarning {
    pub item: ItemId,
    /// Zero-based retained cue position, never the user-authored cue ID.
    pub cue: usize,
    pub language: captions::LanguageTag,
    pub reason: captions::WordHighlightReason,
}
/// At most 64 item/language/reason summaries; additional unrepresented cues are counted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionWordWarnings {
    #[schemars(length(max = 64))]
    pub warnings: Vec<CaptionWordWarning>,
    pub additional: usize,
}
