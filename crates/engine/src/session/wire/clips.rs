//! Defines the clip suggestion output and the export acceptance a control response carries.
use super::{CostView, JobId};
use captions::{Cue, LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::{SourceRange, Tick};

/// One snapped edge of a piece: where the suggestion put it, where it moved and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipEdge {
    pub original: Tick,
    pub adjusted: Tick,
    pub delta: Tick,
    /// `sentence_silence`, `sentence`, `silence`, `word_or_segment` or `word_fade`.
    pub kind: String,
}

/// Both edges of one piece.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipAdjustment {
    pub start: ClipEdge,
    pub end: ClipEdge,
}

/// One snapped source piece in play order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipPieceRange {
    pub range: SourceRange,
    /// `hook`, `context` or `payoff`.
    pub role: String,
    /// Zero-based rank of the piece in source order.
    pub source_order: usize,
}

/// The transcript segment span the suggestion named for one piece, before snapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipPieceSpan {
    pub start_segment_id: NfcText,
    pub end_segment_id: NfcText,
    pub role: String,
}

/// One ranked short after independent per-edge snapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipCandidateView {
    pub title: NfcText,
    pub hook_text: NfcText,
    pub reason: NfcText,
    pub score: f64,
    /// `llm` or `heuristic`.
    pub source: String,
    /// The first rule the short matches.
    pub label: ai::text::clips::ClipLabel,
    /// Sum of the piece durations, excluding gaps.
    pub duration: Tick,
    pub segments: Vec<ClipPieceRange>,
    pub adjustments: Vec<ClipAdjustment>,
    pub pieces: Vec<ClipPieceSpan>,
}

/// One measured speech-energy window, serialized as `[range, rms]`: mono RMS amplitude ratio
/// over at most 20 ms. A hint for trim snapping, never recognition.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClipEnergy(pub SourceRange, pub f32);

/// A suggestion that was refused, with its typed reason name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipRefusal {
    pub title: NfcText,
    pub reason: String,
}

/// The finished suggestion of `clips.suggest`: ranked candidates plus the source-timed cues
/// the caller may caption and translate before `clips.export { job }`.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipsResult {
    pub asset: media::AssetId,
    pub source_language: LanguageTag,
    pub transcription_provider: String,
    pub transcription_model: String,
    pub provider: String,
    pub model: String,
    pub clips: Vec<ClipCandidateView>,
    /// Snap refusals: the suggestion had no safe frame, duration or overlap.
    pub rejected: Vec<ClipRefusal>,
    /// Suggestions the provider contract dropped.
    pub dropped: Vec<ClipRefusal>,
    pub cues: Vec<Cue>,
    /// Speech-energy windows over the whole source, in source order. Merged into coarser
    /// peak-RMS windows when the source is too long for them to fit the result limit.
    pub energy: Vec<ClipEnergy>,
    pub transcript_segments: usize,
    /// ASR windows served from the checkpoint cache.
    pub checkpoints: usize,
    pub transcription_cost: CostView,
    pub suggestion_cost: CostView,
    pub elapsed_seconds: f64,
}
impl std::fmt::Debug for ClipsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipsResult")
            .field("asset", &self.asset)
            .field("clips", &self.clips.len())
            .field("cues", &self.cues.len())
            .finish_non_exhaustive()
    }
}

/// One sidecar file an exported short writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSubtitleFile {
    pub format: captions::Format,
    pub path: PathBuf,
}

/// One accepted short export: its candidate index, its own job and the files it publishes.
/// Cancelling the job stops only this short; a finished short's files are complete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipExportShort {
    /// Zero-based candidate index in `clips.suggest` output order.
    pub candidate: usize,
    pub job: JobId,
    pub video: PathBuf,
    pub subtitles: Vec<ClipSubtitleFile>,
}

/// Why a clip export carries no per-shot analysis: every short then keeps the centred Fill
/// layout. The export still runs; this names what to do to get layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipShotsDiagnostic {
    /// The process lends no vision detector or media cache.
    DetectorUnavailable,
    /// The detector's model weights are not installed, or their download was declined.
    ModelsMissing,
    /// The model cache is in use (a download or check runs): the weights may be installed.
    /// Nothing is lost; ask again after the transfer finishes.
    ModelsBusy,
    /// The analysis runs as this job; an export after it completes reuses its cached result.
    Pending { job: JobId },
    /// The analysis could not be requested or published.
    Failed,
}
