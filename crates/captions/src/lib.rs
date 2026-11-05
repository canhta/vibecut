//! Source-timed transcripts, multilingual cues and forgiving subtitle interchange.
pub mod ass;
mod io;
mod language;
mod mapping;
mod segment;
mod span;
mod timing;
pub use ass::{AssCue, AssImport, AssWarning, AssWarningKind, parse_ass};
pub use io::{Export, Format, Import, export, parse};
pub use language::{LanguageProfile, LanguageRegistry, LanguageTag};
pub use mapping::{CueFragmentId, EditMapping, MappedRange, MappedTick};
pub use segment::{
    LayoutHooks, LineLimit, SegmentRules, Segmentation, SegmentedCue, SourceSpeechUnit, TimingKind,
    segment_transcript, segment_translation, source_speech_units,
};
use serde::{Deserialize, Serialize};
pub use span::{NormalizedText, TextSpan};
use std::{collections::BTreeMap, fmt};
use time::{SourceRange, Tick};
pub use timing::{
    DisplayReplacement, DisplayVariant, MappedTiming, SpanTiming, TimingGroup, TimingProvenance,
    TimingSpans, TimingUnit,
};
use unicode_normalization::UnicodeNormalization;

/// Actionable failures; diagnostics never contain subtitle text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid BCP 47 language tag")]
    Language(#[source] language_tags::ParseError),
    #[error("invalid subtitle input: {0}")]
    Invalid(&'static str),
    #[error("subtitle input or generated output exceeds 16 MiB")]
    TooLarge,
    #[error("subtitle time cannot be represented")]
    Time(#[from] time::TimeError),
}
/// Normalized stored text; construction and deserialization always apply NFC.
#[derive(
    Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "String", into = "String")]
pub struct NfcText(String);
impl From<String> for NfcText {
    fn from(value: String) -> Self {
        Self(value.nfc().collect())
    }
}
impl From<&str> for NfcText {
    fn from(value: &str) -> Self {
        Self::from(value.to_owned())
    }
}
impl From<NfcText> for String {
    fn from(value: NfcText) -> Self {
        value.0
    }
}
impl NfcText {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for NfcText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[text]")
    }
}
/// Optional recognizer confidence, finite and within 0..=1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Confidence(f32);
impl TryFrom<f32> for Confidence {
    type Error = Error;
    fn try_from(value: f32) -> Result<Self, Error> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(Error::Invalid("confidence"));
        }
        Ok(Self(value))
    }
}
impl From<Confidence> for f32 {
    fn from(value: Confidence) -> Self {
        value.0
    }
}
/// A source word; absent timing never implies target-language word alignment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: NfcText,
    pub range: Option<SourceRange>,
    pub speaker: Option<NfcText>,
    pub confidence: Option<Confidence>,
}
/// A stable recognition segment in source coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: NfcText,
    pub range: SourceRange,
    pub text: NfcText,
    pub words: Vec<Word>,
    pub spans: Option<TimingSpans>,
}
/// Provider, model and settings identity used to invalidate derived work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub provider: NfcText,
    pub model: NfcText,
    pub settings: NfcText,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub source_lang: LanguageTag,
    pub fingerprint: Fingerprint,
    pub segments: Vec<Segment>,
}
/// Per-language text and its independent edit/translation provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CueText {
    pub text: NfcText,
    pub edited: bool,
    pub translation_ref: Option<NfcText>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAlignment {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlignment {
    Top,
    Middle,
    Bottom,
}
/// Numpad alignment plus its resolved horizontal/vertical placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CueAlignment {
    /// ASS numpad alignment, 1..=9.
    pub numpad: u8,
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
}
impl CueAlignment {
    /// The one consistent alignment for an ASS numpad value; `None` outside 1..=9.
    pub fn from_numpad(numpad: u8) -> Option<Self> {
        let horizontal = match numpad {
            1 | 4 | 7 => HorizontalAlignment::Left,
            2 | 5 | 8 => HorizontalAlignment::Center,
            3 | 6 | 9 => HorizontalAlignment::Right,
            _ => return None,
        };
        let vertical = match numpad {
            7..=9 => VerticalAlignment::Top,
            4..=6 => VerticalAlignment::Middle,
            _ => VerticalAlignment::Bottom,
        };
        Some(Self {
            numpad,
            horizontal,
            vertical,
        })
    }
}
/// Imported static cue styling. Sizes and margins are ratios of the ASS script
/// resolution so the values survive a different project canvas; `None` style
/// means the caller owns presentation defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CueStyle {
    pub font_family: NfcText,
    /// Font size divided by PlayResY.
    pub font_size_ratio: f32,
    pub primary_color: [u8; 4],
    pub outline_color: [u8; 4],
    pub background_color: [u8; 4],
    /// 1 = outline/shadow, 3 = opaque box.
    pub border_style: u8,
    pub alignment: CueAlignment,
    pub margin_left_ratio: f32,
    pub margin_right_ratio: f32,
    pub margin_vertical_ratio: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
}
impl CueStyle {
    /// Parser and persistence guard for hostile finite values.
    pub fn is_valid(&self) -> bool {
        let bounded = |value: f32, max: f32| value.is_finite() && (0.0..=max).contains(&value);
        !self.font_family.as_str().trim().is_empty()
            && self.font_family.as_str().len() <= 256
            && bounded(self.font_size_ratio, 10.0)
            && self.font_size_ratio > 0.0
            && (1..=3).contains(&self.border_style)
            && CueAlignment::from_numpad(self.alignment.numpad) == Some(self.alignment)
            && bounded(self.margin_left_ratio, 10.0)
            && bounded(self.margin_right_ratio, 10.0)
            && bounded(self.margin_vertical_ratio, 10.0)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Cue {
    pub id: NfcText,
    pub range: SourceRange,
    pub source_lang: LanguageTag,
    pub texts: BTreeMap<LanguageTag, CueText>,
    /// Static style preserved from an ASS import; absent for generated/SRT/VTT cues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<CueStyle>,
}
impl Cue {
    /// Automatic results refuse to overwrite edited text; explicit replacement opts in.
    pub fn set_text(&mut self, lang: LanguageTag, text: CueText, replace_edited: bool) -> bool {
        if !replace_edited && self.texts.get(&lang).is_some_and(|t| t.edited) {
            return false;
        }
        self.texts.insert(lang, text);
        true
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CueTrack {
    pub cues: Vec<Cue>,
}
/// Recoverable categories, with one-based cue/line positions and no user text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub position: usize,
    pub kind: WarningKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarningKind {
    Encoding,
    Bom,
    LineEndings,
    Replacement,
    Malformed,
    Metadata,
    MissingHeader,
    NonMonotonic,
    Overlap,
    Duration,
    NegativeStart,
    EmptyText,
    MissingLanguage,
    RoundedDuration,
    TextLayout,
    ReadingSpeed,
    MinimumGap,
    EstimatedTiming,
}
impl CueTrack {
    /// Stable-sort by start, clamp negative starts, then shift overlaps preserving duration.
    pub fn repair(&mut self) -> Result<Vec<Warning>, Error> {
        let mut warnings = Vec::new();
        if self.cues.windows(2).any(|w| {
            w.first()
                .zip(w.get(1))
                .is_some_and(|(a, b)| a.range.start() > b.range.start())
        }) {
            warnings.push(Warning {
                position: 0,
                kind: WarningKind::NonMonotonic,
            });
        }
        self.cues.sort_by_key(|c| c.range.start());
        let mut end = Tick::ZERO;
        for (i, cue) in self.cues.iter_mut().enumerate() {
            let original = cue.range.start();
            let start = original.max(Tick::ZERO).max(end);
            if original < Tick::ZERO {
                warnings.push(Warning {
                    position: i + 1,
                    kind: WarningKind::NegativeStart,
                });
            }
            if original.max(Tick::ZERO) < end {
                warnings.push(Warning {
                    position: i + 1,
                    kind: WarningKind::Overlap,
                });
            }
            end = start
                .checked_add(cue.range.duration())
                .ok_or(Error::Invalid("repair overflow"))?;
            cue.range = SourceRange::new(start, end)?;
        }
        Ok(warnings)
    }
}
