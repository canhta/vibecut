//! Generates chapter, title and description candidates from the active scene's
//! caption cues through one lent provider.
//!
//! One job snapshots the project, maps the requested language's cues to output
//! time, validates the complete input, obtains the human consent token from the
//! lent [`crate::providers::TextAccess`] and runs one provider attempt. Cancellation discards the
//! result; a typed refusal happens before the job is enqueued.
use crate::{
    EditError,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState},
    providers::TextAccess,
};
use ai::chapters::{
    ChapterDocument, ChapterInput, ChapterRefusal, ChapterSegment, ChapterSettings, MAX_CHAPTERS,
};
use captions::LanguageTag;
use project::Project;
use std::sync::Arc;

const MAX_RESULT_BYTES: usize = 1 << 20;
const RECIPE: &str = "chapters-v1:cue-segments:timeline";

/// One immutable scene's chapter request. It is not cached; the key only
/// identifies the job snapshot.
#[derive(Clone)]
pub struct ChapterRequest {
    input: ChapterInput,
    key: CacheKey,
}
/// Retained runner job; `finish` decodes only a complete validated document.
pub struct ChapterJob {
    pub id: JobId,
    pub cancel: CancelToken,
}

impl ChapterRequest {
    /// Snapshot the requested language's cues in output time and validate them.
    /// An unsupported language, an empty transcript, hostile bounds or
    /// non-increasing/overlapping cues refuse before any provider work.
    pub fn new(
        project: &Arc<Project>,
        language: LanguageTag,
        max_chapters: usize,
    ) -> Result<Self, EditError> {
        if !ai::supports_text_language(&language) {
            return Err(ai::Error::from(ChapterRefusal::UnsupportedLanguage).into());
        }
        if !(1..=MAX_CHAPTERS).contains(&max_chapters) {
            return Err(ai::Error::from(ChapterRefusal::InvalidSettings).into());
        }
        let view = crate::session::transcript::view(project.data())?;
        let mut segments = Vec::new();
        for cue in &view.cues {
            let Some(text) = cue.texts.get(&language) else {
                continue;
            };
            let id = captions::NfcText::from(format!("{}:{}", cue.item.0, cue.id.as_str()));
            if id.as_str().len() > 256 {
                return Err(ai::Error::from(ChapterRefusal::InvalidSegments).into());
            }
            segments.push(ChapterSegment {
                id,
                text: text.text.clone(),
                range: cue.output,
            });
        }
        if segments.is_empty() {
            return Err(ai::Error::from(ChapterRefusal::EmptyTranscript).into());
        }
        let input = ChapterInput {
            segments,
            settings: ChapterSettings {
                language,
                max_chapters,
                duration: project.duration(),
            },
        };
        input.validate().map_err(ai::Error::from)?;
        let source = serde_json::to_vec(&input).map_err(|_| EditError::Invalid("chapter input"))?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Chapters,
            source: *blake3::hash(&source).as_bytes(),
            provider: "chapter-access",
            model: "selected",
            version: RECIPE,
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self { input, key })
    }

    /// The exact validated input the provider receives.
    pub fn input(&self) -> &ChapterInput {
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
    ) -> Result<ChapterJob, EditError> {
        let prepared = access.prepare_chapter(&self.input)?;
        let input = self.input.clone();
        let spec = JobSpec {
            kind: JobKind::GenerateChapters,
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
            let document = ai::chapters::generate_chapters(
                prepared.generator.as_ref(),
                &input,
                &ai::Request {
                    scope: &prepared.scope,
                    consent: Some(&prepared.consent),
                },
                context,
            )?;
            context.progress(crate::jobs::JobStage::Process, 0.9)?;
            let bytes = serde_json::to_vec(&document).map_err(|_| EditError::Limit)?;
            if bytes.len() > MAX_RESULT_BYTES {
                return Err(EditError::Limit);
            }
            Ok(bytes)
        })?;
        Ok(ChapterJob { id, cancel })
    }
}

impl ChapterJob {
    /// Validate and decode complete output; typed terminal failures and
    /// cancellation propagate.
    pub fn finish(&self, runner: &mut JobRunner) -> Result<Option<ChapterDocument>, EditError> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => Err(EditError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(EditError::Runtime(crate::RuntimeError::Failed(
                snapshot.error.ok_or(EditError::StaleResult)?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let document = decode(bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                Ok(Some(document))
            }
            _ => Ok(None),
        }
    }
}

/// Decode retained job bytes through the versioned document validator.
pub fn decode(bytes: &[u8]) -> Result<ChapterDocument, EditError> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(EditError::Limit);
    }
    ChapterDocument::from_json(bytes).map_err(EditError::Ai)
}
