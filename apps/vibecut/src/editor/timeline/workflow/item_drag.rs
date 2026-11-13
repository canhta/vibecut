//! Applies clip drag gestures through the group move/trim preview contract.

use crate::editor::timeline::{
    Action, BTreeSet, Context, DragMode, DragOrigin, DragState, DropZone, Gesture, ItemId, Result,
    Rounding, State, TRACK_GAP, Tick, TimelineLayout, TrackId, TrackRow, edge_auto_scroll, egui,
};

impl State {
    /// Grab a clip: the gesture opens in the frame, the geometry stays here.
    pub(in crate::editor::timeline) fn begin_drag(
        &mut self,
        cx: &Context<'_>,
        item: ItemId,
        mode: DragMode,
        ui: &egui::Ui,
    ) {
        if self.bookmark_drag.is_some() {
            self.finish_bookmark_drag();
        }
        if self.drag.is_some() {
            self.finish_drag();
        }
        let Some(press_x) = ui
            .ctx()
            .input(|input| input.pointer.press_origin().map(|origin| origin.x))
        else {
            return;
        };
        // Grabbing an unselected clip selects it alone first; a selected clip
        // drags or resizes the whole selection.
        let select = (!cx.selection().contains(&item)).then_some(item);
        let selection: BTreeSet<ItemId> = if select.is_some() {
            BTreeSet::from([item])
        } else {
            cx.selection().clone()
        };
        let mut origins = Vec::new();
        let mut source_range = None;
        let mut speed = time::Speed::UNITY;
        for track in cx.session.project().data().scene().timeline.iter() {
            for candidate in &track.items {
                if !selection.contains(&candidate.id) {
                    continue;
                }
                if let Ok(end) = candidate.end() {
                    origins.push(DragOrigin {
                        item: candidate.id,
                        track: track.id,
                        start: candidate.start,
                        end,
                        source_range: candidate.source_range,
                        speed: candidate.retime.speed,
                    });
                    if candidate.id == item {
                        source_range = Some(candidate.source_range);
                        speed = candidate.retime.speed;
                    }
                }
            }
        }
        let Some(source_range) = source_range else {
            return;
        };
        self.drag = Some(DragState {
            item,
            mode,
            press_x,
            delta: Tick::ZERO,
            origins,
            source_range,
            speed,
        });
        self.push(Action::BeginPreview { select });
    }
    /// Drive the active gesture from global pointer state. Release and update do not
    /// depend on the item or handle still being visible or hit-testable this frame.
    pub(in crate::editor::timeline) fn advance_drag(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
    ) {
        if self.drag.is_none() {
            return;
        }
        let (pointer, hover, bypass, down) = ui.ctx().input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.hover_pos(),
                input.modifiers.shift,
                input.pointer.primary_down(),
            )
        });
        if let Some(pointer) = pointer {
            let (_, max_scroll) = self.scroll_bounds(layout, cx.session.project().duration());
            let step = if down {
                edge_auto_scroll(hover, layout.body, self.scroll, max_scroll)
            } else {
                0.0
            };
            if step != 0.0 {
                let before = self.scroll;
                self.scroll = (self.scroll + step / self.zoom).clamp(0.0, max_scroll);
                let step = (self.scroll - before) * self.zoom;
                if let Some(drag) = &mut self.drag
                    && drag.mode == DragMode::Move
                {
                    // Keep the grabbed content under the pointer while the
                    // viewport advances; trim already reads the scroll offset.
                    drag.press_x -= step;
                }
            }
            self.update_drag(cx, pointer, layout.body.left(), bypass);
            self.drop_zone = self.drag.as_ref().and_then(|drag| {
                (drag.mode == DragMode::Move && down && layout.body.contains(pointer))
                    .then(|| self.drop_destination(pointer, layout, rows))
                    .flatten()
                    .and_then(|(target, placement)| self.zone_for(drag, target, placement))
            });
        }
        if !down {
            if let Some(pointer) = pointer
                && let Err(error) = self.drop_group(pointer, layout, rows)
            {
                self.fail(&error);
                self.push(Action::CancelInteraction);
                return;
            }
            self.finish_drag();
        }
    }
    /// A release over another track moves the clip there, allocating a track
    /// when the pointer is between lanes.
    pub(in crate::editor::timeline) fn drop_group(
        &mut self,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
    ) -> Result<()> {
        let Some(drag) = self.drag.clone() else {
            return Ok(());
        };
        if drag.mode != DragMode::Move || !layout.body.contains(pointer) {
            return Ok(());
        }
        let Some((target, placement)) = self.drop_destination(pointer, layout, rows) else {
            return Ok(());
        };
        let items: Vec<ItemId> = drag.origins.iter().map(|origin| origin.item).collect();
        let sources: BTreeSet<TrackId> = drag.origins.iter().map(|origin| origin.track).collect();
        let placement = match placement {
            Some(engine::TrackPlacement::Above { track }) => {
                Some(engine::Placement::Above { track })
            }
            Some(engine::TrackPlacement::Below { track }) => {
                Some(engine::Placement::Below { track })
            }
            Some(engine::TrackPlacement::Auto) => Some(engine::Placement::Auto),
            // A row drop can hold one source track; a group spanning tracks
            // keeps its relative rows and only moves horizontally.
            None if sources.len() == 1 && !sources.contains(&target) => {
                Some(engine::Placement::Track { track: target })
            }
            None => None,
        };
        let Some(placement) = placement else {
            return Ok(());
        };
        self.push(Action::PreviewGesture(Box::new(Gesture::Move {
            items,
            delta: drag.delta,
            placement: Some(placement),
        })));
        Ok(())
    }
    fn update_drag(&mut self, cx: &Context<'_>, pointer: egui::Pos2, body_left: f32, bypass: bool) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let result = match drag.mode {
            DragMode::Move => self.drag_move(cx, &drag, pointer.x, bypass),
            DragMode::TrimStart | DragMode::TrimEnd => {
                self.drag_trim(cx, &drag, pointer.x, body_left, bypass, drag.mode)
            }
        };
        if let Err(error) = result {
            self.fail(&error);
        }
    }
    fn drag_move(
        &mut self,
        cx: &Context<'_>,
        drag: &DragState,
        pointer_x: f32,
        bypass: bool,
    ) -> Result<()> {
        let raw = Tick::from_seconds_f64(
            f64::from(pointer_x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (snapped, indicator) = self.snap_delta(cx, raw, &drag.origins, bypass);
        let (low, high) = move_bounds(cx, &drag.origins);
        let delta = snapped.clamp(low, high);
        self.snap_indicator = indicator.filter(|_| delta == snapped);
        if let Some(state) = &mut self.drag {
            state.delta = delta;
        }
        let items = drag.origins.iter().map(|origin| origin.item).collect();
        self.push(Action::PreviewGesture(Box::new(Gesture::Move {
            items,
            delta,
            placement: None,
        })));
        Ok(())
    }
    fn drag_trim(
        &mut self,
        cx: &Context<'_>,
        drag: &DragState,
        pointer_x: f32,
        body_left: f32,
        bypass: bool,
        mode: DragMode,
    ) -> Result<()> {
        let raw = Tick::from_seconds_f64(
            f64::from(self.scroll + (pointer_x - body_left) / self.zoom),
            Rounding::Nearest,
        )?;
        let exclude: BTreeSet<ItemId> = drag.origins.iter().map(|origin| origin.item).collect();
        let (snapped, indicator) = self.snap_tick(cx, raw, &exclude, None, bypass);
        let Some(grabbed) = drag.origins.iter().find(|origin| origin.item == drag.item) else {
            return Ok(());
        };
        let edge = match mode {
            DragMode::TrimStart => grabbed.start,
            DragMode::TrimEnd => grabbed.end,
            DragMode::Move => return Ok(()),
        };
        let wanted = snapped - edge;
        let (low, high) = trim_bounds(cx, drag, mode);
        let delta = wanted.clamp(low, high);
        self.snap_indicator = indicator.filter(|_| delta == wanted);
        if let Some(state) = &mut self.drag {
            state.delta = delta;
        }
        let items = drag.origins.iter().map(|origin| origin.item).collect();
        self.push(Action::PreviewGesture(Box::new(Gesture::Trim {
            items,
            edge: match mode {
                DragMode::TrimStart => engine::TrimEdge::Start,
                DragMode::TrimEnd => engine::TrimEdge::End,
                DragMode::Move => return Ok(()),
            },
            delta,
        })));
        Ok(())
    }
    /// The pointer was released: ask the frame to publish the gesture.
    pub(in crate::editor::timeline) fn finish_drag(&mut self) {
        if self.drag.take().is_some() {
            self.push(Action::EndPreview);
        }
        self.drop_zone = None;
        self.snap_indicator = None;
    }
    /// Resolve the row under the pointer and whether it lands on a track or
    /// between lanes. Insertions allocate one destination per source track.
    fn drop_destination(
        &self,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
    ) -> Option<(TrackId, Option<engine::TrackPlacement>)> {
        let mut destination = None;
        for row in rows {
            let top = layout.body.top() + row.top - self.vertical_scroll;
            let bottom = top + row.height;
            if pointer.y < top + 3.0 {
                destination = Some((
                    row.id,
                    Some(engine::TrackPlacement::Above { track: row.id }),
                ));
                break;
            }
            if pointer.y <= bottom - 3.0 {
                destination = Some((row.id, None));
                break;
            }
            destination = Some((
                row.id,
                Some(engine::TrackPlacement::Below { track: row.id }),
            ));
            if pointer.y <= bottom + TRACK_GAP / 2.0 {
                break;
            }
        }
        destination
    }
    /// The landing hint to paint, or `None` when the drop would change nothing.
    fn zone_for(
        &self,
        drag: &DragState,
        target: TrackId,
        placement: Option<engine::TrackPlacement>,
    ) -> Option<DropZone> {
        let sources: BTreeSet<TrackId> = drag.origins.iter().map(|origin| origin.track).collect();
        match placement {
            Some(engine::TrackPlacement::Above { track }) => {
                Some(DropZone::Insert { track, above: true })
            }
            Some(engine::TrackPlacement::Below { track }) => Some(DropZone::Insert {
                track,
                above: false,
            }),
            Some(engine::TrackPlacement::Auto) => Some(DropZone::Track(target)),
            None if sources.len() == 1 && !sources.contains(&target) => {
                Some(DropZone::Track(target))
            }
            None => None,
        }
    }
}

fn move_bounds(cx: &Context<'_>, origins: &[DragOrigin]) -> (Tick, Tick) {
    let tracks: BTreeSet<TrackId> = origins.iter().map(|origin| origin.track).collect();
    let selected: BTreeSet<ItemId> = origins.iter().map(|origin| origin.item).collect();
    let earliest = origins
        .iter()
        .map(|origin| origin.start)
        .min()
        .unwrap_or(Tick::ZERO);
    // One shared delta keeps the selected group rigid at the timeline zero edge.
    let mut low = Tick(earliest.0.saturating_neg());
    let mut high = Tick(i64::MAX / 4);
    for track in cx.session.project().data().scene().timeline.iter() {
        if !tracks.contains(&track.id) {
            continue;
        }
        let track_origins: Vec<&DragOrigin> = origins
            .iter()
            .filter(|origin| origin.track == track.id)
            .collect();
        for item in &track.items {
            if selected.contains(&item.id) {
                continue;
            }
            let Ok(end) = item.end() else {
                continue;
            };
            let previous_end = track_origins
                .iter()
                .filter(|origin| origin.end <= item.start)
                .map(|origin| origin.end)
                .max();
            if let Some(previous_end) = previous_end {
                high = high.min(item.start - previous_end);
            }
            let next_start = track_origins
                .iter()
                .filter(|origin| origin.start >= end)
                .map(|origin| origin.start)
                .min();
            if let Some(next_start) = next_start {
                low = low.max(end - next_start);
            }
        }
    }
    (low, high)
}

/// Delta bounds shared by every member: their neighbour and source limits must
/// all admit the same edge movement.
fn trim_bounds(cx: &Context<'_>, drag: &DragState, mode: DragMode) -> (Tick, Tick) {
    let minimum = cx.frame_ticks();
    let selected: BTreeSet<ItemId> = drag.origins.iter().map(|origin| origin.item).collect();
    let mut low = Tick(i64::MIN / 4);
    let mut high = Tick(i64::MAX / 4);
    for origin in &drag.origins {
        let (member_low, member_high) = match mode {
            DragMode::TrimStart => {
                // Output time the source before the trim point would play for.
                let before = origin
                    .speed
                    .output_duration(origin.source_range.start())
                    .unwrap_or(Tick::ZERO);
                let mut start_low = (origin.start - before).max(Tick::ZERO);
                let start_high = origin.end - minimum;
                for track in cx.session.project().data().scene().timeline.iter() {
                    if track.id != origin.track {
                        continue;
                    }
                    for item in &track.items {
                        if selected.contains(&item.id) {
                            continue;
                        }
                        if let Ok(end) = item.end()
                            && end <= origin.start
                        {
                            start_low = start_low.max(end);
                        }
                    }
                }
                (start_low - origin.start, start_high - origin.start)
            }
            DragMode::TrimEnd => {
                let start_low = origin.start + minimum;
                let mut end_high = Tick(i64::MAX / 4);
                for track in cx.session.project().data().scene().timeline.iter() {
                    if track.id != origin.track {
                        continue;
                    }
                    for item in &track.items {
                        if selected.contains(&item.id) {
                            continue;
                        }
                        if item.start >= origin.end {
                            end_high = end_high.min(item.start);
                        }
                    }
                }
                if origin.item == drag.item
                    && let Some(asset) = cx
                        .session
                        .project()
                        .data()
                        .assets
                        .iter()
                        .find(|asset| Some(asset.id()) == item_asset(cx, drag.item))
                    && let Some(duration) = asset.probe().duration
                {
                    let after = drag
                        .speed
                        .output_duration(duration - drag.source_range.start())
                        .unwrap_or(Tick::ZERO);
                    end_high = end_high.min(origin.start + after);
                }
                (start_low - origin.end, end_high - origin.end)
            }
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        low = low.max(member_low);
        high = high.min(member_high);
    }
    if high < low {
        high = low;
    }
    (low, high)
}

fn item_asset(cx: &Context<'_>, item: ItemId) -> Option<media::AssetId> {
    cx.session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| candidate.source_asset)
}
