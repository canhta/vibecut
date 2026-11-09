//! Reads caption cues and measured words in output time.
use crate::{EditError, Result, Session};
use captions::{CueText, LanguageTag, NfcText};
use project::{ItemId, ItemParams, ProjectData, SceneId, TrackId, WordSpan};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::SourceRange;

/// Stable address of one measured word: its item, retained cue and index in the
/// cue's `word_ranges`. Text is never part of an address.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct WordId {
    pub item: ItemId,
    pub cue: NfcText,
    pub word: usize,
}

/// One measured word in output time. `text` is absent when the cue text does not
/// pair token-for-token with its measured ranges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TranscriptWord {
    pub id: WordId,
    pub text: Option<NfcText>,
    pub output: SourceRange,
    /// The cue's source text was edited, so the word range refuses cuts.
    pub protected: bool,
    /// The owning track is locked.
    pub locked: bool,
}

/// One retained cue in output time with its measured words.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TranscriptCue {
    pub item: ItemId,
    pub track: TrackId,
    pub id: NfcText,
    pub output: SourceRange,
    pub texts: BTreeMap<LanguageTag, CueText>,
    pub protected: bool,
    pub locked: bool,
    pub words: Vec<TranscriptWord>,
}

/// Caption cues of the active scene in output time, sorted by start.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TranscriptView {
    pub scene: SceneId,
    pub cues: Vec<TranscriptCue>,
}

impl Session {
    /// Cues and measured words of the active scene in output time. Cue text,
    /// languages, edit flags and native word timings are read, never rewritten.
    ///
    /// # Errors
    ///
    /// Returns an error when a retained item's mapping or range is unrepresentable.
    pub fn transcript(&self) -> Result<TranscriptView> {
        let scene = self.project().data().scene();
        let mut cues = Vec::new();
        for track in scene.timeline.iter() {
            for item in &track.items {
                let ItemParams::Caption(params) = &item.content else {
                    continue;
                };
                let words = item.word_spans()?;
                for cue in &params.cues.cues {
                    let Some(range) = item.source_range.intersect(cue.range) else {
                        continue;
                    };
                    let output = SourceRange::new(
                        item.output_time_of(range.start())?,
                        item.output_time_of(range.end())?,
                    )?;
                    if output.start() >= output.end() {
                        continue;
                    }
                    let tokens = cue.words();
                    let protected = cue.source_text().is_some_and(|text| text.edited);
                    let cue_words = words
                        .iter()
                        .filter(|word| word.cue == cue.id)
                        .map(|word| TranscriptWord {
                            id: WordId {
                                item: item.id,
                                cue: cue.id.clone(),
                                word: word.index,
                            },
                            text: tokens.get(word.index).map(|token| token.text.clone()),
                            output: word.output,
                            protected,
                            locked: track.locked,
                        })
                        .collect();
                    cues.push(TranscriptCue {
                        item: item.id,
                        track: track.id,
                        id: cue.id.clone(),
                        output,
                        texts: cue.texts.clone(),
                        protected,
                        locked: track.locked,
                        words: cue_words,
                    });
                }
            }
        }
        cues.sort_by_key(|cue| cue.output.start());
        Ok(TranscriptView {
            scene: scene.id,
            cues,
        })
    }
}

/// Every measured word of a scene, sorted by output start then identity.
pub(crate) fn scene_words(scene: &project::Scene) -> Result<Vec<WordSpan>> {
    let mut words = Vec::new();
    for item in scene.timeline.iter().flat_map(|track| &track.items) {
        words.extend(item.word_spans()?);
    }
    words.sort_by_key(|word| (word.output.start(), word.item, word.index));
    Ok(words)
}

/// Resolve one selection against `all`: every id must address a measured word of
/// the active scene, be unique, and sit in a cue whose source text is not edited.
/// An id whose item lives in another scene refuses with `SceneBoundary`.
pub(super) fn selected_words(
    data: &ProjectData,
    all: &[WordSpan],
    ids: &[WordId],
) -> Result<BTreeSet<WordId>> {
    if ids.is_empty() {
        return Err(EditError::Invalid("empty transcript selection"));
    }
    let scene = data.scene();
    let mut selected = BTreeSet::new();
    for id in ids {
        if !selected.insert(id.clone()) {
            return Err(EditError::Invalid("duplicate transcript word"));
        }
        let in_scene = scene
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .any(|item| item.id == id.item);
        if !in_scene {
            let elsewhere = data
                .scenes
                .iter()
                .filter(|other| other.id != scene.id)
                .flat_map(|other| other.timeline.iter())
                .flat_map(|track| &track.items)
                .any(|item| item.id == id.item);
            return if elsewhere {
                Err(EditError::SceneBoundary)
            } else {
                Err(EditError::NoItem(id.item))
            };
        }
        let word = all
            .iter()
            .find(|word| word.item == id.item && word.cue == id.cue && word.index == id.word)
            .ok_or(EditError::Invalid("transcript word"))?;
        let item = scene
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|item| item.id == word.item)
            .ok_or(EditError::NoItem(word.item))?;
        let ItemParams::Caption(params) = &item.content else {
            return Err(EditError::Invalid("transcript word item"));
        };
        let cue = params
            .cues
            .cues
            .iter()
            .find(|cue| cue.id == word.cue)
            .ok_or(EditError::Invalid("transcript word cue"))?;
        if cue.source_text().is_some_and(|text| text.edited) {
            return Err(EditError::Protected);
        }
    }
    Ok(selected)
}
