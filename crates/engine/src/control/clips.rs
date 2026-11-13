//! The clip workflow a composition root lends to `clips.suggest` and `clips.export`.
use super::{Error as EngineError, Result};
use crate::cache::CacheKey;
use crate::jobs::{JobId, JobRunner};
use crate::session::wire::{ClipEnergy, ClipExportShort, ClipsResult, bounded_json};
use crate::subtitles::CaptionAccess;
use crate::{EditError, Session};
use captions::{CueTrack, Format, LanguageTag, NfcText};
use std::path::PathBuf;
use std::sync::Arc;
use time::{SourceRange, Tick};

/// Largest retained output of one suggestion job; also its result reservation.
pub const MAX_CLIPS_RESULT_BYTES: usize = 16 << 20;

/// What `clips.suggest` asks for. Every `None` takes the workflow's documented default;
/// the request carries no credential and no approval, which the lent access owns.
#[derive(Clone, Debug)]
pub struct ClipsSuggestRequest {
    /// The project asset to transcribe and suggest from, imported and probed by the caller.
    pub asset: media::SourceAsset,
    /// `None` asks the selected ASR provider to detect the language locally.
    pub source_language: Option<LanguageTag>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub count: Option<usize>,
    pub min_duration: Option<Tick>,
    pub max_duration: Option<Tick>,
    pub max_segments: Option<usize>,
    pub min_segment_duration: Option<Tick>,
    pub overlap_tolerance: Option<f64>,
    pub keywords: Vec<NfcText>,
}

/// What `clips.export` asks for, with the completed suggestion already decoded.
#[derive(Clone, Debug)]
pub struct ClipsExportRequest {
    pub result: ClipsResult,
    /// Source-timed cues, with any translations, that captions are burned from.
    pub cues: CueTrack,
    /// Zero-based candidate indices; 1..=1000 distinct values.
    pub select: Vec<usize>,
    pub languages: Vec<LanguageTag>,
    pub formats: Vec<Format>,
    /// Existing output directory.
    pub directory: PathBuf,
    pub overwrite: bool,
    pub allow_degraded: bool,
}

/// The two clip workflows, implemented above the engine and lent through
/// [`CaptionAccess::with_clips`]. Admission, grants, receipts and revision checks run before
/// either is invoked; the engine decodes the retained suggestion and builds the response.
pub trait ClipWorkflow: Send + Sync {
    /// Submit one suggestion job onto `runner` and return the accepted job. Provider choice
    /// and credential presence are checked here, before any ASR or upload work starts.
    /// Providers, consent and the checkpoint cache come from `access`.
    fn suggest(
        &self,
        runner: &mut JobRunner,
        access: &CaptionAccess,
        session: &Session,
        request: ClipsSuggestRequest,
        input: CacheKey,
    ) -> Result<JobId>;

    /// Prepare every selected short against `project` and submit one export job per short.
    /// A failed admission cancels the shorts already admitted and returns its typed error.
    fn export(
        &self,
        runner: &mut JobRunner,
        project: &project::Project,
        request: ClipsExportRequest,
    ) -> Result<Vec<ClipExportShort>>;
}

impl CaptionAccess {
    /// Lend the clip workflow implemented above the engine. Clip jobs share these providers,
    /// this consent and this checkpoint cache, so no clip command carries its own.
    pub fn with_clips(mut self, clips: Arc<dyn ClipWorkflow>) -> Self {
        self.clips = Some(Arc::new(clips));
        self
    }
    /// The lent clip workflow, absent unless the composition root supplied one.
    pub fn clips(&self) -> Option<&Arc<dyn ClipWorkflow>> {
        self.clips
            .as_ref()
            .and_then(|lent| lent.downcast_ref::<Arc<dyn ClipWorkflow>>())
    }
}

/// Decode the retained bytes of a finished `clips.suggest` job.
pub fn decode_clips(bytes: &[u8]) -> Result<ClipsResult> {
    if bytes.len() > MAX_CLIPS_RESULT_BYTES {
        return Err(EngineError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    serde_json::from_slice(bytes)
        .map_err(|_| EngineError::Edit(EditError::Invalid("clip suggestion output is malformed")))
}

/// Encode a finished suggestion within [`MAX_CLIPS_RESULT_BYTES`]. When the speech-energy
/// windows of an hours-long source overflow it, adjacent windows are merged in groups of
/// 2, 4, 8, ... until the result fits: a merged window spans its group's ranges and keeps the
/// group's peak RMS, so speech stays visible to trim snapping at a coarser resolution and the
/// lane still covers the whole source. Every other field stays exact.
pub fn encode_clips(mut result: ClipsResult) -> Result<Vec<u8>> {
    let mut group = 1usize;
    let full = std::mem::take(&mut result.energy);
    loop {
        result.energy = merge_energy(&full, group);
        match bounded_json(&result, MAX_CLIPS_RESULT_BYTES) {
            Ok(bytes) => return Ok(bytes),
            Err(_) if group < full.len() => group = group.saturating_mul(2),
            Err(error) => return Err(EngineError::Control(error)),
        }
    }
}

fn merge_energy(windows: &[ClipEnergy], group: usize) -> Vec<ClipEnergy> {
    if group <= 1 {
        return windows.to_vec();
    }
    windows
        .chunks(group)
        .filter_map(|chunk| {
            let (first, last) = (chunk.first()?, chunk.last()?);
            let range = SourceRange::new(first.0.start(), last.0.end()).ok()?;
            let peak = chunk.iter().map(|window| window.1).fold(0.0f32, f32::max);
            Some(ClipEnergy(range, peak))
        })
        .collect()
}
