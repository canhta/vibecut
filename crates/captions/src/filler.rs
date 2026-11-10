//! Whole-word filler candidates from per-language lexicon data over cue word timings.

use crate::{Cue, CueText, CueTrack, Error, LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::SourceRange;

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

/// Whether a lexicon phrase is a filled pause or a meaning-bearing marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FillerKind {
    /// A hesitation sound such as `um`, `ừm` or `嗯`.
    FilledPause,
    /// A normal word or phrase that can also act as a discourse marker.
    DiscourseMarker,
}

/// One whole-word filler span awaiting review for removal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FillerCandidate {
    /// Stable for a cue and word span: `filler:<cue id>:<start tick>`.
    pub id: NfcText,
    /// The source word span; adjacent filled pauses merge into one span.
    pub range: SourceRange,
    /// The cue that owns every word in `range`.
    pub cue_id: NfcText,
    /// Whether the detected language's text on that cue was edited.
    pub edited_cue: bool,
    /// Whether the match is a filled pause or a discourse marker.
    pub kind: FillerKind,
}

/// Validated per-language lexicon data; phrases match whole normalized words.
#[derive(Clone, Debug)]
pub struct FillerLexicon {
    language: LanguageTag,
    entries: Vec<LexiconEntry>,
}
#[derive(Clone, Debug)]
struct LexiconEntry {
    tokens: Vec<String>,
    kind: FillerKind,
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
    kind: FillerKind,
    references: Vec<String>,
}

impl FillerLexicon {
    /// Parse and validate one lexicon file. Every entry declares a kind and
    /// cites a nonempty header reference; phrases are NFC, lowercased and
    /// tokenized here.
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
        let mut entries: Vec<LexiconEntry> = Vec::with_capacity(wire.entries.len());
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
            if entries.iter().any(|existing| existing.tokens == tokens) {
                return Err(Error::MalformedLexicon("duplicate phrase"));
            }
            entries.push(LexiconEntry {
                tokens,
                kind: entry.kind,
            });
        }
        entries.sort_by(|a, b| {
            b.tokens
                .len()
                .cmp(&a.tokens.len())
                .then_with(|| a.tokens.cmp(&b.tokens))
        });
        Ok(Self { language, entries })
    }

    /// The shipped lexicon for a language tag: its primary subtag selects the
    /// reviewed data file, and the full requested tag is retained although cues
    /// are matched by primary subtag.
    pub fn builtin(language: &LanguageTag) -> Result<Self, Error> {
        let json = BUILTIN
            .iter()
            .find(|(tag, _)| *tag == language.primary())
            .map(|(_, json)| *json)
            .ok_or(Error::NoFillerLexicon)?;
        let mut lexicon = Self::parse(json)?;
        lexicon.language = language.clone();
        Ok(lexicon)
    }
}

/// Detect whole-word filler candidates in source order. Each cue whose source
/// language shares the lexicon's primary subtag contributes its source text
/// (`Cue::source_text`), tokenized with UAX #29 and paired index-for-index with
/// `Cue::word_ranges`; a cue whose counts disagree has no measured word timing
/// and contributes nothing. Only `FilledPause` entries match unless
/// `include_markers` is set, when `DiscourseMarker` entries also match. The
/// longest eligible phrase wins. Contiguous filled pauses merge into one
/// candidate; each marker stays separate because it is its own review decision.
pub fn detect_fillers(
    track: &CueTrack,
    lexicon: &FillerLexicon,
    include_markers: bool,
) -> Result<Vec<FillerCandidate>, Error> {
    let mut output = Vec::new();
    let mut track_tokens = 0usize;
    for cue in &track.cues {
        // Word ranges are recognition timing; a lexicon for another language
        // must never pair with this cue's source text.
        if lexicon.language.primary() != cue.source_lang.primary() {
            continue;
        }
        let Some(text) = cue.source_text() else {
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
        let mut run: Option<(usize, usize, FillerKind)> = None;
        let mut index = 0usize;
        while index < tokens.len() {
            if let Some((end, kind)) = match_phrase(&tokens, index, lexicon, include_markers) {
                run = match run {
                    Some((start, run_end, run_kind))
                        if kind == FillerKind::FilledPause
                            && run_kind == kind
                            && run_end == index =>
                    {
                        Some((start, end, kind))
                    }
                    _ => {
                        flush(&mut output, cue, text, run.take())?;
                        Some((index, end, kind))
                    }
                };
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

fn match_phrase(
    tokens: &[String],
    index: usize,
    lexicon: &FillerLexicon,
    include_markers: bool,
) -> Option<(usize, FillerKind)> {
    lexicon.entries.iter().find_map(|entry| {
        if !include_markers && entry.kind != FillerKind::FilledPause {
            return None;
        }
        let window = tokens.get(index..index + entry.tokens.len())?;
        window
            .iter()
            .zip(&entry.tokens)
            .all(|(token, word)| token == word)
            .then_some((index + entry.tokens.len(), entry.kind))
    })
}

fn flush(
    output: &mut Vec<FillerCandidate>,
    cue: &Cue,
    text: &CueText,
    run: Option<(usize, usize, FillerKind)>,
) -> Result<(), Error> {
    let Some((start, end, kind)) = run else {
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
        kind,
    });
    Ok(())
}

/// UAX #29 word segments that contain a letter or digit, NFC and lowercased.
fn tokenize(text: &str) -> Vec<String> {
    crate::word_tokens(text)
        .map(|token| {
            token
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect()
}
