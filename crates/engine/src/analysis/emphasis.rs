//! Suggests caption keyword emphasis for one caption item's cues through one lent text
//! provider.
//!
//! One job snapshots the item's cues that carry verified word timing, validates the complete
//! bounded input, obtains the human consent token from the lent [`TextAccess`] and runs one
//! provider attempt. Returned positions must be words of the cue they came with. A refused
//! consent, a missing adapter or a provider failure fails the command or job with its typed
//! reason; the lexicon never stands in. Nothing mutates the project: `captions.set_emphasis`
//! applies a suggestion.
use crate::analysis::{Error as AnalysisError, Result};
use crate::{
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState},
    providers::TextAccess,
    session::wire::EmphasisSuggestion,
};
use ai::text::emphasis::{
    EmphasisInput, EmphasisRefusal, EmphasisSegment, EmphasisSelection, EmphasisSettings,
    EmphasisWord,
};
use captions::{CueWordTiming, NfcText};
use project::{ItemId, ItemParams, Project};
use std::sync::Arc;

const MAX_RESULT_BYTES: usize = 1 << 20;
const RECIPE: &str = "emphasis-v1:verified-words:item";

/// One immutable caption item's emphasis request. It is not cached; the key only identifies the
/// job snapshot.
#[derive(Clone)]
pub struct EmphasisRequest {
    item: ItemId,
    input: EmphasisInput,
    key: CacheKey,
}
/// Retained runner job; `finish` decodes only a complete validated suggestion list.
pub struct EmphasisJob {
    pub id: JobId,
    pub cancel: CancelToken,
}

impl EmphasisRequest {
    /// Snapshot the item's cues with verified word timing and validate them. `cues` names the
    /// cues to cover; `None` covers every eligible cue. A named cue that is missing or lacks
    /// verified word timing refuses with the typed reason before any provider work, as does
    /// an item with no eligible cue.
    pub fn new(project: &Arc<Project>, item: ItemId, cues: Option<&[NfcText]>) -> Result<Self> {
        let params = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .and_then(|candidate| match &candidate.content {
                ItemParams::Caption(params) => Some(params),
                _ => None,
            })
            .ok_or(AnalysisError::Edit(crate::session::EditError::Invalid(
                "emphasis requires caption item",
            )))?;
        if let Some(named) = cues {
            for id in named {
                let cue = params.cues.cues.iter().find(|cue| &cue.id == id).ok_or(
                    AnalysisError::Edit(crate::session::EditError::Invalid("emphasis cue")),
                )?;
                captions::validate_selection(cue, &[])
                    .map_err(|error| AnalysisError::Edit(error.into()))?;
            }
        }
        let mut language = None;
        let mut segments = Vec::new();
        for cue in &params.cues.cues {
            if cues.is_some_and(|named| !named.contains(&cue.id))
                || captions::validate_selection(cue, &[]).is_err()
            {
                continue;
            }
            let CueWordTiming::Words { text, records, .. } = &cue.word_timing else {
                continue;
            };
            let language = language.get_or_insert_with(|| cue.source_lang.clone());
            if language.primary() != cue.source_lang.primary() {
                continue;
            }
            let mut words = Vec::with_capacity(records.len());
            for (position, record) in records.iter().enumerate() {
                let word = text
                    .as_str()
                    .get(record.bytes.clone())
                    .ok_or(ai::Error::from(EmphasisRefusal::InvalidSegments))?;
                words.push(EmphasisWord {
                    position: u32::try_from(position)
                        .map_err(|_| ai::Error::from(EmphasisRefusal::Payload))?,
                    text: word.into(),
                });
            }
            segments.push(EmphasisSegment {
                id: format!("{}:{}", item.0, cue.id.as_str()).into(),
                text: text.clone(),
                words,
            });
        }
        let language = language.ok_or(ai::Error::from(EmphasisRefusal::EmptyTranscript))?;
        if !ai::supports_text_language(&language) {
            return Err(ai::Error::from(EmphasisRefusal::UnsupportedLanguage).into());
        }
        let input = EmphasisInput {
            segments,
            settings: EmphasisSettings {
                language,
                max_per_cue: captions::MAX_EMPHASIS_WORDS,
            },
        };
        input.validate().map_err(ai::Error::from)?;
        let source = serde_json::to_vec(&input).map_err(|_| {
            AnalysisError::Edit(crate::session::EditError::Invalid("emphasis input"))
        })?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Emphasis,
            source: *blake3::hash(&source).as_bytes(),
            provider: "text-access",
            model: "selected",
            version: RECIPE,
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self { item, input, key })
    }

    /// The exact validated cues the provider receives.
    pub fn input(&self) -> &EmphasisInput {
        &self.input
    }

    pub fn cache_key(&self) -> CacheKey {
        self.key
    }

    /// Obtain consent synchronously, then enqueue one network job. A refusal returns the
    /// typed reason and leaves the runner untouched.
    pub fn submit(&self, runner: &mut JobRunner, access: &TextAccess) -> Result<EmphasisJob> {
        let prepared = access.prepare_emphasis(&self.input)?;
        let input = self.input.clone();
        let item = self.item;
        let spec = JobSpec {
            kind: JobKind::SelectEmphasis,
            class: crate::jobs::ResourceClass::Network,
            input: self.key,
            bytes: JobBytes {
                result: MAX_RESULT_BYTES as u64,
                ..Default::default()
            },
        };
        let (id, cancel) = runner.submit(spec, move |context| {
            context.check_cancelled()?;
            context.progress(crate::jobs::JobStage::Process, 0.1)?;
            let selections = ai::text::emphasis::select_emphasis(
                prepared.selector.as_ref(),
                &input,
                &ai::Request {
                    scope: &prepared.scope,
                    consent: Some(&prepared.consent),
                },
                context,
            )?;
            context.progress(crate::jobs::JobStage::Process, 0.9)?;
            let suggestions = resolve(item, &selections);
            let bytes = serde_json::to_vec(&suggestions)
                .map_err(|_| AnalysisError::from(crate::session::wire::ControlError::Limit))?;
            if bytes.len() > MAX_RESULT_BYTES {
                return Err(AnalysisError::from(
                    crate::session::wire::ControlError::Limit,
                ));
            }
            Ok(bytes)
        })?;
        Ok(EmphasisJob { id, cancel })
    }
}

impl EmphasisJob {
    /// Validate and decode complete output; typed terminal failures and cancellation propagate.
    pub fn finish(&self, runner: &mut JobRunner) -> Result<Option<Vec<EmphasisSuggestion>>> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => Err(AnalysisError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(AnalysisError::Runtime(crate::RuntimeError::Failed(
                snapshot.error.ok_or(AnalysisError::from(
                    crate::session::SessionError::StaleResult,
                ))?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let suggestions = decode(bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
                Ok(Some(suggestions))
            }
            _ => Ok(None),
        }
    }
}

/// Turn resolved selections into suggestions; a cue the model skipped stays out.
fn resolve(item: ItemId, selections: &[EmphasisSelection]) -> Vec<EmphasisSuggestion> {
    let prefix = format!("{}:", item.0);
    selections
        .iter()
        .filter(|selection| !selection.positions.is_empty())
        .filter_map(|selection| {
            selection
                .segment
                .as_str()
                .strip_prefix(&prefix)
                .map(|cue| EmphasisSuggestion {
                    item,
                    cue: cue.into(),
                    word_ids: selection.positions.clone(),
                })
        })
        .collect()
}

/// Decode retained job bytes as the exact suggestion list.
pub fn decode(bytes: &[u8]) -> crate::session::Result<Vec<EmphasisSuggestion>> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(crate::session::EditError::from(
            crate::session::wire::ControlError::Limit,
        ));
    }
    let suggestions: Vec<EmphasisSuggestion> = serde_json::from_slice(bytes)
        .map_err(|_| crate::session::EditError::Invalid("emphasis result"))?;
    if suggestions.len() > crate::session::wire::MAX_COLLECTION {
        return Err(crate::session::EditError::from(
            crate::session::wire::ControlError::Limit,
        ));
    }
    for suggestion in &suggestions {
        if suggestion.cue.as_str().trim().is_empty()
            || suggestion.word_ids.len() > captions::MAX_EMPHASIS_WORDS
            || suggestion
                .word_ids
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a >= b))
        {
            return Err(crate::session::EditError::Invalid("emphasis result"));
        }
    }
    Ok(suggestions)
}
