//! Deterministic caption keyword emphasis over a cue's verified word timings.

use crate::{Cue, CueWordTiming, Error, LanguageTag, VerifiedCueWord};
use std::collections::BTreeSet;

/// A cue carries at most this many emphasised words.
pub const MAX_EMPHASIS_WORDS: usize = 3;
/// Embedded lexicon files are reviewed data; a caller file is capped before parsing.
const MAX_LEXICON_BYTES: usize = 512 * 1024;
/// Longest word, in characters, that still adds length score.
const LENGTH_CAP: usize = 12;
const DIGIT_BONUS: usize = 20;
const ACRONYM_BONUS: usize = 15;
const PROPER_NOUN_BONUS: usize = 10;

const BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("../data/emphasis/en.json")),
    ("vi", include_str!("../data/emphasis/vi.json")),
    ("zh", include_str!("../data/emphasis/zh.json")),
];

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    language: String,
    /// Where the list comes from, so an update stays inside the data file.
    source: String,
    stopwords: Vec<String>,
}

/// Per-language stopwords: words that carry no emphasis by themselves.
#[derive(Clone, Debug)]
pub struct EmphasisLexicon {
    language: LanguageTag,
    stopwords: BTreeSet<String>,
}

impl EmphasisLexicon {
    /// Parse one lexicon file. Words are NFC and lowercased; the header must cite a source.
    pub fn parse(json: &str) -> Result<Self, Error> {
        if json.len() > MAX_LEXICON_BYTES {
            return Err(Error::MalformedLexicon("size"));
        }
        let wire: Wire = serde_json::from_str(json).map_err(|_| Error::MalformedLexicon("json"))?;
        let language =
            LanguageTag::parse(&wire.language).map_err(|_| Error::MalformedLexicon("language"))?;
        if wire.source.trim().is_empty() || wire.stopwords.is_empty() {
            return Err(Error::MalformedLexicon("source or words"));
        }
        let stopwords = wire
            .stopwords
            .iter()
            .map(|word| normalize(word))
            .filter(|word| !word.is_empty())
            .collect();
        Ok(Self {
            language,
            stopwords,
        })
    }

    /// The shipped lexicon for a language's primary subtag.
    pub fn builtin(language: &LanguageTag) -> Result<Self, Error> {
        let json = BUILTIN
            .iter()
            .find(|(tag, _)| *tag == language.primary())
            .map(|(_, json)| *json)
            .ok_or(Error::NoEmphasisLexicon)?;
        Self::parse(json)
    }

    pub fn language(&self) -> &LanguageTag {
        &self.language
    }

    fn is_stopword(&self, word: &str) -> bool {
        self.stopwords.contains(word)
    }
}

/// Lowercased word with edge punctuation removed.
fn normalize(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Verified word records of a cue whose source text still matches them.
fn verified_words(cue: &Cue) -> Result<(&str, &[VerifiedCueWord]), Error> {
    let CueWordTiming::Words { text, records, .. } = &cue.word_timing else {
        return Err(Error::NoWordTiming);
    };
    let source = cue.source_text().ok_or(Error::NoWordTiming)?;
    if records.is_empty() || source.text != *text {
        return Err(Error::NoWordTiming);
    }
    Ok((text.as_str(), records))
}

/// How many words a cue of `count` timed words may emphasise: none for one word, one for
/// two to five, two for six to nine and three from ten ([`MAX_EMPHASIS_WORDS`] at most).
pub fn emphasis_limit(count: usize) -> usize {
    ((count + 2) / 4).min(MAX_EMPHASIS_WORDS)
}

/// Pick up to [`MAX_EMPHASIS_WORDS`] emphasis word ids for one cue, deterministically.
///
/// Ids index the cue's verified `CueWordTiming::Words` records, so a selection is always a
/// subset of timed words. A cue without verified word timing refuses with
/// [`Error::NoWordTiming`] and never gets an invented selection. A word scores by length
/// (capped), digits, all-capital acronyms and a capital letter inside the cue; stopwords never
/// score. Ties keep the earlier word. The result is ascending.
pub fn select_lexicon(cue: &Cue, lexicon: Option<&EmphasisLexicon>) -> Result<Vec<u32>, Error> {
    let (text, records) = verified_words(cue)?;
    let count = records.len();
    let mut scored: Vec<(usize, usize)> = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let Some(raw) = text.get(record.bytes.clone()) else {
            return Err(Error::NoWordTiming);
        };
        let word = normalize(raw);
        if word.is_empty() || lexicon.is_some_and(|lexicon| lexicon.is_stopword(&word)) {
            continue;
        }
        let letters: Vec<char> = raw.chars().filter(|c| c.is_alphanumeric()).collect();
        let mut score = word.chars().count().min(LENGTH_CAP);
        if letters.iter().any(char::is_ascii_digit) {
            score += DIGIT_BONUS;
        }
        let upper = letters.iter().filter(|c| c.is_uppercase()).count();
        if letters.len() >= 2 && upper == letters.len() {
            score += ACRONYM_BONUS;
        } else if index > 0 && letters.first().is_some_and(|c| c.is_uppercase()) {
            score += PROPER_NOUN_BONUS;
        }
        scored.push((index, score));
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(emphasis_limit(count));
    let mut ids: Vec<u32> = scored
        .into_iter()
        .map(|(index, _)| u32::try_from(index).unwrap_or(u32::MAX))
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Whether `word_ids` is a valid stored selection for `cue`: ascending, unique, at most
/// [`MAX_EMPHASIS_WORDS`], and every id a verified timed word of the cue's current source text.
pub fn validate_selection(cue: &Cue, word_ids: &[u32]) -> Result<(), Error> {
    let (_, records) = verified_words(cue)?;
    let count = records.len();
    if word_ids.len() > MAX_EMPHASIS_WORDS
        || word_ids
            .windows(2)
            .any(|pair| matches!(pair, [a, b] if a >= b))
        || word_ids.iter().any(|&id| id as usize >= count)
    {
        return Err(Error::Invalid("emphasis word ids"));
    }
    Ok(())
}

/// The stored selection when it is still valid for the cue, else nothing. Render and export
/// read emphasis only through this check, so ids that outlived their words are never drawn.
pub fn resolve_selection<'a>(cue: &Cue, word_ids: &'a [u32]) -> &'a [u32] {
    if validate_selection(cue, word_ids).is_ok() {
        word_ids
    } else {
        &[]
    }
}
