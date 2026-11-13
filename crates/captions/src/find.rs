//! Find-replace over each cue's source text; cue timings and translations never move.

use crate::{Cue, CueText, CueTrack, Error, LanguageTag, NfcText};
use std::collections::BTreeSet;
use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

/// Largest find or replacement field, in bytes; longer input is refused before matching.
const MAX_FIELD_BYTES: usize = 512;
/// Largest cue text after a replacement: the per-cue limit the caption editor applies to typed text.
const MAX_CUE_TEXT_BYTES: usize = 64 * 1024;

/// Matching options. Both default off: exact, case-sensitive substring matching.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    /// Off compares each character's simple lowercase form, which covers Vietnamese and Chinese.
    pub case_sensitive: bool,
    /// Neither neighbouring character may be a letter, digit, underscore or combining mark.
    /// Unspaced Han text has no inner boundary, so a run of Han counts as one word.
    pub whole_word: bool,
}

/// What a replacement does to one cue that has matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CueOutcome {
    /// The source text after every match is replaced; the cue is written and marked edited.
    Replace(NfcText),
    /// The cue's source text is edited, so it is skipped and reported.
    Protected,
    /// The result would be empty or whitespace only, so it is skipped and reported.
    Emptied,
    /// The result would exceed the per-cue text limit, so it is skipped and reported.
    TooLong,
    /// The result equals the current text, so nothing is written.
    Unchanged,
}

/// One cue with at least one match in scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CuePlan {
    pub cue: NfcText,
    pub matches: usize,
    pub outcome: CueOutcome,
}

/// A preview of one find-replace: every match in scope and what each cue would become.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindPlan {
    /// Matches in scope, whatever their outcome.
    pub matches: usize,
    /// Cues with at least one match, in track order.
    pub cues: Vec<CuePlan>,
}

impl FindPlan {
    /// Matches in the cues that the replacement writes.
    pub fn replaced_matches(&self) -> usize {
        self.cues
            .iter()
            .filter(|cue| matches!(cue.outcome, CueOutcome::Replace(_)))
            .map(|cue| cue.matches)
            .sum()
    }

    /// The number of cues whose outcome satisfies `which`.
    pub fn cues_with(&self, which: fn(&CueOutcome) -> bool) -> usize {
        self.cues.iter().filter(|cue| which(&cue.outcome)).count()
    }
}

/// Previews a replacement without writing anything. Matches are searched in each cue's source
/// text (its own language, else the same primary subtag), only in the cues of `scope` when it
/// is given. Empty find text and oversized fields are refused.
pub fn find_plan(
    track: &CueTrack,
    scope: Option<&BTreeSet<NfcText>>,
    find: &str,
    with: &str,
    options: FindOptions,
) -> Result<FindPlan, Error> {
    let find = NfcText::from(find);
    let with = NfcText::from(with);
    if find.as_str().is_empty()
        || find.as_str().len() > MAX_FIELD_BYTES
        || with.as_str().len() > MAX_FIELD_BYTES
    {
        return Err(Error::Invalid("find or replacement text"));
    }
    let needle: Vec<char> = find.as_str().chars().collect();
    let mut plan = FindPlan::default();
    for cue in &track.cues {
        if scope.is_some_and(|ids| !ids.contains(&cue.id)) {
            continue;
        }
        let Some(key) = source_key(cue) else {
            continue;
        };
        let Some(text) = cue.texts.get(key) else {
            continue;
        };
        let spans = occurrences(text.text.as_str(), &needle, options);
        if spans.is_empty() {
            continue;
        }
        let outcome = if text.edited {
            CueOutcome::Protected
        } else {
            let next = NfcText::from(substitute(text.text.as_str(), &spans, with.as_str()));
            if next == text.text {
                CueOutcome::Unchanged
            } else if next.as_str().trim().is_empty() {
                CueOutcome::Emptied
            } else if next.as_str().len() > MAX_CUE_TEXT_BYTES {
                CueOutcome::TooLong
            } else {
                CueOutcome::Replace(next)
            }
        };
        plan.matches += spans.len();
        plan.cues.push(CuePlan {
            cue: cue.id.clone(),
            matches: spans.len(),
            outcome,
        });
    }
    Ok(plan)
}

/// Runs [`find_plan`] and writes each replaceable cue: its source text changes, it is marked
/// edited and its translation reference stays. Timings, translations and every other cue are
/// untouched, so a batch is one value to write as one edit.
pub fn find_replace(
    track: &mut CueTrack,
    scope: Option<&BTreeSet<NfcText>>,
    find: &str,
    with: &str,
    options: FindOptions,
) -> Result<FindPlan, Error> {
    let preview = find_plan(track, scope, find, with, options)?;
    for entry in &preview.cues {
        let CueOutcome::Replace(text) = &entry.outcome else {
            continue;
        };
        let Some(cue) = track.cues.iter_mut().find(|cue| cue.id == entry.cue) else {
            continue;
        };
        let Some(key) = source_key(cue).cloned() else {
            continue;
        };
        let translation_ref = cue.texts.get(&key).and_then(|t| t.translation_ref.clone());
        cue.set_text(
            key,
            CueText {
                text: text.clone(),
                edited: true,
                translation_ref,
            },
            false,
        );
    }
    Ok(preview)
}

/// The key of a cue's source text: its own source language, else the same primary subtag.
fn source_key(cue: &Cue) -> Option<&LanguageTag> {
    cue.texts
        .keys()
        .find(|tag| **tag == cue.source_lang)
        .or_else(|| {
            cue.texts
                .keys()
                .find(|tag| tag.primary() == cue.source_lang.primary())
        })
}

/// Non-overlapping matches as byte ranges, left to right. A match starts and ends on a grapheme
/// boundary, so a combining mark is never separated from its base letter.
fn occurrences(text: &str, needle: &[char], options: FindOptions) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut boundaries: BTreeSet<usize> = text.grapheme_indices(true).map(|(at, _)| at).collect();
    boundaries.insert(text.len());
    let mut found = Vec::new();
    let mut next = 0;
    for start in 0..chars.len() {
        if start < next {
            continue;
        }
        let Some(rest) = chars.get(start..) else {
            continue;
        };
        let Some(width) = needle_width(rest, needle, options.case_sensitive) else {
            continue;
        };
        let Some(&(begin, _)) = chars.get(start) else {
            continue;
        };
        let end_index = start + width;
        let end = chars.get(end_index).map_or(text.len(), |&(at, _)| at);
        if !boundaries.contains(&begin) || !boundaries.contains(&end) {
            continue;
        }
        if options.whole_word {
            let before = text.get(..begin).and_then(|head| head.chars().next_back());
            let after = text.get(end..).and_then(|tail| tail.chars().next());
            if [before, after].into_iter().flatten().any(is_word_char) {
                continue;
            }
        }
        found.push((begin, end));
        next = end_index;
    }
    found
}

/// The number of characters a match consumes when `rest` begins with `needle`.
fn needle_width(rest: &[(usize, char)], needle: &[char], case_sensitive: bool) -> Option<usize> {
    let same = |a: char, b: char| {
        a == b || (!case_sensitive && Iterator::eq(a.to_lowercase(), b.to_lowercase()))
    };
    let matched =
        rest.len() >= needle.len() && rest.iter().zip(needle).all(|(&(_, a), &b)| same(a, b));
    matched.then_some(needle.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || is_combining_mark(c)
}

/// The text with each span replaced by `with`; spans are ascending and non-overlapping.
fn substitute(text: &str, spans: &[(usize, usize)], with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &(begin, end) in spans {
        out.push_str(text.get(last..begin).unwrap_or_default());
        out.push_str(with);
        last = end;
    }
    out.push_str(text.get(last..).unwrap_or_default());
    out
}
