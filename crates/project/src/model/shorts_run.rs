//! The persisted result of one long-video-to-shorts run.
use crate::{Error, Result};
use captions::{LanguageTag, NfcText};
use media::AssetId;
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick};

/// Most shorts one run keeps; the generation request admits the same count.
pub const MAX_RUN_CLIPS: usize = 32;
/// Most pieces one short names, before and after boundary snapping.
pub const MAX_RUN_PIECES: usize = 32;
/// Most speech-energy windows one run keeps. Longer sources merge adjacent windows
/// (peak RMS, spanning range) before saving, so the lane still covers the whole source.
pub const MAX_RUN_ENERGY: usize = 16384;
const MAX_NAME_BYTES: usize = 4096;
const MAX_ID_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShortsRole {
    Hook,
    Context,
    Payoff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShortsSource {
    Llm,
    Heuristic,
}

/// The category the suggester gave one short: the first rule it matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShortsLabel {
    Hook,
    OnTopic,
    CleanStart,
    Standalone,
}

/// The settings of the run, as the entry screen offers them. The source file itself is the
/// run project's only asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortsRequest {
    pub language: String,
    pub transcription_provider: String,
    pub transcription_model: String,
    pub suggestion_provider: String,
    pub suggestion_model: String,
    pub count: u32,
    pub min_seconds: u32,
    pub max_seconds: u32,
    pub max_segments: u32,
}

/// One snapped source piece of a short, in play order.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortsSegment {
    pub range: SourceRange,
    pub role: ShortsRole,
    /// Zero-based rank of the piece in source order.
    pub source_order: u32,
}

/// The transcript span the suggestion named for one piece, before snapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortsPiece {
    pub start_segment_id: NfcText,
    pub end_segment_id: NfcText,
    pub role: ShortsRole,
}

/// One ranked short.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortsClip {
    pub title: NfcText,
    pub hook_text: NfcText,
    pub reason: NfcText,
    pub score: f64,
    pub source: ShortsSource,
    pub label: ShortsLabel,
    /// Sum of the segment durations, excluding gaps.
    pub duration: Tick,
    pub segments: Vec<ShortsSegment>,
    pub pieces: Vec<ShortsPiece>,
}

/// One measured speech-energy window: mono RMS amplitude ratio over `range`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortsEnergy {
    pub range: SourceRange,
    pub rms: f32,
}

/// What a finished `clips.suggest` run keeps beside its project: the source asset and the
/// source-timed caption cues live in the project itself, the suggestions live here. A project
/// with this field is a run of the shorts window, never an editor project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortsRun {
    /// The project asset the run was made from.
    pub asset: AssetId,
    /// The source file name the entry screen shows.
    pub source_name: NfcText,
    pub source_language: LanguageTag,
    pub transcription_provider: String,
    pub transcription_model: String,
    pub request: ShortsRequest,
    pub clips: Vec<ShortsClip>,
    pub energy: Vec<ShortsEnergy>,
}

impl ShortsRun {
    /// Whole-run admission; `Project::new` calls it with the project's asset ids.
    pub(crate) fn validate(&self, has_asset: impl Fn(AssetId) -> bool) -> Result<()> {
        if !has_asset(self.asset) {
            return Err(Error::Invalid("shorts run asset"));
        }
        let name = self.source_name.as_str();
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(Error::Invalid("shorts run source name"));
        }
        for text in [
            &self.transcription_provider,
            &self.transcription_model,
            &self.request.language,
            &self.request.transcription_provider,
            &self.request.transcription_model,
            &self.request.suggestion_provider,
            &self.request.suggestion_model,
        ] {
            if text.len() > MAX_ID_BYTES {
                return Err(Error::Invalid("shorts run identifier"));
            }
        }
        let request = &self.request;
        if !(1..=32).contains(&request.count)
            || request.min_seconds == 0
            || request.min_seconds > request.max_seconds
            || request.max_seconds > 180
            || !(1..=32).contains(&request.max_segments)
        {
            return Err(Error::Invalid("shorts run request"));
        }
        if self.clips.len() > MAX_RUN_CLIPS {
            return Err(Error::TooLarge("shorts run clips"));
        }
        for clip in &self.clips {
            super::project::text(&clip.title)?;
            super::project::text(&clip.hook_text)?;
            super::project::text(&clip.reason)?;
            if !clip.score.is_finite()
                || clip.duration < Tick::ZERO
                || clip.segments.is_empty()
                || clip.segments.len() > MAX_RUN_PIECES
                || clip.pieces.len() > MAX_RUN_PIECES
                || clip
                    .pieces
                    .iter()
                    .any(|piece| piece.start_segment_id.as_str().len() > MAX_ID_BYTES)
                || clip
                    .pieces
                    .iter()
                    .any(|piece| piece.end_segment_id.as_str().len() > MAX_ID_BYTES)
                || clip
                    .segments
                    .iter()
                    .any(|segment| segment.source_order as usize >= clip.segments.len())
            {
                return Err(Error::Invalid("shorts run clip"));
            }
        }
        if self.energy.len() > MAX_RUN_ENERGY {
            return Err(Error::TooLarge("shorts run energy"));
        }
        if self
            .energy
            .iter()
            .any(|window| !window.rms.is_finite() || window.rms < 0.0)
        {
            return Err(Error::Invalid("shorts run energy"));
        }
        Ok(())
    }
}
