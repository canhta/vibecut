//! Ranked continuous source ranges. Engine owns scheduling, retries and aggregate budgets.
mod heuristic;
mod remote;
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, Operation, ProviderConfig, Request, Scope,
    Stage,
};
use captions::{LanguageTag, NfcText};
pub use heuristic::HeuristicClipSuggester;
pub use remote::{AnthropicClipSuggester, ClipDisclosure, OpenAiClipSuggester};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{SourceRange, Tick};

pub const MODE: &str = "transcript-clips";
pub const MAX_WINDOW_SEGMENTS: usize = 128;
pub const MAX_WINDOW_BYTES: usize = 24 * 1024;
pub const MAX_WIRE: u64 = 64 * 1024;
const MAX_WINDOWS: usize = 1024;
const MAX_OBSERVATIONS: usize = 4096;
pub const HEURISTIC_SETTINGS: &str = "clips-heuristic-v1;sentence-pause-keyword;char-bigram-dedupe";
pub const OPENAI_SETTINGS: &str =
    "clips-openai-v1;responses;strict;store=false;max_output=8192;char-bigram-dedupe";
pub const ANTHROPIC_SETTINGS: &str =
    "clips-anthropic-v1;messages;json_schema;max_output=8192;char-bigram-dedupe";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSegment {
    pub id: NfcText,
    pub text: NfcText,
    pub range: SourceRange,
}
impl From<&captions::Segment> for ClipSegment {
    fn from(s: &captions::Segment) -> Self {
        Self {
            id: s.id.clone(),
            text: s.text.clone(),
            range: s.range,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSettings {
    /// Maximum returned suggestions, 1..=32. Fewer may be possible.
    pub count: usize,
    /// Positive duration bounds in source ticks, inclusive.
    pub min_duration: Tick,
    pub max_duration: Tick,
    /// Maximum range intersection-over-union allowed between returned candidates.
    pub overlap_tolerance: f64,
    pub language: LanguageTag,
    /// Exact NFC keywords for local density ranking; also included in remote input.
    pub keywords: Vec<NfcText>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipInput {
    pub segments: Vec<ClipSegment>,
    pub settings: ClipSettings,
}
impl ClipInput {
    pub fn validate(&self) -> Result<(), Error> {
        let s = &self.settings;
        if !(1..=32).contains(&s.count)
            || s.min_duration <= Tick::ZERO
            || s.max_duration < s.min_duration
            || !s.overlap_tolerance.is_finite()
            || !(0.0..=1.0).contains(&s.overlap_tolerance)
            || s.keywords.len() > 64
            || s.keywords
                .iter()
                .any(|k| k.as_str().trim().is_empty() || k.as_str().len() > 256)
        {
            return Err(Error::Invalid("clip settings"));
        }
        if self.segments.is_empty() || self.segments.len() > 10_000 {
            return Err(Error::Payload);
        }
        let mut ids = BTreeSet::new();
        let mut end = Tick::ZERO;
        let mut bytes = 0usize;
        for seg in &self.segments {
            if seg.id.as_str().trim().is_empty()
                || seg.id.as_str().len() > 256
                || seg.text.as_str().trim().is_empty()
                || seg.range.start() < end
                || !ids.insert(&seg.id)
            {
                return Err(Error::Invalid("clip segment"));
            }
            end = seg.range.end();
            bytes = bytes
                .saturating_add(seg.text.as_str().len())
                .saturating_add(seg.id.as_str().len());
        }
        if bytes > 4 * 1024 * 1024 {
            return Err(Error::Payload);
        }
        Ok(())
    }
    /// Exact complete text bound to each invocation's consent, including IDs, settings and ranges.
    pub fn consent_text(&self) -> Result<NfcText, Error> {
        self.validate()?;
        if self.segments.len() > MAX_WINDOW_SEGMENTS {
            return Err(Error::Payload);
        }
        let text = serde_json::to_string(self).map_err(|_| Error::Payload)?;
        if text.len() > MAX_WINDOW_BYTES {
            return Err(Error::Payload);
        }
        Ok(text.into())
    }
    /// Plan bounded overlapping windows before requesting consent or reserving costs.
    /// Overlap covers max_duration so every admissible cross-window range is visible.
    /// Refuses density too high to preserve this guarantee within the wire bounds.
    pub fn windows(&self) -> Result<Vec<Self>, Error> {
        self.validate()?;
        let mut windows = Vec::new();
        let mut start = 0;
        while start < self.segments.len() {
            if windows.len() >= MAX_WINDOWS {
                return Err(Error::Payload);
            }
            let mut window = Self {
                segments: Vec::new(),
                settings: self.settings.clone(),
            };
            let mut end = start;
            for seg in self.segments.iter().skip(start).take(MAX_WINDOW_SEGMENTS) {
                window.segments.push(seg.clone());
                if window.consent_text().is_err() {
                    window.segments.pop();
                    break;
                }
                end += 1;
            }
            let last = window.segments.last().ok_or(Error::Payload)?;
            if end == self.segments.len() {
                windows.push(window);
                break;
            }
            let cutoff = last
                .range
                .end()
                .0
                .saturating_sub(self.settings.max_duration.0);
            let next = self
                .segments
                .iter()
                .enumerate()
                .skip(start)
                .take(end - start)
                .find(|(_, s)| s.range.end().0 > cutoff)
                .map(|(i, _)| i)
                .ok_or(Error::Payload)?;
            if next <= start {
                return Err(Error::Payload);
            }
            windows.push(window);
            start = next;
        }
        Ok(windows)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipProposal {
    pub start_segment_id: NfcText,
    pub end_segment_id: NfcText,
    pub title: NfcText,
    pub hook: NfcText,
    pub reason: NfcText,
    /// Ranking score only, never a probability.
    pub score: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipSource {
    Llm,
    Heuristic,
}
#[derive(Clone, Debug, Serialize)]
pub struct ClipSuggestion {
    pub proposal: ClipProposal,
    pub range: SourceRange,
    pub source: ClipSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ClipDropReason {
    Duration,
    Duplicate,
    Overlap,
    CountLimit,
}
#[derive(Clone, Debug, Serialize)]
pub struct DroppedClip {
    pub proposal: ClipProposal,
    pub reason: ClipDropReason,
}
#[derive(Clone, Debug, Serialize)]
pub struct ClipSuggestions {
    pub clips: Vec<ClipSuggestion>,
    pub dropped: Vec<DroppedClip>,
}

/// Blocking single-attempt adapter. A remote error remains typed; callers may offer
/// an explicit rerun using HeuristicClipSuggester, never silently switch providers.
pub trait ClipSuggester: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    fn source(&self) -> ClipSource;
    fn suggest(
        &self,
        window: &ClipInput,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<ClipProposal>, Error>;
}
pub(super) fn prepare(
    provider: &dyn ClipSuggester,
    window: &ClipInput,
    request: &Request<'_>,
) -> Result<(), Error> {
    if provider.config() != &request.scope.provider {
        return Err(Error::Unsupported);
    }
    if request.scope.source_language != window.settings.language
        || request.scope.target_language.is_some()
        || request.scope.texts != [window.consent_text()?]
    {
        return Err(Error::ConsentRequired);
    }
    if window.consent_text()?.as_str().len() as u64 > request.scope.max_bytes {
        return Err(Error::Payload);
    }
    request.prepare(
        &provider.capabilities(request.scope),
        &provider.estimate(request.scope),
        Operation::SuggestClips,
    )
}
/// Execute a preplanned set of windows atomically. Supply one independently costed,
/// consent-bound Request per window in windows() order. All approvals are checked
/// before the first IO; any failure discards the result, with no fallback or retry.
pub fn suggest_clips(
    provider: &dyn ClipSuggester,
    input: &ClipInput,
    requests: &[Request<'_>],
    hooks: &dyn ExecutionHooks,
) -> Result<ClipSuggestions, Error> {
    crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
    let windows = input.windows()?;
    if windows.len() != requests.len() {
        return Err(Error::Invalid("clip window requests"));
    }
    for (w, r) in windows.iter().zip(requests) {
        prepare(provider, w, r)?;
    }
    let mut observations = BTreeMap::new();
    for (w, r) in windows.iter().zip(requests) {
        crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
        let proposals = provider.suggest(w, r, hooks)?;
        let validated = validate(w, proposals, provider.source())?;
        for clip in validated.clips {
            observe(&mut observations, clip.proposal, Some(clip.range))?;
        }
        for drop in validated.dropped {
            observe(&mut observations, drop.proposal, None)?;
        }
    }
    crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
    let mut all = Vec::new();
    let mut dropped = Vec::new();
    for observation in observations.into_values() {
        if let Some(range) = observation.range {
            all.push(ClipSuggestion {
                proposal: observation.proposal,
                range,
                source: provider.source(),
            });
        } else {
            dropped.push(DroppedClip {
                proposal: observation.proposal,
                reason: ClipDropReason::Duration,
            });
        }
        if let Some(proposal) = observation.duplicate {
            dropped.push(DroppedClip {
                proposal,
                reason: ClipDropReason::Duplicate,
            });
        }
    }
    merge(input, all, dropped, hooks)
}

struct WindowObservation {
    proposal: ClipProposal,
    // The same endpoint IDs always resolve to the same source range and duration
    // verdict, regardless of which overlapping window observed them.
    range: Option<SourceRange>,
    duplicate: Option<ClipProposal>,
}
fn observe(
    observations: &mut BTreeMap<(NfcText, NfcText), WindowObservation>,
    proposal: ClipProposal,
    range: Option<SourceRange>,
) -> Result<(), Error> {
    let key = (
        proposal.start_segment_id.clone(),
        proposal.end_segment_id.clone(),
    );
    if let Some(previous) = observations.get_mut(&key) {
        // Strict comparison retains the first observation on score ties, matching
        // the stable final sort. Duplicate diagnostics retain one example per pair.
        let discarded = if proposal.score.total_cmp(&previous.proposal.score).is_gt() {
            std::mem::replace(&mut previous.proposal, proposal)
        } else {
            proposal
        };
        if previous.range.is_some() && previous.duplicate.is_none() {
            previous.duplicate = Some(discarded);
        }
    } else {
        if observations.len() >= MAX_OBSERVATIONS {
            return Err(Error::Payload);
        }
        observations.insert(
            key,
            WindowObservation {
                proposal,
                range,
                duplicate: None,
            },
        );
    }
    Ok(())
}
fn validate(
    input: &ClipInput,
    proposals: Vec<ClipProposal>,
    source: ClipSource,
) -> Result<ClipSuggestions, Error> {
    if proposals.len() > MAX_WINDOW_SEGMENTS {
        return Err(Error::Schema);
    }
    let ids: BTreeMap<_, _> = input
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| (&s.id, (i, s)))
        .collect();
    let mut result = ClipSuggestions {
        clips: Vec::new(),
        dropped: Vec::new(),
    };
    for p in proposals {
        if !p.score.is_finite()
            || !(0.0..=1.0).contains(&p.score)
            || [&p.title, &p.hook, &p.reason]
                .iter()
                .any(|s| s.as_str().trim().is_empty() || s.as_str().len() > 2048)
        {
            return Err(Error::Schema);
        }
        let (a, first) = ids.get(&p.start_segment_id).ok_or(Error::Schema)?;
        let (b, last) = ids.get(&p.end_segment_id).ok_or(Error::Schema)?;
        if a > b {
            return Err(Error::Schema);
        }
        let range = SourceRange::new(first.range.start(), last.range.end())?;
        if range.duration() < input.settings.min_duration
            || range.duration() > input.settings.max_duration
        {
            result.dropped.push(DroppedClip {
                proposal: p,
                reason: ClipDropReason::Duration,
            });
        } else {
            result.clips.push(ClipSuggestion {
                proposal: p,
                range,
                source,
            });
        }
    }
    Ok(result)
}
fn merge(
    input: &ClipInput,
    mut all: Vec<ClipSuggestion>,
    mut dropped: Vec<DroppedClip>,
    hooks: &dyn ExecutionHooks,
) -> Result<ClipSuggestions, Error> {
    all.sort_by(|a, b| {
        b.proposal
            .score
            .total_cmp(&a.proposal.score)
            .then(a.range.start().cmp(&b.range.start()))
            .then(a.range.end().cmp(&b.range.end()))
    });
    let mut kept: Vec<(ClipSuggestion, BTreeSet<(char, char)>)> = Vec::new();
    for clip in all {
        crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
        let text = bigrams(input, clip.range);
        let mut reason = None;
        for (other, other_text) in &kept {
            let intersection = clip
                .range
                .intersect(other.range)
                .map_or(0, |r| r.duration().0);
            let union = i128::from(clip.range.duration().0) + i128::from(other.range.duration().0)
                - i128::from(intersection);
            let iou = intersection as f64 / union as f64;
            let similarity = text.intersection(other_text).count() as f64
                / text.union(other_text).count().max(1) as f64;
            if clip.range == other.range || (iou >= 0.5 && similarity >= 0.8) {
                reason = Some(ClipDropReason::Duplicate);
                break;
            }
            if iou > input.settings.overlap_tolerance {
                reason = Some(ClipDropReason::Overlap);
                break;
            }
        }
        if reason.is_none() && kept.len() >= input.settings.count {
            reason = Some(ClipDropReason::CountLimit);
        }
        if let Some(reason) = reason {
            dropped.push(DroppedClip {
                proposal: clip.proposal,
                reason,
            });
        } else {
            kept.push((clip, text));
        }
    }
    Ok(ClipSuggestions {
        clips: kept.into_iter().map(|(c, _)| c).collect(),
        dropped,
    })
}
fn bigrams(input: &ClipInput, range: SourceRange) -> BTreeSet<(char, char)> {
    let chars: Vec<_> = input
        .segments
        .iter()
        .filter(|s| range.overlaps(s.range))
        .flat_map(|s| s.text.as_str().chars())
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect();
    chars
        .windows(2)
        .filter_map(|p| Some((*p.first()?, *p.get(1)?)))
        .collect()
}

/// UI/CLI may present this choice after a remote invocation fails. Returning the
/// choice performs no work and grants no consent; the user must select it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipRecoveryChoice {
    RerunLocally,
}
pub fn clip_recovery_choice(error: &Error) -> Option<ClipRecoveryChoice> {
    match error {
        Error::Network
        | Error::Server
        | Error::RateLimit { .. }
        | Error::Schema
        | Error::Incomplete
        | Error::Refused
        | Error::Auth
        | Error::Quota
        | Error::Cost => Some(ClipRecoveryChoice::RerunLocally),
        _ => None,
    }
}
