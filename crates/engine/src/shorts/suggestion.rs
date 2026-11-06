//! Produces source-snapped suggestions for accepted shorts.
use crate::EditError;
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::clip_boundaries::{
    BoundaryFailure, ClipBoundarySettings, SegmentAdjustment, snap_clip_boundaries,
};
use crate::jobs::{JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass};
use crate::providers as engine_providers;
use crate::providers::{
    CatalogOperation, ConsentPrompt, Cost, Credentials, Providers, UploadRequest,
};
use crate::shorts::{ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, MAX_BATCH_CLIPS};
use crate::transcript::{
    Cancellation, Progress, Stage, Transcription, build_session, hex, transcribe, translate_track,
    wait_for_job,
};
use ai::clips::{
    ANTHROPIC_SETTINGS, AnthropicClipSuggester, ClipDisclosure, ClipInput, ClipProposal,
    ClipSegment, ClipSettings, ClipSource, ClipSourceSegment, ClipSuggester, ClipSuggestion,
    DroppedClip, HeuristicClipSuggester, MAX_WIRE, MODE, OPENAI_LUNA_MODEL, OPENAI_SETTINGS,
    OpenAiClipSuggester,
};
use ai::{Consent, CostEstimate, Location, Money, Operation, ProviderConfig, SampleRange, Scope};
use captions::{Cue, CueTrack, Format, LanguageTag, NfcText};
use media::SourceAsset;
use project::Project;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use time::{FrameRate, SourceRange, Tick};

/// One user-facing source of clip proposals. No implicit fallback exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuggestionProvider {
    Heuristic,
    OpenAi { model: String },
    Anthropic { model: String },
}

/// Offline picker data; constructing it never loads an adapter or calls a provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestionOption {
    pub id: &'static str,
    pub label: &'static str,
    pub local: bool,
    pub models: Vec<&'static str>,
    pub policy_url: Option<&'static str>,
}

/// Operation-specific provider data link shown before remote clip consent.
pub const OPENAI_POLICY_URL: &str = "https://developers.openai.com/api/docs/guides/your-data";

/// Frame snapping inputs. `frame_rate` defaults to the source video rate.
#[derive(Clone, Copy, Debug)]
pub struct SnapSettings {
    pub frame_rate: Option<FrameRate>,
    pub tolerance: Tick,
    pub silence_rms: f64,
}

#[derive(Clone, Debug)]
pub struct ClipRequest {
    pub input: PathBuf,
    /// `None` asks the selected ASR provider for local language detection.
    pub source_language: Option<LanguageTag>,
    pub settings: ClipSettings,
    pub snap: SnapSettings,
    pub provider: SuggestionProvider,
}

/// One ranked short after independent per-edge snapping.
#[derive(Clone, Debug)]
pub struct ClipCandidate {
    pub proposal: ClipProposal,
    pub segments: Vec<ClipSourceSegment>,
    pub adjustments: Vec<SegmentAdjustment>,
    pub duration: Tick,
    pub source: ClipSource,
}

/// A suggestion that could not be snapped safely and was dropped from the result.
#[derive(Clone, Debug)]
pub struct RejectedClip {
    pub title: NfcText,
    pub reason: BoundaryFailure,
}

pub struct ClipSuggestionOutcome {
    pub input: PathBuf,
    pub source_language: LanguageTag,
    pub transcription_provider: String,
    pub transcription_model: String,
    pub provider: String,
    pub model: String,
    pub clips: Vec<ClipCandidate>,
    pub dropped: Vec<DroppedClip>,
    pub rejected: Vec<RejectedClip>,
    pub transcript_segments: usize,
    pub cues: usize,
    pub checkpoints: usize,
    pub transcription_cost: Cost,
    pub suggestion_cost: Cost,
    pub elapsed: Duration,
    /// Retained source context for desktop review; no ASR or energy re-decoding.
    pub asset: SourceAsset,
    duration: Tick,
    pub source_cues: Vec<Cue>,
    pub source_project: Project,
    pub transcript: captions::Transcript,
    pub energy: Vec<media::SpeechEnergyWindow>,
    pub boundaries: ClipBoundarySettings,
}

enum BuiltSuggester {
    Heuristic(HeuristicClipSuggester),
    OpenAi(OpenAiClipSuggester),
    Anthropic(AnthropicClipSuggester),
}

struct PromptApproval<'a> {
    consent: &'a dyn ConsentPrompt,
    requests: Vec<(Scope, UploadRequest)>,
    spent: Cost,
    refusal: Option<EditError>,
}

#[derive(Clone, Debug)]
pub struct ClipExportRequest {
    /// Existing output directory.
    pub directory: PathBuf,
    /// Zero-based candidate indices in candidate order; 1..=1000 distinct values.
    pub select: Vec<usize>,
    /// None/empty for no captions, or one/two distinct languages to burn.
    pub languages: Vec<LanguageTag>,
    pub formats: Vec<Format>,
    pub overwrite: bool,
    pub allow_degraded: bool,
    pub max_output_bytes: u64,
}

/// Prepared clips keep the requested candidate index for each batch slot.
pub struct ClipExportPrep {
    pub batch: ClipBatch,
    pub selected: Vec<usize>,
    pub translation_cost: Cost,
}

/// One published short in batch-slot order.
#[derive(Clone, Debug)]
pub struct ClipExportPaths {
    pub candidate: usize,
    pub paths: ClipPaths,
}

#[derive(Clone, Debug)]
pub struct ClipExportOutcome {
    pub exports: Vec<ClipExportPaths>,
    pub translation_cost: Cost,
}
mod selection;

mod ranking;

mod export;

pub use selection::{check_suggestion_credentials, choose_suggestion, suggestion_options};

use selection::{build_suggester, upload_request};

use ranking::fingerprint;

pub use ranking::suggest;

pub use export::{export, prepare_export};
