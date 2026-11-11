//! Turns speech-activity spans into reviewed output-time cut candidates.
//!
//! One job decodes each contiguous audible source interval of the active scene with
//! [`SpeechActivityRequest`]'s detector, pads every pause, maps it to output time
//! through the item's retime owner, merges overlaps and removes every range covered
//! by a native caption word span. The job holds no artifact cache: the Session owns
//! none, so a repeated query decodes again.
pub use crate::session::wire::{OutputSpan, PauseCandidates};
use crate::{
    EditError,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobState,
        ResourceClass,
    },
    session::wire::{MAX_PAUSE_MS, MAX_PAUSE_SPANS},
    speech_activity::{Activity, SpeechActivityRequest, SpeechActivitySettings},
};
use media::{SourceAsset, StreamKind};
use project::{Item, ItemParams, Project, ProjectData};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use time::{Rounding, SourceRange, Tick};

const MAX_BYTES: usize = 1 << 20;
const RECIPE: &str = "pause-candidates-v1:speech-activity-v2:pad:word-guard:merge";

/// Candidate settings in milliseconds. Defaults and their sources live in the
/// analysis README and the milestone map, never in this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseSettings {
    pub min_pause_ms: i64,
    pub pad_ms: i64,
    pub crossfade_ms: i64,
}
impl Default for PauseSettings {
    fn default() -> Self {
        Self {
            min_pause_ms: 500,
            pad_ms: 20,
            crossfade_ms: 20,
        }
    }
}
impl PauseSettings {
    /// `pad` keeps a detection frame of silence on each side, so a fade of at most
    /// the pad never attenuates speech.
    pub fn validate(self) -> Result<(), EditError> {
        if !(0..=MAX_PAUSE_MS).contains(&self.min_pause_ms)
            || !(0..=MAX_PAUSE_MS).contains(&self.pad_ms)
            || !(0..=self.pad_ms).contains(&self.crossfade_ms)
        {
            return Err(EditError::Invalid("pause settings"));
        }
        Ok(())
    }
    pub fn min_pause(self) -> Result<Tick, EditError> {
        Ok(Tick::from_units(
            self.min_pause_ms,
            1000,
            Rounding::Nearest,
        )?)
    }
    pub fn pad(self) -> Result<Tick, EditError> {
        Ok(Tick::from_units(self.pad_ms, 1000, Rounding::Nearest)?)
    }
}

/// One contiguous audible source interval that the job decodes.
struct Ask {
    asset: SourceAsset,
    hash: [u8; 32],
    range: SourceRange,
}
/// The pauses of one decoded source window in source time.
struct Analysis {
    hash: [u8; 32],
    pauses: Vec<SourceRange>,
}

/// One immutable scene's candidate request. It is not cached, so the key only
/// identifies the job snapshot.
#[derive(Clone)]
pub struct PauseCandidatesRequest {
    project: Arc<Project>,
    settings: PauseSettings,
    key: CacheKey,
}
/// Retained runner job; `finish` decodes only complete validated output.
pub struct PauseCandidatesJob {
    pub id: JobId,
    pub cancel: CancelToken,
}
impl PauseCandidatesRequest {
    pub fn new(project: Arc<Project>, settings: PauseSettings) -> Result<Self, EditError> {
        settings.validate()?;
        let canonical =
            serde_json::to_vec(&settings).map_err(|_| EditError::Invalid("pause settings"))?;
        let source = scene_digest(project.data())?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::PauseCandidates,
            source,
            provider: "local",
            model: "energy",
            version: RECIPE,
            settings: &canonical,
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self {
            project,
            settings,
            key,
        })
    }

    pub fn cache_key(&self) -> CacheKey {
        self.key
    }

    /// Enqueue one CPU job. An inaudible timeline still completes with zero spans,
    /// so callers never special-case an empty project.
    pub fn submit(&self, runner: &mut JobRunner) -> Result<PauseCandidatesJob, EditError> {
        let request = self.clone();
        // One retained 10-minute PCM window (38.4 MB) plus the detector history
        // (96 MiB) and at most 1 MiB of serialized spans.
        let spec = JobSpec {
            kind: JobKind::AnalyzePauses,
            class: ResourceClass::Cpu,
            input: self.key,
            bytes: JobBytes {
                decoded_frames: 40 << 20,
                compositor: 96 << 20,
                result: MAX_BYTES as u64,
                ..Default::default()
            },
        };
        let (id, cancel) = runner.submit(spec, move |context| {
            context.check_cancelled()?;
            let result = request.analyze(context)?;
            context.check_cancelled()?;
            let bytes = serde_json::to_vec(&result).map_err(|_| EditError::Limit)?;
            if bytes.len() > MAX_BYTES {
                return Err(EditError::Limit);
            }
            Ok(bytes)
        })?;
        Ok(PauseCandidatesJob { id, cancel })
    }

    fn analyze(&self, context: &JobContext) -> Result<PauseCandidates, EditError> {
        let asks = asks(self.project.data())?;
        let total = asks.len().max(1);
        let min_pause = self.settings.min_pause()?;
        let settings = SpeechActivitySettings {
            min_pause,
            ..SpeechActivitySettings::default()
        };
        let mut analyses = Vec::with_capacity(asks.len());
        for (index, ask) in asks.iter().enumerate() {
            context.check_cancelled()?;
            let request = SpeechActivityRequest::new(ask.asset.clone(), ask.range, settings)?;
            let base = index as f32 / total as f32;
            let (spans, _frames) = request.detect(context, base, 1.0 / total as f32)?;
            analyses.push(Analysis {
                hash: ask.hash,
                pauses: spans
                    .into_iter()
                    .filter(|span| span.activity == Activity::Pause)
                    .map(|span| span.range)
                    .collect(),
            });
        }
        assemble(self.project.data(), self.settings, &analyses)
    }
}

impl PauseCandidatesJob {
    /// Validate and decode complete output; typed terminal failures and
    /// cancellation propagate.
    pub fn finish(&self, runner: &mut JobRunner) -> Result<Option<PauseCandidates>, EditError> {
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
                let result = decode(bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
}

/// Decode and validate retained job bytes. A result must be a sorted, bounded,
/// non-overlapping span list.
pub fn decode(bytes: &[u8]) -> Result<PauseCandidates, EditError> {
    if bytes.len() > MAX_BYTES {
        return Err(EditError::Invalid("pause candidates result"));
    }
    let result: PauseCandidates =
        serde_json::from_slice(bytes).map_err(|_| EditError::Invalid("pause candidates result"))?;
    if result.spans.len() > MAX_PAUSE_SPANS {
        return Err(EditError::Limit);
    }
    let mut previous = Tick::ZERO;
    for span in &result.spans {
        if span.start < previous || span.end <= span.start {
            return Err(EditError::Invalid("pause candidates result"));
        }
        previous = span.end;
    }
    Ok(result)
}

/// Output-time word ranges of every caption item in the active scene.
fn word_spans(data: &ProjectData) -> Result<Vec<SourceRange>, EditError> {
    Ok(crate::session::transcript::scene_words(data.scene())?
        .into_iter()
        .map(|word| word.output)
        .collect())
}

fn audible(item: &Item) -> bool {
    match &item.content {
        ItemParams::Audio(params) => !params.muted,
        ItemParams::Video(params) => params.source_audio_enabled && !params.muted,
        _ => false,
    }
}

fn source_asset<'a>(data: &'a ProjectData, item: &Item) -> Result<&'a SourceAsset, EditError> {
    item.source_asset
        .and_then(|id| data.assets.iter().find(|asset| asset.id() == id))
        .ok_or(EditError::Invalid("pause candidate item has no asset"))
}

fn asks(data: &ProjectData) -> Result<Vec<Ask>, EditError> {
    let mut sources: BTreeMap<[u8; 32], (SourceAsset, Vec<SourceRange>)> = BTreeMap::new();
    for track in data.scene().timeline.iter() {
        if track.muted {
            continue;
        }
        for item in &track.items {
            if !audible(item) {
                continue;
            }
            let asset = source_asset(data, item)?;
            if !asset
                .probe()
                .streams
                .iter()
                .any(|stream| stream.kind == StreamKind::Audio)
            {
                continue;
            }
            sources
                .entry(asset.content_hash().0)
                .or_insert_with(|| (asset.clone(), Vec::new()))
                .1
                .push(item.source_range);
        }
    }
    let mut asks = Vec::new();
    for (hash, (asset, mut ranges)) in sources {
        ranges.sort_unstable_by_key(|range| (range.start(), range.end()));
        let mut merged: Vec<SourceRange> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && range.start() <= last.end()
            {
                *last = SourceRange::new(last.start(), last.end().max(range.end()))?;
            } else {
                merged.push(range);
            }
        }
        for range in merged {
            asks.push(Ask {
                asset: asset.clone(),
                hash,
                range,
            });
        }
    }
    Ok(asks)
}

fn scene_digest(data: &ProjectData) -> Result<[u8; 32], EditError> {
    let mut hash = blake3::Hasher::new();
    hash.update(RECIPE.as_bytes());
    for track in data.scene().timeline.iter() {
        hash.update(&[u8::from(track.muted)]);
        for item in &track.items {
            let asset = item.source_asset.map_or([0; 32], |id| {
                data.assets
                    .iter()
                    .find(|asset| asset.id() == id)
                    .map_or([0; 32], |asset| asset.content_hash().0)
            });
            hash.update(&asset);
            hash.update(&item.source_range.start().0.to_le_bytes());
            hash.update(&item.source_range.end().0.to_le_bytes());
            hash.update(&item.start.0.to_le_bytes());
            hash.update(&[u8::from(audible(item))]);
        }
    }
    for word in word_spans(data)? {
        hash.update(&word.start().0.to_le_bytes());
        hash.update(&word.end().0.to_le_bytes());
    }
    Ok(*hash.finalize().as_bytes())
}

fn assemble(
    data: &ProjectData,
    settings: PauseSettings,
    analyses: &[Analysis],
) -> Result<PauseCandidates, EditError> {
    let pad = settings.pad()?;
    let timeline = &data.scene().timeline;
    let mut spans = Vec::new();
    for analysis in analyses {
        for pause in &analysis.pauses {
            for track in timeline.iter() {
                if track.muted {
                    continue;
                }
                for item in &track.items {
                    if !audible(item) || source_asset(data, item)?.content_hash().0 != analysis.hash
                    {
                        continue;
                    }
                    // A pause clipped by the item edge keeps no pad on that side: the
                    // pad only separates a cut from speech, and there is no speech there.
                    let start = pause.start().max(item.source_range.start());
                    let end = pause.end().min(item.source_range.end());
                    if end <= start {
                        continue;
                    }
                    let start = if start == pause.start() {
                        match start.checked_add(pad) {
                            Some(start) => start,
                            None => continue,
                        }
                    } else {
                        start
                    };
                    let end = if end == pause.end() {
                        match end.checked_sub(pad) {
                            Some(end) => end,
                            None => continue,
                        }
                    } else {
                        end
                    };
                    if end <= start {
                        continue;
                    }
                    let start = item.output_time_of(start)?;
                    let end = item.output_time_of(end)?;
                    if start < end {
                        spans.push(OutputSpan { start, end });
                    }
                }
            }
        }
    }
    let spans = subtract(merge(spans), &word_spans(data)?);
    let spans = merge(spans);
    if spans.len() > MAX_PAUSE_SPANS {
        return Err(EditError::Limit);
    }
    Ok(PauseCandidates {
        spans,
        analyzed_sources: analyses.len() as u64,
    })
}

fn merge(spans: Vec<OutputSpan>) -> Vec<OutputSpan> {
    let mut spans = spans;
    spans.sort_by_key(|span| (span.start, span.end));
    let mut merged: Vec<OutputSpan> = Vec::with_capacity(spans.len().min(MAX_PAUSE_SPANS));
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.start <= last.end
        {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    merged
}

/// Subtract every word range from the sorted candidates in one sweep.
fn subtract(spans: Vec<OutputSpan>, words: &[SourceRange]) -> Vec<OutputSpan> {
    let mut out = Vec::with_capacity(spans.len());
    let mut first = 0usize;
    for span in spans {
        let mut cursor = span.start;
        while words.get(first).is_some_and(|word| word.end() <= cursor) {
            first += 1;
        }
        let mut index = first;
        while let Some(word) = words.get(index) {
            if word.start() >= span.end {
                break;
            }
            if word.end() <= cursor {
                index += 1;
                continue;
            }
            if word.start() > cursor {
                out.push(OutputSpan {
                    start: cursor,
                    end: word.start().min(span.end),
                });
            }
            cursor = cursor.max(word.end());
            if cursor >= span.end {
                break;
            }
            index += 1;
        }
        if cursor < span.end {
            out.push(OutputSpan {
                start: cursor,
                end: span.end,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtraction_splits_around_words_and_merge_unions_overlaps() {
        let spans = merge(vec![
            OutputSpan {
                start: Tick(10),
                end: Tick(40),
            },
            OutputSpan {
                start: Tick(30),
                end: Tick(60),
            },
        ]);
        assert_eq!(
            spans,
            vec![OutputSpan {
                start: Tick(10),
                end: Tick(60)
            }]
        );
        let words = [
            SourceRange::new(Tick(20), Tick(25)).unwrap(),
            SourceRange::new(Tick(50), Tick(55)).unwrap(),
        ];
        assert_eq!(
            subtract(spans, &words),
            vec![
                OutputSpan {
                    start: Tick(10),
                    end: Tick(20)
                },
                OutputSpan {
                    start: Tick(25),
                    end: Tick(50)
                },
                OutputSpan {
                    start: Tick(55),
                    end: Tick(60)
                },
            ]
        );
    }

    #[test]
    fn settings_refuse_a_fade_wider_than_the_kept_pad() {
        assert!(
            PauseSettings {
                crossfade_ms: 21,
                ..PauseSettings::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PauseSettings {
                min_pause_ms: -1,
                ..PauseSettings::default()
            }
            .validate()
            .is_err()
        );
        assert!(PauseSettings::default().validate().is_ok());
    }
}
