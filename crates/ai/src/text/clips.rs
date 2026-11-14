//! Ranked shorts assembled from ordered source segments. Engine owns scheduling, retries and aggregate budgets.
mod heuristic;
mod remote;
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, Operation, ProviderConfig, Request, Scope,
    Stage,
};
use captions::{LanguageTag, NfcText};
pub use heuristic::HeuristicClipSuggester;
pub use remote::{AnthropicClipSuggester, OpenAiClipSuggester};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{SourceRange, Tick};

pub const MODE: &str = "transcript-clips";
impl Scope {
    /// Consent scope of one clip-suggestion window. The fingerprint identifies the
    /// source the transcript came from.
    pub fn clip_suggestion(
        provider: ProviderConfig,
        window: &ClipInput,
        payload_fingerprint: NfcText,
    ) -> Result<Self, Error> {
        Ok(Self {
            provider,
            operation: Operation::SuggestClips,
            payload_fingerprint,
            mode: MODE.into(),
            texts: vec![window.consent_text()?],
            source_language: window.settings.language.clone(),
            target_language: None,
            source_samples: crate::SampleRange::new(0, 1_000)?,
            sample_rate: 1,
            max_bytes: MAX_WIRE,
        })
    }
}
pub const MAX_WINDOW_SEGMENTS: usize = 128;
pub const MAX_WINDOW_BYTES: usize = 24 * 1024;
pub const MAX_WIRE: u64 = 64 * 1024;
const MAX_WINDOWS: usize = 1024;
const MAX_OBSERVATIONS: usize = 4096;
pub const HEURISTIC_SETTINGS: &str = "clips-heuristic-v2;sentence-pause-keyword;char-bigram-dedupe";
pub const OPENAI_SETTINGS: &str =
    "clips-openai-v2;responses;strict;store=false;max_output=8192;char-bigram-dedupe";
pub const ANTHROPIC_SETTINGS: &str =
    "clips-anthropic-v2;messages;json_schema;max_output=8192;char-bigram-dedupe";

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
    /// Maximum pieces per short, 1..=32; defaults to four in JSON.
    #[serde(default = "default_max_segments")]
    pub max_segments: usize,
    /// Minimum duration of each canonical piece in source ticks (positive).
    pub min_segment_duration: Tick,
    /// Maximum summed source intersection-over-union between returned candidates.
    pub overlap_tolerance: f64,
    pub language: LanguageTag,
    /// Exact NFC keywords for local density ranking; also included in remote input.
    pub keywords: Vec<NfcText>,
}
fn default_max_segments() -> usize {
    4
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
            || !(1..=32).contains(&s.max_segments)
            || s.min_segment_duration <= Tick::ZERO
            || s.min_segment_duration > s.max_duration
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
    /// Overlap covers max_duration so every admissible contiguous cross-window range is visible.
    /// Noncontiguous assembly uses only IDs present together in one window.
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
/// Editorial role; play order may differ from source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipRole {
    Hook,
    Context,
    Payoff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipPiece {
    pub start_segment_id: NfcText,
    pub end_segment_id: NfcText,
    pub role: ClipRole,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipProposal {
    /// Pieces in playback order. IDs within each piece are in source order.
    pub segments: Vec<ClipPiece>,
    pub title: NfcText,
    pub hook_text: NfcText,
    pub reason: NfcText,
    /// Ranking score only, never a probability.
    pub score: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipRepairReason {
    AdjacentMerged,
}
/// Resolved piece in playback order; source_order is its zero-based source rank.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipSourceSegment {
    pub range: SourceRange,
    pub role: ClipRole,
    pub source_order: usize,
    /// Present when canonicalization merged touching pieces into this segment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<ClipRepairReason>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipSource {
    Llm,
    Heuristic,
}
/// Category of one short. The first matching rule wins: `Hook` when the opening piece is
/// the hook, `OnTopic` at keyword density `ON_TOPIC_DENSITY`, `CleanStart` when the opening
/// segment begins a sentence, otherwise `Standalone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClipLabel {
    Hook,
    OnTopic,
    CleanStart,
    Standalone,
}
/// Keyword density (see `keyword_density`) at or above which a short is `OnTopic`.
pub(super) const ON_TOPIC_DENSITY: f64 = 0.5;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipSuggestion {
    pub proposal: ClipProposal,
    pub segments: Vec<ClipSourceSegment>,
    /// Sum of piece durations, excluding gaps, in ticks.
    pub duration: Tick,
    pub label: ClipLabel,
    pub source: ClipSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipDropReason {
    Duration,
    SegmentDuration,
    SegmentCount,
    UnknownId,
    ReversedRange,
    HookOrder,
    SegmentOverlap,
    Duplicate,
    Overlap,
    CountLimit,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DroppedClip {
    pub proposal: ClipProposal,
    pub reason: ClipDropReason,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// Provider-neutral upload disclosure for this exact scope; the local
    /// heuristic returns `Ok(None)` and never uploads.
    fn disclosure(&self, _scope: &Scope) -> Result<Option<crate::UploadDisclosure>, Error> {
        Ok(None)
    }
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
            observe(
                &mut observations,
                clip.proposal,
                Some((clip.segments, clip.label)),
                ClipDropReason::Duration,
            )?;
        }
        for drop in validated.dropped {
            observe(&mut observations, drop.proposal, None, drop.reason)?;
        }
        // A running count for a live view; ranking and the count limit still apply below.
        let produced = observations
            .values()
            .filter(|observation| observation.segments.is_some())
            .count();
        hooks.produced(u32::try_from(produced).unwrap_or(u32::MAX));
    }
    crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
    let mut all = Vec::new();
    let mut dropped = Vec::new();
    for observation in observations.into_values() {
        if let Some((segments, label)) = observation.segments {
            all.push(ClipSuggestion {
                proposal: observation.proposal,
                duration: duration(&segments)?,
                segments,
                label,
                source: provider.source(),
            });
        } else {
            dropped.push(DroppedClip {
                proposal: observation.proposal,
                reason: observation.reason,
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
    segments: Option<(Vec<ClipSourceSegment>, ClipLabel)>,
    reason: ClipDropReason,
    duplicate: Option<ClipProposal>,
}
fn observe(
    observations: &mut BTreeMap<Vec<(NfcText, NfcText, ClipRole)>, WindowObservation>,
    proposal: ClipProposal,
    segments: Option<(Vec<ClipSourceSegment>, ClipLabel)>,
    reason: ClipDropReason,
) -> Result<(), Error> {
    let key = proposal
        .segments
        .iter()
        .map(|s| (s.start_segment_id.clone(), s.end_segment_id.clone(), s.role))
        .collect();
    if let Some(previous) = observations.get_mut(&key) {
        // A hallucinated out-of-window ID can be valid in a later window. Only
        // validated observations may supply the score/metadata of an accepted short.
        if previous.segments.is_none() && segments.is_some() {
            *previous = WindowObservation {
                proposal,
                segments,
                reason,
                duplicate: None,
            };
            return Ok(());
        }
        if previous.segments.is_some() && segments.is_none() {
            return Ok(());
        }
        // Strict comparison retains the first observation on score ties, matching
        // the stable final sort. Duplicate diagnostics retain one example per pair.
        let discarded = if proposal.score.total_cmp(&previous.proposal.score).is_gt() {
            previous.segments = segments;
            std::mem::replace(&mut previous.proposal, proposal)
        } else {
            proposal
        };
        if previous.segments.is_some() && previous.duplicate.is_none() {
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
                segments,
                reason,
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
    for mut p in proposals {
        if !p.score.is_finite()
            || !(0.0..=1.0).contains(&p.score)
            || [&p.title, &p.hook_text, &p.reason]
                .iter()
                .any(|s| s.as_str().trim().is_empty() || s.as_str().len() > 2048)
            || p.segments.len() > MAX_WINDOW_SEGMENTS
            || p.segments.iter().any(|s| {
                s.start_segment_id.as_str().len() > 256 || s.end_segment_id.as_str().len() > 256
            })
        {
            return Err(Error::Schema);
        }
        match resolve(input, &ids, &mut p) {
            Ok(segments) => result.clips.push(ClipSuggestion {
                duration: duration(&segments)?,
                label: label(input, &p, &segments),
                proposal: p,
                segments,
                source,
            }),
            Err(reason) => result.dropped.push(DroppedClip {
                proposal: p,
                reason,
            }),
        }
    }
    Ok(result)
}
/// Keyword hits per character, scaled so one hit per 20 characters is 1.0. Shared by every
/// suggester so the same text gets the same density.
pub(super) fn keyword_density(keywords: &[NfcText], text: &str) -> f64 {
    let lowered = text.to_lowercase();
    let hits: usize = keywords
        .iter()
        .map(|k| lowered.matches(&k.as_str().to_lowercase()).count())
        .sum();
    (hits as f64 / lowered.chars().count().max(1) as f64 * 20.0).min(1.0)
}
/// True when the segment text ends a sentence.
pub(super) fn sentence(s: &str) -> bool {
    s.trim_end().ends_with(['.', '?', '!', '。', '？', '！'])
}
/// Label from the opening role, the keyword density of the clip's transcript text and
/// whether the opening segment starts a sentence. A pure function of its inputs.
fn label(input: &ClipInput, proposal: &ClipProposal, segments: &[ClipSourceSegment]) -> ClipLabel {
    if proposal
        .segments
        .first()
        .is_some_and(|piece| piece.role == ClipRole::Hook)
    {
        return ClipLabel::Hook;
    }
    let text = input
        .segments
        .iter()
        .filter(|s| segments.iter().any(|piece| piece.range.overlaps(s.range)))
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if keyword_density(&input.settings.keywords, &text) >= ON_TOPIC_DENSITY {
        return ClipLabel::OnTopic;
    }
    let opening = segments.first().map(|s| s.range.start());
    let starts_sentence = input
        .segments
        .iter()
        .position(|s| Some(s.range.start()) == opening)
        .is_some_and(|i| {
            i.checked_sub(1)
                .and_then(|previous| input.segments.get(previous))
                .is_none_or(|previous| sentence(previous.text.as_str()))
        });
    if starts_sentence {
        ClipLabel::CleanStart
    } else {
        ClipLabel::Standalone
    }
}
fn duration(segments: &[ClipSourceSegment]) -> Result<Tick, Error> {
    segments.iter().try_fold(Tick::ZERO, |sum, s| {
        sum.0
            .checked_add(s.range.duration().0)
            .map(Tick)
            .ok_or(Error::Payload)
    })
}
fn resolve(
    input: &ClipInput,
    ids: &BTreeMap<&NfcText, (usize, &ClipSegment)>,
    proposal: &mut ClipProposal,
) -> Result<Vec<ClipSourceSegment>, ClipDropReason> {
    if proposal.segments.is_empty() || proposal.segments.len() > input.settings.max_segments {
        return Err(ClipDropReason::SegmentCount);
    }
    let hooks = proposal
        .segments
        .iter()
        .filter(|s| s.role == ClipRole::Hook)
        .count();
    if hooks > 0
        && (hooks != 1
            || proposal
                .segments
                .first()
                .is_none_or(|s| s.role != ClipRole::Hook))
    {
        return Err(ClipDropReason::HookOrder);
    }
    let mut pieces: Vec<ClipPiece> = Vec::new();
    let mut resolved: Vec<ClipSourceSegment> = Vec::new();
    for piece in &proposal.segments {
        let (a, first) = ids
            .get(&piece.start_segment_id)
            .ok_or(ClipDropReason::UnknownId)?;
        let (b, last) = ids
            .get(&piece.end_segment_id)
            .ok_or(ClipDropReason::UnknownId)?;
        if a > b {
            return Err(ClipDropReason::ReversedRange);
        }
        let range = SourceRange::new(first.range.start(), last.range.end())
            .map_err(|_| ClipDropReason::ReversedRange)?;
        if resolved.iter().any(|s| s.range.overlaps(range)) {
            return Err(ClipDropReason::SegmentOverlap);
        }
        if let (Some(previous), Some(previous_piece)) = (resolved.last_mut(), pieces.last_mut())
            && previous.range.end() == range.start()
        {
            // Canonical adjacency retains the opening piece's editorial role.
            previous.range = SourceRange::new(previous.range.start(), range.end())
                .map_err(|_| ClipDropReason::ReversedRange)?;
            previous_piece.end_segment_id = piece.end_segment_id.clone();
            previous.repair = Some(ClipRepairReason::AdjacentMerged);
            continue;
        }
        pieces.push(piece.clone());
        resolved.push(ClipSourceSegment {
            range,
            role: piece.role,
            source_order: 0,
            repair: None,
        });
    }
    if resolved
        .iter()
        .any(|s| s.range.duration() < input.settings.min_segment_duration)
    {
        return Err(ClipDropReason::SegmentDuration);
    }
    let total = duration(&resolved).map_err(|_| ClipDropReason::Duration)?;
    if total < input.settings.min_duration || total > input.settings.max_duration {
        return Err(ClipDropReason::Duration);
    }
    let mut order: Vec<_> = (0..resolved.len()).collect();
    order.sort_by_key(|i| resolved.get(*i).map(|s| s.range.start()));
    for (rank, index) in order.into_iter().enumerate() {
        if let Some(s) = resolved.get_mut(index) {
            s.source_order = rank;
        }
    }
    proposal.segments = pieces;
    Ok(resolved)
}

fn merge(
    input: &ClipInput,
    mut all: Vec<ClipSuggestion>,
    mut dropped: Vec<DroppedClip>,
    hooks: &dyn ExecutionHooks,
) -> Result<ClipSuggestions, Error> {
    all.sort_by(|a, b| {
        b.proposal.score.total_cmp(&a.proposal.score).then_with(|| {
            a.segments
                .iter()
                .map(|s| (s.range.start(), s.range.end()))
                .cmp(b.segments.iter().map(|s| (s.range.start(), s.range.end())))
        })
    });
    let mut kept: Vec<(ClipSuggestion, BTreeSet<(char, char)>)> = Vec::new();
    for clip in all {
        crate::contracts::checkpoint(hooks, Stage::SuggestClips)?;
        let text = bigrams(input, &clip.segments);
        let mut reason = None;
        for (other, other_text) in &kept {
            let intersection: i128 = clip
                .segments
                .iter()
                .flat_map(|s| {
                    other
                        .segments
                        .iter()
                        .filter_map(move |o| s.range.intersect(o.range))
                })
                .map(|r| i128::from(r.duration().0))
                .sum();
            let union = i128::from(clip.duration.0) + i128::from(other.duration.0) - intersection;
            let iou = intersection as f64 / union as f64;
            let similarity = text.intersection(other_text).count() as f64
                / text.union(other_text).count().max(1) as f64;
            if (intersection == i128::from(clip.duration.0)
                && intersection == i128::from(other.duration.0))
                || (iou >= 0.5 && similarity >= 0.8)
            {
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
fn bigrams(input: &ClipInput, segments: &[ClipSourceSegment]) -> BTreeSet<(char, char)> {
    let chars: Vec<_> = input
        .segments
        .iter()
        .filter(|s| segments.iter().any(|piece| piece.range.overlaps(s.range)))
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
