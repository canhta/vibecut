//! Timeline snapping: candidate building, ranking and the screen-pixel threshold.
//!
//! The timeline owns this policy; engine commands receive only the resulting tick
//! position. Candidates are ranked by the smallest pixel distance, then by the
//! stable source order below. That order is deliberately independent of scene
//! iteration order so a tie cannot change between frames or project reloads. The
//! preview canvas ranks with the same [`closest`] rule and its own measured
//! threshold.
use crate::editor::context::closest;
use crate::editor::timeline::{
    BTreeSet, BookmarkId, Context, DragOrigin, ItemId, Rounding, State, Tick,
};
use time::TICKS_PER_SECOND;

/// The reference timeline threshold in logical screen pixels.
const SNAP_PIXELS: f32 = 10.0;

/// Stable identity for a snap target. Variant order is the documented tie-break:
/// playhead, bookmarks, item edges, then item keyframes; equal variants compare
/// their owner IDs and keyframe time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SnapSource {
    Playhead,
    BookmarkStart(BookmarkId),
    BookmarkEnd(BookmarkId),
    ItemStart(ItemId),
    ItemEnd(ItemId),
    Keyframe { item: ItemId, time: Tick },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SnapCandidate {
    at: Tick,
    source: SnapSource,
}

impl SnapCandidate {
    const fn new(at: Tick, source: SnapSource) -> Self {
        Self { at, source }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SnapMatch {
    at: Tick,
    source: SnapSource,
    /// Absolute distance in logical pixels at the zoom used for this query.
    pixels: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SnapOutcome {
    at: Tick,
    target: Option<SnapMatch>,
}

impl SnapOutcome {
    const fn raw(at: Tick) -> Self {
        Self { at, target: None }
    }
}

/// Convert the 10 px threshold to ticks at the current viewport scale.
///
/// Hostile or non-positive zoom cannot form a useful threshold and returns zero,
/// so only an exact candidate can match.
fn threshold_ticks(zoom: f32) -> Tick {
    if !zoom.is_finite() || zoom <= 0.0 {
        return Tick::ZERO;
    }
    Tick::from_seconds_f64(f64::from(SNAP_PIXELS) / f64::from(zoom), Rounding::Nearest)
        .unwrap_or(Tick::ZERO)
}

/// Snap one raw position. Disabled or bypassed snapping returns the raw tick with
/// no indicator target.
fn snap_point(
    raw: Tick,
    candidates: &[SnapCandidate],
    enabled: bool,
    bypass: bool,
    zoom: f32,
) -> SnapOutcome {
    if !enabled || bypass {
        return SnapOutcome::raw(raw);
    }
    match nearest(raw, candidates, zoom) {
        Some(target) => SnapOutcome {
            at: target.at,
            target: Some(target),
        },
        None => SnapOutcome::raw(raw),
    }
}

/// Snap a move delta using every selected item edge. The globally closest edge
/// wins, so a multi-selection stays rigid while one of its edges meets a target.
fn snap_delta(
    raw: Tick,
    edges: &[Tick],
    candidates: &[SnapCandidate],
    enabled: bool,
    bypass: bool,
    zoom: f32,
) -> SnapOutcome {
    if !enabled || bypass || edges.is_empty() {
        return SnapOutcome::raw(raw);
    }
    let mut best: Option<(SnapMatch, Tick)> = None;
    for edge in edges {
        let Some(current) = edge.checked_add(raw) else {
            continue;
        };
        let Some(target) = nearest(current, candidates, zoom) else {
            continue;
        };
        let better = best.is_none_or(|(previous, _)| {
            target.pixels < previous.pixels
                || (target.pixels == previous.pixels && target.source < previous.source)
        });
        if better {
            let Some(delta) = target.at.checked_sub(current) else {
                continue;
            };
            let Some(at) = raw.checked_add(delta) else {
                continue;
            };
            best = Some((target, at));
        }
    }
    match best {
        Some((target, at)) => SnapOutcome {
            at,
            target: Some(target),
        },
        None => SnapOutcome::raw(raw),
    }
}

fn nearest(raw: Tick, candidates: &[SnapCandidate], zoom: f32) -> Option<SnapMatch> {
    if !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let threshold = threshold_ticks(zoom);
    closest(candidates.iter().filter_map(|candidate| {
        let difference = candidate.at.checked_sub(raw)?;
        if difference.0.unsigned_abs() > threshold.0.unsigned_abs() {
            return None;
        }
        let pixels = difference.0.unsigned_abs() as f64 / TICKS_PER_SECOND as f64 * f64::from(zoom);
        Some((candidate.at, candidate.source, pixels))
    }))
    .map(|(at, source, pixels)| SnapMatch { at, source, pixels })
}

impl State {
    /// Build the one ranked candidate set. Selected items and the bookmark being
    /// dragged are excluded so a gesture cannot snap to itself.
    fn snap_candidates(
        &self,
        cx: &Context<'_>,
        exclude_items: &BTreeSet<ItemId>,
        exclude_bookmark: Option<BookmarkId>,
    ) -> Vec<SnapCandidate> {
        let project = cx.session.project();
        let data = project.data();
        let mut points = vec![SnapCandidate::new(cx.playhead, SnapSource::Playhead)];
        for bookmark in &data.scene().bookmarks {
            if Some(bookmark.id) == exclude_bookmark {
                continue;
            }
            points.push(SnapCandidate::new(
                bookmark.start,
                SnapSource::BookmarkStart(bookmark.id),
            ));
            if let Some(end) = bookmark.start.checked_add(bookmark.duration) {
                points.push(SnapCandidate::new(
                    end,
                    SnapSource::BookmarkEnd(bookmark.id),
                ));
            }
        }
        for item in data
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
        {
            if exclude_items.contains(&item.id) {
                continue;
            }
            points.push(SnapCandidate::new(
                item.start,
                SnapSource::ItemStart(item.id),
            ));
            if let Ok(end) = item.end() {
                points.push(SnapCandidate::new(end, SnapSource::ItemEnd(item.id)));
            }
            for (_, param) in item.animations.iter() {
                let mut push = |key: &project::Keyframe| {
                    let Some(at) = item.start.checked_add(key.time) else {
                        return;
                    };
                    if at < Tick::ZERO {
                        return;
                    }
                    points.push(SnapCandidate::new(
                        at,
                        SnapSource::Keyframe {
                            item: item.id,
                            time: key.time,
                        },
                    ));
                };
                match param {
                    project::Param::Value(channel) => {
                        for key in channel.keys() {
                            push(key);
                        }
                    }
                    project::Param::Color(channels) => {
                        for channel in channels.iter() {
                            for key in channel.keys() {
                                push(key);
                            }
                        }
                    }
                }
            }
        }
        points.sort_by(|a, b| a.at.cmp(&b.at).then(a.source.cmp(&b.source)));
        points.dedup();
        points
    }
    pub(in crate::editor::timeline) fn snap_delta(
        &self,
        cx: &Context<'_>,
        raw: Tick,
        origins: &[DragOrigin],
        bypass: bool,
    ) -> (Tick, Option<Tick>) {
        let exclude: BTreeSet<ItemId> = origins.iter().map(|origin| origin.item).collect();
        let candidates = self.snap_candidates(cx, &exclude, None);
        let edges: Vec<Tick> = origins
            .iter()
            .flat_map(|origin| [origin.start, origin.end])
            .collect();
        let outcome = snap_delta(
            raw,
            &edges,
            &candidates,
            cx.preferences.snapping,
            bypass,
            self.zoom,
        );
        (outcome.at, outcome.target.map(|target| target.at))
    }
    pub(in crate::editor::timeline) fn snap_tick(
        &self,
        cx: &Context<'_>,
        raw: Tick,
        exclude: &BTreeSet<ItemId>,
        exclude_bookmark: Option<BookmarkId>,
        bypass: bool,
    ) -> (Tick, Option<Tick>) {
        let candidates = self.snap_candidates(cx, exclude, exclude_bookmark);
        let outcome = snap_point(raw, &candidates, cx.preferences.snapping, bypass, self.zoom);
        (outcome.at, outcome.target.map(|target| target.at))
    }
}

/// Round a pointer time to the nearest frame of the canvas rate.
pub(in crate::editor::timeline) fn frame_snap(cx: &Context<'_>, at: Tick) -> Tick {
    cx.session
        .project()
        .data()
        .canvas
        .fps
        .snap_nearest(at)
        .unwrap_or(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SECOND: Tick = Tick(TICKS_PER_SECOND);

    fn candidate(at: Tick, source: SnapSource) -> SnapCandidate {
        SnapCandidate::new(at, source)
    }

    #[test]
    fn threshold_converts_ten_pixels_at_tenth_reference_one_and_ten_times_zoom() {
        for (zoom, expected) in [
            (4.0_f32, Tick(635_040_000_000)),
            (40.0, Tick(63_504_000_000)),
            (400.0, Tick(6_350_400_000)),
        ] {
            assert_eq!(threshold_ticks(zoom), expected, "zoom {zoom}");
        }
        assert_eq!(threshold_ticks(0.0), Tick::ZERO);
        assert_eq!(threshold_ticks(-1.0), Tick::ZERO);
        assert_eq!(threshold_ticks(f32::NAN), Tick::ZERO);
        assert_eq!(threshold_ticks(f32::INFINITY), Tick::ZERO);
        // At a hostile but finite zoom the threshold rounds to a bounded tick.
        assert!(threshold_ticks(f32::MAX).0 >= 0);
        assert!(threshold_ticks(1.0e-9).0 <= Tick(i64::MAX).0);
    }

    #[test]
    fn nearest_candidate_uses_pixels_then_stable_source_order() {
        let raw = SECOND;
        let candidates = [
            candidate(Tick(SECOND.0 - 10), SnapSource::ItemEnd(ItemId(2))),
            candidate(Tick(SECOND.0 + 10), SnapSource::ItemEnd(ItemId(1))),
            candidate(Tick(SECOND.0 - 30), SnapSource::ItemStart(ItemId(1))),
        ];
        let outcome = snap_point(raw, &candidates, true, false, 40.0);
        assert_eq!(
            outcome.target.unwrap().source,
            SnapSource::ItemEnd(ItemId(1)),
            "equal distances choose the lower stable source id"
        );
        assert_eq!(outcome.at, Tick(SECOND.0 + 10));
        let farther = snap_point(
            Tick(SECOND.0 + 20),
            &[candidate(
                Tick(SECOND.0 - 10),
                SnapSource::ItemEnd(ItemId(1)),
            )],
            true,
            false,
            40.0,
        );
        assert_eq!(farther.at, Tick(SECOND.0 - 10));
    }

    #[test]
    fn disabled_or_bypassed_snapping_returns_the_raw_position_without_an_indicator() {
        let raw = Tick(123_456);
        let candidates = [candidate(raw, SnapSource::Playhead)];
        for outcome in [
            snap_point(raw, &candidates, false, false, 40.0),
            snap_point(raw, &candidates, true, true, 40.0),
        ] {
            assert_eq!(outcome, SnapOutcome::raw(raw));
        }
        let delta = snap_delta(
            Tick(50),
            &[Tick::ZERO],
            &[candidate(Tick(60), SnapSource::Playhead)],
            false,
            false,
            40.0,
        );
        assert_eq!(delta, SnapOutcome::raw(Tick(50)));
    }

    #[test]
    fn multi_edge_delta_uses_the_closest_edge_and_keeps_the_group_rigid() {
        let candidates = [candidate(Tick(100), SnapSource::ItemStart(ItemId(9)))];
        let outcome = snap_delta(
            Tick(45),
            &[Tick(50), Tick(150)],
            &candidates,
            true,
            false,
            40.0,
        );
        assert_eq!(outcome.at, Tick(50));
        assert_eq!(outcome.target.unwrap().at, Tick(100));
        let edge = Tick(50).checked_add(outcome.at).unwrap();
        assert_eq!(edge, Tick(100), "the selected start edge lands on target");
    }

    #[test]
    fn closest_ranks_by_pixel_distance_then_stable_source_order() {
        // Equal distances pick the lower source; a closer candidate always wins.
        let outcome = closest([
            (1_u8, SnapSource::ItemEnd(ItemId(2)), 4.0),
            (2, SnapSource::ItemEnd(ItemId(1)), 4.0),
            (3, SnapSource::Playhead, 6.0),
        ])
        .unwrap();
        assert_eq!(outcome, (2, SnapSource::ItemEnd(ItemId(1)), 4.0));
        let closer = closest([
            (1_u8, SnapSource::Playhead, 2.5),
            (2, SnapSource::ItemEnd(ItemId(1)), 4.0),
        ])
        .unwrap();
        assert_eq!(closer, (1, SnapSource::Playhead, 2.5));
        assert_eq!(closest(std::iter::empty::<(u8, SnapSource, f64)>()), None);
        // A non-finite distance never wins, so hostile geometry cannot snap.
        let finite = closest([
            (1_u8, SnapSource::Playhead, f64::INFINITY),
            (2, SnapSource::ItemStart(ItemId(3)), 1.0),
        ])
        .unwrap();
        assert_eq!(finite, (2, SnapSource::ItemStart(ItemId(3)), 1.0));
    }

    #[test]
    fn timeline_threshold_is_the_reference_ten_pixels() {
        assert_eq!(SNAP_PIXELS, 10.0);
    }

    proptest! {
        #[test]
        fn snapping_never_moves_beyond_the_tick_threshold(
            raw in -2_000_000_000i64..2_000_000_000,
            offset in -1_000_000i64..1_000_000,
            zoom in 0.001_f32..10_000.0,
        ) {
            let at = Tick(raw).checked_add(Tick(offset)).unwrap();
            let candidates = [candidate(at, SnapSource::Keyframe { item: ItemId(1), time: Tick(offset) })];
            let outcome = snap_point(Tick(raw), &candidates, true, false, zoom);
            let threshold = threshold_ticks(zoom);
            if let Some(target) = outcome.target {
                prop_assert_eq!(outcome.at, target.at);
                prop_assert!(outcome.at.checked_sub(Tick(raw)).unwrap().0.unsigned_abs()
                    <= threshold.0.unsigned_abs());
            } else {
                prop_assert_eq!(outcome.at, Tick(raw));
            }
        }

        #[test]
        fn delta_snapping_never_moves_beyond_the_tick_threshold(
            raw in -1_000_000i64..1_000_000,
            edge in -10_000_000i64..10_000_000,
            offset in -500_000i64..500_000,
            zoom in 0.01_f32..1_000.0,
        ) {
            let at = Tick(edge).checked_add(Tick(offset)).unwrap();
            let candidates = [candidate(at, SnapSource::ItemEnd(ItemId(4)))];
            let outcome = snap_delta(Tick(raw), &[Tick(edge)], &candidates, true, false, zoom);
            let threshold = threshold_ticks(zoom);
            if outcome.target.is_some() {
                prop_assert!(outcome.at.checked_sub(Tick(raw)).unwrap().0.unsigned_abs()
                    <= threshold.0.unsigned_abs());
            } else {
                prop_assert_eq!(outcome.at, Tick(raw));
            }
        }

        #[test]
        fn disabled_snapping_is_always_the_identity(
            raw in -2_000_000_000i64..2_000_000_000,
            at in -2_000_000_000i64..2_000_000_000,
            zoom in 0.001_f32..10_000.0,
        ) {
            let candidates = [candidate(Tick(at), SnapSource::Playhead)];
            prop_assert_eq!(
                snap_point(Tick(raw), &candidates, false, false, zoom),
                SnapOutcome::raw(Tick(raw))
            );
        }
    }
}
