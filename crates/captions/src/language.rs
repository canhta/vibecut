//! BCP 47 language tags and editable subtitle profiles.

use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Syntactically well-formed BCP 47 tag with standardized casing; unknown tags are allowed.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct LanguageTag(String);
impl LanguageTag {
    pub fn parse(value: &str) -> Result<Self, Error> {
        language_tags::LanguageTag::parse(value)
            .map(|tag| Self(tag.into_string()))
            .map_err(Error::Language)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The lowercase primary language subtag, e.g. `zh` for `zh-Hans` or `en`
    /// for `en-US`. [`LanguageTag::parse`] standardizes casing, so no caller
    /// needs to lowercase the result.
    pub fn primary(&self) -> &str {
        self.0.split('-').next().unwrap_or(self.as_str())
    }

    /// Whether adjacent words in this language are written with an inter-word
    /// space. Chinese and Japanese write Han and Kana runs without one; the
    /// segmentation policy already states "Chinese gets no inserted spaces".
    /// Source: CLDR script metadata (Han, Hiragana, Katakana and Bopomofo are
    /// unspaced scripts) plus `crates/captions/src/segment/README.md`.
    pub fn joins_with_space(&self) -> bool {
        !matches!(self.primary(), "zh" | "ja")
    }
}
impl TryFrom<String> for LanguageTag {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        Self::parse(&value)
    }
}
impl From<LanguageTag> for String {
    fn from(value: LanguageTag) -> Self {
        value.0
    }
}
/// Segmentation/layout policy data. Fonts are family preferences, not bundled assets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LanguageProfile {
    pub timing_unit: crate::TimingUnit,
    pub max_graphemes: u32,
    pub graphemes_per_second: u32,
    pub line_breaking: crate::NfcText,
    pub punctuation: crate::NfcText,
    pub word_segmenter: Option<crate::NfcText>,
    pub font_fallback: Vec<crate::NfcText>,
}
/// Extensible registry. Exact tag then progressively shorter prefixes, then generic.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanguageRegistry {
    pub profiles: BTreeMap<LanguageTag, LanguageProfile>,
    pub generic: LanguageProfile,
}
impl LanguageRegistry {
    /// Built-in policies are editable data; adding a language requires no code branch.
    pub fn standard() -> Result<Self, Error> {
        standard_profiles()
    }
    pub fn profile(&self, language: &LanguageTag) -> &LanguageProfile {
        let mut key = language.as_str();
        loop {
            if let Ok(tag) = LanguageTag::parse(key)
                && let Some(profile) = self.profiles.get(&tag)
            {
                return profile;
            }
            let Some((prefix, _)) = key.rsplit_once('-') else {
                return &self.generic;
            };
            key = prefix;
        }
    }
}
fn standard_profiles() -> Result<LanguageRegistry, Error> {
    let generic = LanguageProfile {
        timing_unit: crate::TimingUnit::Word,
        max_graphemes: 42,
        graphemes_per_second: 20,
        line_breaking: "unicode".into(),
        punctuation: ".!?".into(),
        word_segmenter: None,
        font_fallback: vec!["sans-serif".into()],
    };
    let mut profiles = BTreeMap::new();
    for (tag, unit, limit, speed, punctuation, font) in [
        (
            "vi",
            crate::TimingUnit::Syllable,
            42,
            17,
            ".!?…",
            "Noto Sans",
        ),
        ("en", crate::TimingUnit::Word, 42, 20, ".!?…", "Noto Sans"),
        (
            "zh",
            crate::TimingUnit::Character,
            16,
            9,
            "。！？…",
            "Noto Sans CJK SC",
        ),
        (
            "zh-Hans",
            crate::TimingUnit::Character,
            16,
            9,
            "。！？…",
            "Noto Sans CJK SC",
        ),
        (
            "zh-Hant",
            crate::TimingUnit::Character,
            16,
            9,
            "。！？…",
            "Noto Sans CJK TC",
        ),
    ] {
        profiles.insert(
            LanguageTag::parse(tag)?,
            LanguageProfile {
                timing_unit: unit,
                max_graphemes: limit,
                graphemes_per_second: speed,
                punctuation: punctuation.into(),
                font_fallback: vec![font.into(), "sans-serif".into()],
                ..generic.clone()
            },
        );
    }
    Ok(LanguageRegistry { profiles, generic })
}
