//! Reads lexicon filler candidates in output time and resolves reviewed ids to measured words.

use crate::{EditError, Result, WordId};
use captions::{FillerKind, FillerLexicon, LanguageTag, NfcText};
use project::{Item, ItemId, ItemParams, ProjectData, WordSpan};
use std::collections::BTreeMap;
use time::{SourceRange, Tick};

/// One visible filler occurrence in output time and the measured words its removal
/// selects. The id is stable for the source cue and word start; the owning item
/// keeps repeated source occurrences distinct.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FillerOccurrence {
    pub id: NfcText,
    pub item: ItemId,
    pub cue: NfcText,
    pub output: SourceRange,
    pub kind: FillerKind,
    pub edited_cue: bool,
    words: Vec<WordId>,
}

impl FillerOccurrence {
    /// Every measured word this occurrence covers, in cue order.
    pub(crate) fn words(&self) -> &[WordId] {
        &self.words
    }
}

/// Active-scene filler occurrences in output time. `language` selects one shipped
/// lexicon and refuses an unknown language; `None` uses every shipped lexicon a
/// cue of the scene names, which is how removal resolves opaque ids without a
/// language argument. Only visible measured words qualify, so a trim never yields
/// a candidate outside the item's output span.
pub(crate) fn scene_candidates(
    data: &ProjectData,
    language: Option<&LanguageTag>,
    include_markers: bool,
) -> Result<Vec<FillerOccurrence>> {
    let lexicons = match language {
        Some(language) => vec![FillerLexicon::builtin(language)?],
        None => {
            let mut tags: BTreeMap<String, LanguageTag> = BTreeMap::new();
            for cue in data
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .filter_map(|item| match &item.content {
                    ItemParams::Caption(params) => Some(params.cues.cues.iter()),
                    _ => None,
                })
                .flatten()
            {
                tags.entry(primary(cue.source_lang.as_str()).to_owned())
                    .or_insert_with(|| cue.source_lang.clone());
            }
            tags.into_values()
                .filter_map(|tag| FillerLexicon::builtin(&tag).ok())
                .collect()
        }
    };
    let mut occurrences = Vec::new();
    for item in data.scene().timeline.iter().flat_map(|track| &track.items) {
        item_candidates(item, &lexicons, include_markers, &mut occurrences)?;
    }
    occurrences.sort_by_key(|candidate| (candidate.output.start(), candidate.item));
    Ok(occurrences)
}

fn item_candidates(
    item: &Item,
    lexicons: &[FillerLexicon],
    include_markers: bool,
    occurrences: &mut Vec<FillerOccurrence>,
) -> Result<()> {
    let ItemParams::Caption(params) = &item.content else {
        return Ok(());
    };
    let words = item.word_spans()?;
    for lexicon in lexicons {
        for candidate in captions::detect_fillers(&params.cues, lexicon, include_markers)? {
            let Some(cue) = params
                .cues
                .cues
                .iter()
                .find(|cue| cue.id == candidate.cue_id)
            else {
                continue;
            };
            let selected: Vec<&WordSpan> = cue
                .word_ranges
                .iter()
                .enumerate()
                .filter(|(_, word)| {
                    word.start() >= candidate.range.start() && word.end() <= candidate.range.end()
                })
                .filter_map(|(index, _)| {
                    words
                        .iter()
                        .find(|word| word.cue == cue.id && word.index == index)
                })
                .collect();
            let (Some(first), Some(last)) = (selected.first(), selected.last()) else {
                // The whole candidate lies outside the item's retained output span.
                continue;
            };
            if occurrences.len() >= super::wire::MAX_COLLECTION {
                return Err(EditError::Limit);
            }
            let Some(range) = cue.word_ranges.get(first.index) else {
                continue;
            };
            occurrences.push(FillerOccurrence {
                id: candidate_id(item.id, &cue.id, range.start()),
                item: item.id,
                cue: cue.id.clone(),
                output: SourceRange::new(first.output.start(), last.output.end())?,
                kind: candidate.kind,
                edited_cue: candidate.edited_cue,
                words: selected
                    .iter()
                    .map(|word| WordId {
                        item: item.id,
                        cue: word.cue.clone(),
                        word: word.index,
                    })
                    .collect(),
            });
        }
    }
    Ok(())
}

/// Stable candidate identity: item, retained cue and the candidate's source start.
fn candidate_id(item: ItemId, cue: &NfcText, start: Tick) -> NfcText {
    format!("filler:{}:{}:{}", item.0, cue.as_str(), start.0).into()
}

/// Every visible measured word a reviewed candidate may select, keyed by the same
/// stable ID the local detector and the LLM classifier produce. LLM candidates are
/// not limited to the lexicon, so removal resolves those IDs here before cutting.
pub(crate) fn visible_words(data: &ProjectData) -> Result<BTreeMap<NfcText, (WordId, bool)>> {
    let mut output = BTreeMap::new();
    for item in data.scene().timeline.iter().flat_map(|track| &track.items) {
        let ItemParams::Caption(params) = &item.content else {
            continue;
        };
        let spans = item.word_spans()?;
        for cue in &params.cues.cues {
            let Some(text) = cue.source_text() else {
                continue;
            };
            let words = cue.words();
            if words.is_empty() || words.len() != cue.word_ranges.len() {
                continue;
            }
            for (index, word) in words.iter().enumerate() {
                let Some(_) = spans
                    .iter()
                    .find(|span| span.cue == cue.id && span.index == index)
                else {
                    continue;
                };
                output.insert(
                    candidate_id(item.id, &cue.id, word.range.start()),
                    (
                        WordId {
                            item: item.id,
                            cue: cue.id.clone(),
                            word: index,
                        },
                        text.edited,
                    ),
                );
            }
        }
    }
    Ok(output)
}

fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or_default()
}

impl From<FillerOccurrence> for super::wire::FillerCandidate {
    fn from(candidate: FillerOccurrence) -> Self {
        Self {
            id: candidate.id,
            item: candidate.item,
            cue: candidate.cue,
            range: super::wire::OutputSpan {
                start: candidate.output.start(),
                end: candidate.output.end(),
            },
            kind: candidate.kind,
            edited_cue: candidate.edited_cue,
        }
    }
}
