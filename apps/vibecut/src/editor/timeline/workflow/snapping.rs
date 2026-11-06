//! Resolves timeline snap targets.

use crate::editor::timeline::{
    BTreeSet, BookmarkId, Context, DragOrigin, ItemId, SnapCandidate, SnapSource, State, Tick,
    snap_point,
};

impl State {
    /// Build the one ranked candidate set. Selected items and the bookmark being
    /// dragged are excluded so a gesture cannot snap to itself.
    pub(in crate::editor::timeline) fn snap_candidates(
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
        let outcome = crate::snapping::snap_delta(
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
