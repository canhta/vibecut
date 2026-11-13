//! Plans the layout of every shot piece of a short in play order.
use super::choice::{self, Choice};
use super::composition::{self, Geometry, Slot};
use super::overrides::{Layout, LayoutOverrides};
use super::pieces;
use engine::ShotAnalysis;
use time::{SourceRange, Tick};

/// The layout one stretch of a short gets. Layouts change only where a shot
/// ends, so each stretch lies inside one shot (or outside the analysis).
#[derive(Clone, Debug, PartialEq)]
pub struct ShotLayout {
    /// Play-order index of the segment holding the stretch.
    pub segment: usize,
    /// The stretch in source time.
    pub range: SourceRange,
    /// The layout in force.
    pub layout: Layout,
    /// The layout asked for, by override or by the automatic choice, failed its
    /// condition and `layout` is the Fill or Fit fallback.
    pub fallback: bool,
    pub(crate) slots: Vec<Slot>,
}

/// Plan every segment of a short. `segments` are the source ranges in play
/// order and `shots` the ordered, disjoint analysis of the source (empty means
/// no evidence, so every stretch is a centred Fill). Adjacent stretches of one
/// segment with the same layout and slots are one stretch, so a shot cut that
/// changes nothing adds no item edge.
pub(crate) fn plan(
    segments: &[SourceRange],
    overrides: &LayoutOverrides,
    shots: &[ShotAnalysis],
    geometry: Geometry,
    min_piece: Tick,
) -> Vec<ShotLayout> {
    let split: Vec<_> = segments
        .iter()
        .map(|segment| pieces::split(*segment, shots, min_piece))
        .collect();
    let pin = if overrides.pin_first_speaker {
        choice::first_speaker(split.iter().flatten().filter_map(|piece| piece.evidence))
    } else {
        None
    };
    let mut planned: Vec<ShotLayout> = Vec::new();
    for (segment, stretches) in split.iter().enumerate() {
        let forced = overrides.for_segment(segment);
        for piece in stretches {
            let Choice { layout, fallback } = choice::resolve(forced, piece.evidence);
            let slots = composition::compose(layout, piece.evidence, pin, geometry);
            if let Some(last) = planned.last_mut()
                && last.segment == segment
                && last.layout == layout
                && last.slots == slots
                && last.range.end() == piece.range.start()
                && let Ok(range) = SourceRange::new(last.range.start(), piece.range.end())
            {
                last.range = range;
                last.fallback |= fallback;
                continue;
            }
            planned.push(ShotLayout {
                segment,
                range: piece.range,
                layout,
                fallback,
                slots,
            });
        }
    }
    planned
}
