//! Item identities, typed item parameters and item timing.
use super::project::text;
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionParams {
    pub cues: CueTrack,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverlayParams {
    pub color: Color,
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
}
impl ItemParams {
    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Text(_) => TrackKind::Text,
            Self::Caption(_) => TrackKind::Text,
            Self::Overlay(_) => TrackKind::Video,
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
