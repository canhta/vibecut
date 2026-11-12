//! Produces source-snapped suggestions for accepted shorts.
use crate::clip_boundaries::{
    BoundaryFailure, ClipBoundarySettings, SegmentAdjustment, snap_clip_boundaries,
};
use crate::error::Error;
use ai::clips::{
    AnthropicClipSuggester, ClipInput, ClipProposal, ClipSegment, ClipSettings, ClipSource,
    ClipSourceSegment, ClipSuggester, ClipSuggestion, DroppedClip, HeuristicClipSuggester,
    OpenAiClipSuggester,
};
use ai::{CostEstimate, Money, Scope};
use captions::{Cue, LanguageTag, NfcText};
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::control::Error as EngineError;
use engine::jobs::{JobBytes, JobKind, JobRunner, JobSpec, ResourceClass};
use engine::providers::{
    CatalogOperation, ConsentPrompt, Cost, Credentials, Providers, UploadRequest,
};
use engine::transcript::{
    Cancellation, Progress, Stage, Transcription, build_session, transcribe, wait_for_job,
};
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
    /// Probed source duration.
    pub duration: Tick,
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

mod selection;

mod ranking;

mod export;

mod job;

mod view;

pub use selection::{check_suggestion_credentials, choose_suggestion, suggestion_options};

use selection::{build_suggester, upload_request};

use ranking::fingerprint;

pub use ranking::suggest;

pub(crate) use export::submit_export;

pub(crate) use job::submit_suggest;

pub use view::{candidate_view, clips_result, segments_from_view};
