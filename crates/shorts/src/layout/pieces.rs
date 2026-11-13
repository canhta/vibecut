//! Cuts one segment into the pieces a shot boundary separates.
use super::choice::Evidence;
use engine::ShotAnalysis;
use time::{SourceRange, Tick};

/// A stretch of a segment inside one shot, or outside every analysed shot.
#[derive(Clone, Copy)]
pub(super) struct Piece<'a> {
    pub range: SourceRange,
    pub evidence: Option<Evidence<'a>>,
}

/// Split `segment` at the boundaries of the ordered, disjoint `shots`. Stretches
/// the analysis does not cover are pieces without evidence. A piece shorter
/// than `min` is absorbed by its neighbour, so no piece lasts under a frame.
pub(super) fn split(segment: SourceRange, shots: &[ShotAnalysis], min: Tick) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut cursor = segment.start();
    let mut push = |start: Tick, end: Tick, evidence: Option<Evidence<'_>>| {
        if let Ok(range) = SourceRange::new(start, end) {
            pieces.push(Piece { range, evidence });
        }
    };
    for (index, shot) in shots.iter().enumerate() {
        if shot.range.end() <= cursor {
            continue;
        }
        if shot.range.start() >= segment.end() {
            break;
        }
        if shot.range.start() > cursor {
            push(cursor, shot.range.start(), None);
        }
        let start = shot.range.start().max(cursor);
        let end = shot.range.end().min(segment.end());
        push(start, end, Some(Evidence { index, shot }));
        cursor = end;
    }
    if cursor < segment.end() {
        push(cursor, segment.end(), None);
    }
    absorb_slivers(&mut pieces, min);
    pieces
}

fn absorb_slivers(pieces: &mut Vec<Piece<'_>>, min: Tick) {
    let mut at = 0;
    while at < pieces.len() {
        let short = pieces
            .get(at)
            .is_some_and(|piece| piece.range.duration() < min);
        if !short || pieces.len() < 2 {
            at += 1;
            continue;
        }
        let removed = pieces.remove(at);
        if at > 0 {
            if let Some(previous) = pieces.get_mut(at - 1)
                && let Ok(range) = SourceRange::new(previous.range.start(), removed.range.end())
            {
                previous.range = range;
            }
        } else if let Some(next) = pieces.get_mut(0)
            && let Ok(range) = SourceRange::new(removed.range.start(), next.range.end())
        {
            next.range = range;
        }
    }
}
