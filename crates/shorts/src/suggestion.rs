//! Produces source-snapped suggestions for accepted shorts.
use crate::clip_boundaries::{
    BoundaryFailure, ClipBoundarySettings, SegmentAdjustment, snap_clip_boundaries,
};
use crate::error::Error;
use ai::Scope;
use ai::clips::{
    AnthropicClipSuggester, ClipInput, ClipProposal, ClipSegment, ClipSettings, ClipSource,
    ClipSourceSegment, ClipSuggester, ClipSuggestion, HeuristicClipSuggester, OpenAiClipSuggester,
};
use captions::{Cue, LanguageTag, NfcText};
use engine::control::Error as EngineError;
use engine::providers::{CatalogOperation, Cost, Credentials, UploadRequest};
use media::SourceAsset;
use std::sync::Arc;
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

pub(crate) use export::submit_export;

pub(crate) use job::submit_suggest;

pub use view::{candidate_view, segments_from_view};
