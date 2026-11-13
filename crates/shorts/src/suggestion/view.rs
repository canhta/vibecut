//! Converts clip candidates to and from the wire views a `clips.suggest` job returns.
use super::{ClipCandidate, RejectedClip};
use crate::clip_boundaries::{BoundaryAdjustment, BoundaryFailure, BoundaryKind};
use crate::error::Error;
use ai::text::clips::{ClipDropReason, ClipRole, ClipSource, ClipSourceSegment, DroppedClip};
use engine::control::{
    ClipAdjustment, ClipCandidateView, ClipEdge, ClipPieceRange, ClipPieceSpan, ClipRefusal,
    CostView,
};
use engine::providers::Cost;

pub(super) fn role_name(role: ClipRole) -> &'static str {
    match role {
        ClipRole::Hook => "hook",
        ClipRole::Context => "context",
        ClipRole::Payoff => "payoff",
    }
}
fn role_from_name(name: &str) -> Option<ClipRole> {
    match name {
        "hook" => Some(ClipRole::Hook),
        "context" => Some(ClipRole::Context),
        "payoff" => Some(ClipRole::Payoff),
        _ => None,
    }
}
fn source_name(source: ClipSource) -> &'static str {
    match source {
        ClipSource::Llm => "llm",
        ClipSource::Heuristic => "heuristic",
    }
}
fn kind_name(kind: BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::SentenceSilence => "sentence_silence",
        BoundaryKind::Sentence => "sentence",
        BoundaryKind::Silence => "silence",
        BoundaryKind::WordOrSegment => "word_or_segment",
        BoundaryKind::WordFade => "word_fade",
    }
}
fn failure_name(reason: BoundaryFailure) -> &'static str {
    match reason {
        BoundaryFailure::NoSafeFrame => "no_safe_frame",
        BoundaryFailure::SegmentDuration => "segment_duration",
        BoundaryFailure::TotalDuration => "total_duration",
        BoundaryFailure::SegmentOverlap => "segment_overlap",
        BoundaryFailure::Candidate => "candidate",
    }
}
fn drop_name(reason: ClipDropReason) -> &'static str {
    match reason {
        ClipDropReason::Duration => "duration",
        ClipDropReason::SegmentDuration => "segment_duration",
        ClipDropReason::SegmentCount => "segment_count",
        ClipDropReason::UnknownId => "unknown_id",
        ClipDropReason::ReversedRange => "reversed_range",
        ClipDropReason::HookOrder => "hook_order",
        ClipDropReason::SegmentOverlap => "segment_overlap",
        ClipDropReason::Duplicate => "duplicate",
        ClipDropReason::Overlap => "overlap",
        ClipDropReason::CountLimit => "count_limit",
    }
}
fn edge_view(edge: &BoundaryAdjustment) -> ClipEdge {
    ClipEdge {
        original: edge.original,
        adjusted: edge.adjusted,
        delta: edge.delta,
        kind: kind_name(edge.kind).into(),
    }
}

/// One candidate's wire view: the ranked proposal, its snapped pieces and the edge deltas.
pub fn candidate_view(candidate: &ClipCandidate) -> ClipCandidateView {
    ClipCandidateView {
        title: candidate.proposal.title.clone(),
        hook_text: candidate.proposal.hook_text.clone(),
        reason: candidate.proposal.reason.clone(),
        score: candidate.proposal.score,
        source: source_name(candidate.source).into(),
        duration: candidate.duration,
        segments: candidate
            .segments
            .iter()
            .map(|segment| ClipPieceRange {
                range: segment.range,
                role: role_name(segment.role).into(),
                source_order: segment.source_order,
            })
            .collect(),
        adjustments: candidate
            .adjustments
            .iter()
            .map(|pair| ClipAdjustment {
                start: edge_view(&pair.start),
                end: edge_view(&pair.end),
            })
            .collect(),
        pieces: candidate
            .proposal
            .segments
            .iter()
            .map(|piece| ClipPieceSpan {
                start_segment_id: piece.start_segment_id.clone(),
                end_segment_id: piece.end_segment_id.clone(),
                role: role_name(piece.role).into(),
            })
            .collect(),
    }
}

pub(super) fn rejected_views(rejected: &[RejectedClip]) -> Vec<ClipRefusal> {
    rejected
        .iter()
        .map(|clip| ClipRefusal {
            title: clip.title.clone(),
            reason: failure_name(clip.reason).into(),
        })
        .collect()
}
pub(super) fn dropped_views(dropped: &[DroppedClip]) -> Vec<ClipRefusal> {
    dropped
        .iter()
        .map(|clip| ClipRefusal {
            title: clip.proposal.title.clone(),
            reason: drop_name(clip.reason).into(),
        })
        .collect()
}

/// The snapped pieces of a candidate view in play order, as the batch exporter reads them.
/// A view whose role is not one of the three roles is refused, never guessed.
pub fn segments_from_view(view: &ClipCandidateView) -> Result<Vec<ClipSourceSegment>, Error> {
    view.segments
        .iter()
        .map(|piece| {
            Ok(ClipSourceSegment {
                range: piece.range,
                role: role_from_name(&piece.role).ok_or(Error::ClipExport {
                    short: 0,
                    reason: "clip piece role",
                })?,
                source_order: piece.source_order,
                repair: None,
            })
        })
        .collect()
}

pub(super) fn cost_view(cost: &Cost) -> CostView {
    CostView::from(cost)
}
