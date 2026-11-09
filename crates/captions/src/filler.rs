//! Whole-word filler candidates from per-language lexicon data over cue word timings.

use crate::{Cue, CueText, CueTrack, Error, LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::SourceRange;
use unicode_segmentation::UnicodeSegmentation;

/// Embedded lexicons are small reviewed data; a caller-provided file is capped
/// before parsing so hostile input cannot allocate unboundedly.
const MAX_LEXICON_BYTES: usize = 512 * 1024;
const MAX_ENTRIES: usize = 2_048;
const MAX_PHRASE_TOKENS: usize = 8;
const MAX_TOKEN_BYTES: usize = 128;
const MAX_TRACK_TOKENS: usize = 1_000_000;
const MAX_CANDIDATES: usize = 100_000;
/// Shipped lexicons by primary language subtag. Adding a language adds one file
/// and one row; the detector contract is already language-neutral.
const BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("../data/fillers/en.json")),
    ("vi", include_str!("../data/fillers/vi.json")),
    ("zh", include_str!("../data/fillers/zh.json")),
];

/// One whole-word filler span awaiting review for removal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FillerCandidate {
    /// Stable for a cue and word span: `filler:<cue id>:<start tick>`.
    pub id: NfcText,
    /// The source word span; adjacent filler words merge into one span.
    pub range: SourceRange,
    /// The cue that owns every word in `range`.
    pub cue_id: NfcText,
    /// Whether the detected language's text on that cue was edited.
    pub edited_cue: bool,
}

/// Validated per-language lexicon data; phrases match whole normalized words.
#[derive(Clone, Debug)]
pub struct FillerLexicon {
    language: LanguageTag,
    entries: Vec<Vec<String>>,
}
/// The one data-file shape; every phrase names references declared in the header.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    language: String,
    references: BTreeMap<String, String>,
    entries: Vec<WireEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEntry {
    phrase: String,
    references: Vec<String>,
}

impl FillerLexicon {
    /// Parse and validate one lexicon file. Every entry must cite a nonempty
    /// header reference; phrases are NFC, lowercased and tokenized here.
    pub fn parse(json: &str) -> Result<Self, Error> {
        if json.len() > MAX_LEXICON_BYTES {
            return Err(Error::MalformedLexicon("size"));
        }
        let wire: Wire = serde_json::from_str(json).map_err(|_| Error::MalformedLexicon("json"))?;
        let language =
            LanguageTag::parse(&wire.language).map_err(|_| Error::MalformedLexicon("language"))?;
        if wire.entries.is_empty() || wire.entries.len() > MAX_ENTRIES {
            return Err(Error::MalformedLexicon("entry count"));
        }
        if wire
            .references
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            return Err(Error::MalformedLexicon("reference"));
        }
        let mut entries = Vec::with_capacity(wire.entries.len());
        for entry in wire.entries {
            if entry.references.is_empty()
                || entry
                    .references
                    .iter()
                    .any(|key| !wire.references.contains_key(key))
            {
                return Err(Error::MalformedLexicon("entry reference"));
            }
            let tokens = tokenize(NfcText::from(entry.phrase.as_str()).as_str());
            if tokens.is_empty() || tokens.len() > MAX_PHRASE_TOKENS {
                return Err(Error::MalformedLexicon("phrase"));
            }
            if tokens
                .iter()
                .any(|token| token.len() > MAX_TOKEN_BYTES || token.is_empty())
            {
                return Err(Error::MalformedLexicon("phrase token"));
            }
            if entries.iter().any(|existing| existing == &tokens) {
                return Err(Error::MalformedLexicon("duplicate phrase"));
            }
            entries.push(tokens);
        }
        entries.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        Ok(Self { language, entries })
    }

    /// The shipped lexicon for a language tag: its primary subtag selects the
    /// reviewed data file, and the full requested tag is retained for cue lookup.
    pub fn builtin(language: &LanguageTag) -> Result<Self, Error> {
        let primary = language
            .as_str()
            .split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let json = BUILTIN
            .iter()
            .find(|(tag, _)| *tag == primary)
            .map(|(_, json)| *json)
            .ok_or(Error::NoFillerLexicon)?;
        let mut lexicon = Self::parse(json)?;
        lexicon.language = language.clone();
        Ok(lexicon)
    }
}

/// Detect whole-word filler candidates in source order. Every cue's text in the
/// lexicon language is tokenized with UAX #29 and paired index-for-index with
/// `Cue::word_ranges`; a cue whose counts disagree has no measured word timing
/// and contributes nothing. Longest phrase wins, and contiguous filler words
/// merge into one candidate.
pub fn detect_fillers(
    track: &CueTrack,
    lexicon: &FillerLexicon,
) -> Result<Vec<FillerCandidate>, Error> {
    let mut output = Vec::new();
    let mut track_tokens = 0usize;
    for cue in &track.cues {
        let Some(text) = cue_text(cue, &lexicon.language) else {
            continue;
        };
        let tokens = tokenize(text.text.as_str());
        track_tokens = track_tokens
            .checked_add(tokens.len())
            .filter(|count| *count <= MAX_TRACK_TOKENS)
            .ok_or(Error::Invalid("filler word limit"))?;
        if tokens.is_empty() || tokens.len() != cue.word_ranges.len() {
            continue;
        }
        let mut run: Option<(usize, usize)> = None;
        let mut index = 0usize;
        while index < tokens.len() {
            if let Some(end) = match_phrase(&tokens, index, lexicon) {
                let start = run.map_or(index, |(start, _)| start);
                run = Some((start, end));
                index = end;
            } else {
                flush(&mut output, cue, text, run.take())?;
                index += 1;
            }
        }
        flush(&mut output, cue, text, run)?;
    }
    Ok(output)
}

fn cue_text<'a>(cue: &'a Cue, language: &LanguageTag) -> Option<&'a CueText> {
    let (tag, text) = match cue.texts.get_key_value(language) {
        Some(entry) => entry,
        None => {
            let wanted = primary(language.as_str());
            cue.texts
                .iter()
                .find(|(tag, _)| primary(tag.as_str()) == wanted)
                .map(|(tag, text)| (tag, text))?
        }
    };
    // Word ranges are recognition timing; a translation's text must never be
    // paired with the source language's word times.
    (primary(tag.as_str()) == primary(cue.source_lang.as_str())).then_some(text)
}

fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
}

fn match_phrase(tokens: &[String], index: usize, lexicon: &FillerLexicon) -> Option<usize> {
    lexicon.entries.iter().find_map(|entry| {
        let window = tokens.get(index..index + entry.len())?;
        window
            .iter()
            .zip(entry)
            .all(|(token, word)| token == word)
            .then_some(index + entry.len())
    })
}

fn flush(
    output: &mut Vec<FillerCandidate>,
    cue: &Cue,
    text: &CueText,
    run: Option<(usize, usize)>,
) -> Result<(), Error> {
    let Some((start, end)) = run else {
        return Ok(());
    };
    let first = cue
        .word_ranges
        .get(start)
        .ok_or(Error::Invalid("filler word span"))?;
    let last = cue
        .word_ranges
        .get(
            end.checked_sub(1)
                .ok_or(Error::Invalid("filler word span"))?,
        )
        .ok_or(Error::Invalid("filler word span"))?;
    let range = SourceRange::new(first.start(), last.end())?;
    if output.len() >= MAX_CANDIDATES {
        return Err(Error::Invalid("filler candidate limit"));
    }
    output.push(FillerCandidate {
        id: format!("filler:{}:{}", cue.id.as_str(), range.start().0).into(),
        range,
        cue_id: cue.id.clone(),
        edited_cue: text.edited,
    });
    Ok(())
}

/// UAX #29 word segments that contain a letter or digit, NFC and lowercased.
fn tokenize(text: &str) -> Vec<String> {
    text.split_word_bound_indices()
        .filter_map(|(_, token)| {
            let folded = token
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            (!folded.is_empty() && folded.chars().any(char::is_alphanumeric)).then_some(folded)
        })
        .collect()
}
