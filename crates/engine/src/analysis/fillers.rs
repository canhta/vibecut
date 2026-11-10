//! Classifies filler words in the active scene's timed transcript through one
//! lent text provider.
//!
//! One job snapshots the scene's visible measured words, validates the complete
//! bounded input, obtains the human consent token from the lent [`TextAccess`] and
//! runs one provider attempt. Returned IDs must exist in that exact transcript;
//! unknown or duplicate IDs fail the job with the typed `InvalidSegments` reason.
//! Cancellation discards the result and no project mutation ever happens.
use crate::{
    EditError,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState},
    providers::TextAccess,
    session::wire::{FillerCandidate, OutputSpan},
};
use ai::fillers::{
    FillerFinding, FillerInput, FillerRefusal, FillerSegment, FillerSettings, FillerWord,
};
use captions::{LanguageTag, NfcText};
use project::{ItemId, ItemParams, Project};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::{SourceRange, Tick};

const MAX_RESULT_BYTES: usize = 1 << 20;
const RECIPE: &str = "fillers-v1:timed-words:timeline";

/// One visible measured word the model may select, in output time.
#[derive(Clone)]
struct Word {
    item: ItemId,
    cue: NfcText,
    output: SourceRange,
    edited: bool,
}

/// One immutable scene's filler request. It is not cached; the key only
/// identifies the job snapshot.
#[derive(Clone)]
pub struct FillerRequest {
    input: FillerInput,
    words: BTreeMap<NfcText, Word>,
    key: CacheKey,
}
/// Retained runner job; `finish` decodes only a complete validated candidate list.
pub struct FillerJob {
    pub id: JobId,
    pub cancel: CancelToken,
}

impl FillerRequest {
    /// Snapshot the requested language's visible measured words and validate them.
    /// An unsupported language or a transcript without timed words refuses before
    /// any provider work.
    pub fn new(
        project: &Arc<Project>,
        language: LanguageTag,
        include_markers: bool,
    ) -> Result<Self, EditError> {
        if !ai::supports_text_language(&language) {
            return Err(ai::Error::from(FillerRefusal::UnsupportedLanguage).into());
        }
        let mut segments = Vec::new();
        let mut words = BTreeMap::new();
        for item in project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
        {
            let ItemParams::Caption(params) = &item.content else {
                continue;
            };
            let spans = item.word_spans()?;
            for cue in &params.cues.cues {
                if primary(cue.source_lang.as_str()) != primary(language.as_str()) {
                    continue;
                }
                let Some(text) = cue.source_text() else {
                    continue;
                };
                if text.text.as_str().trim().is_empty() {
                    continue;
                }
                let cue_words = cue.words();
                if cue_words.len() != cue.word_ranges.len() {
                    continue;
                }
                let mut input_words = Vec::new();
                for (index, cue_word) in cue_words.iter().enumerate() {
                    let Some(span) = spans
                        .iter()
                        .find(|span| span.cue == cue.id && span.index == index)
                    else {
                        continue;
                    };
                    let id: NfcText = format!(
                        "filler:{}:{}:{}",
                        item.id.0,
                        cue.id.as_str(),
                        cue_word.range.start().0
                    )
                    .into();
                    words.insert(
                        id.clone(),
                        Word {
                            item: item.id,
                            cue: cue.id.clone(),
                            output: span.output,
                            edited: text.edited,
                        },
                    );
                    input_words.push(FillerWord {
                        id,
                        text: cue_word.text.clone(),
                    });
                }
                if input_words.is_empty() {
                    continue;
                }
                segments.push(FillerSegment {
                    id: format!("{}:{}", item.id.0, cue.id.as_str()).into(),
                    text: text.text.clone(),
                    words: input_words,
                });
            }
        }
        if segments.is_empty() {
            return Err(ai::Error::from(FillerRefusal::EmptyTranscript).into());
        }
        let input = FillerInput {
            segments,
            settings: FillerSettings {
                language,
                include_markers,
            },
        };
        input.validate().map_err(ai::Error::from)?;
        let source = serde_json::to_vec(&input).map_err(|_| EditError::Invalid("filler input"))?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Fillers,
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
        Ok(Self { input, words, key })
    }

    /// The exact validated transcript the provider receives.
    pub fn input(&self) -> &FillerInput {
        &self.input
    }

    /// Bind the job key to the lent provider so job snapshots stay distinct per
    /// configuration.
    pub fn cache_key(&self) -> CacheKey {
        self.key
    }

    /// Obtain consent synchronously, then enqueue one network job. A refusal
    /// returns the typed reason and leaves the runner untouched.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        access: &TextAccess,
    ) -> Result<FillerJob, EditError> {
        let prepared = access.prepare_fillers(&self.input)?;
        let input = self.input.clone();
        let words = self.words.clone();
        let spec = JobSpec {
            kind: JobKind::ClassifyFillers,
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
            let findings = ai::fillers::classify_fillers(
                prepared.classifier.as_ref(),
                &input,
                &ai::Request {
                    scope: &prepared.scope,
                    consent: Some(&prepared.consent),
                },
                context,
            )?;
            context.progress(crate::jobs::JobStage::Process, 0.9)?;
            let candidates = resolve(&findings, &words)?;
            let bytes = serde_json::to_vec(&candidates).map_err(|_| EditError::Limit)?;
            if bytes.len() > MAX_RESULT_BYTES {
                return Err(EditError::Limit);
            }
            Ok(bytes)
        })?;
        Ok(FillerJob { id, cancel })
    }
}

impl FillerJob {
    /// Validate and decode complete output; typed terminal failures and
    /// cancellation propagate.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
    ) -> Result<Option<Vec<FillerCandidate>>, EditError> {
        let snapshot = runner.snapshot(self.id).ok_or(EditError::JobNotRetained)?;
        match snapshot.state {
            JobState::Cancelled => Err(EditError::Cancelled),
            JobState::Failed => Err(EditError::JobFailure(
                snapshot.error.ok_or(EditError::StaleResult)?,
            )),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                let candidates = decode(bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                Ok(Some(candidates))
            }
            _ => Ok(None),
        }
    }
}

/// Map model IDs back to the exact visible words and output spans this job sent.
fn resolve(
    findings: &[FillerFinding],
    words: &BTreeMap<NfcText, Word>,
) -> Result<Vec<FillerCandidate>, EditError> {
    let mut candidates = Vec::with_capacity(findings.len());
    for finding in findings {
        let word = words
            .get(&finding.id)
            .ok_or(ai::Error::from(FillerRefusal::InvalidSegments))?;
        candidates.push(FillerCandidate {
            id: finding.id.clone(),
            item: word.item,
            cue: word.cue.clone(),
            range: OutputSpan {
                start: word.output.start(),
                end: word.output.end(),
            },
            kind: finding.kind,
            edited_cue: word.edited,
        });
    }
    candidates.sort_by_key(|candidate| (candidate.range.start, candidate.item));
    Ok(candidates)
}

/// Decode retained job bytes as the exact candidate list.
pub fn decode(bytes: &[u8]) -> Result<Vec<FillerCandidate>, EditError> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(EditError::Limit);
    }
    let candidates: Vec<FillerCandidate> =
        serde_json::from_slice(bytes).map_err(|_| EditError::Invalid("filler result"))?;
    if candidates.len() > crate::session::wire::MAX_COLLECTION {
        return Err(EditError::Limit);
    }
    for candidate in &candidates {
        if candidate.id.as_str().trim().is_empty()
            || candidate.range.start < Tick::ZERO
            || candidate.range.end <= candidate.range.start
        {
            return Err(EditError::Invalid("filler result"));
        }
    }
    Ok(candidates)
}

fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or_default()
}
