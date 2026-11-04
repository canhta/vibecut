//! Source-timed transcripts, multilingual cues and forgiving subtitle interchange.
mod io;
mod language;
mod mapping;
mod segment;
mod span;
mod timing;
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Cue {
    pub id: NfcText,
    pub range: SourceRange,
    pub source_lang: LanguageTag,
    pub texts: BTreeMap<LanguageTag, CueText>,
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
