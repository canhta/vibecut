//! Applies clip drag gestures.

use crate::editor::timeline::{
    BTreeSet, Command, CreatedId, DragMode, DragOrigin, DragState, Editor, ItemId, Result,
    Rounding, TRACK_GAP, Tick, TimelineLayout, TrackId, TrackRow, edge_auto_scroll, egui,
};

impl Editor {
    pub(in crate::editor) fn begin_drag(&mut self, item: ItemId, mode: DragMode, ui: &egui::Ui) {
        if self.bookmark_drag.is_some() {
            self.finish_bookmark_drag();
        }
        if self.drag.is_some() {
            self.finish_drag();
        }
        if !self.selection.contains(&item) {
            self.select_single(item);
        }
        let Some(press_x) = ui
            .ctx()
            .input(|input| input.pointer.press_origin().map(|origin| origin.x))
        else {
            return;
        };
        self.session.end_merge();
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.error = Some(error.to_string());
            return;
        }
        if let Err(error) = self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.begin_gesture())
        {
            self.error = Some(error.to_string());
            return;
        }
        let mut origins = Vec::new();
        let mut source_range = None;
        let mut speed = time::Speed::UNITY;
        for track in self.session.project().data().scene().timeline.iter() {
            for candidate in &track.items {
                let wanted = if mode == DragMode::Move {
                    self.selection.contains(&candidate.id)
                } else {
                    candidate.id == item
                };
                if !wanted {
                    continue;
                }
                if let Ok(end) = candidate.end() {
                    origins.push(DragOrigin {
                        item: candidate.id,
                        track: track.id,
                        start: candidate.start,
                        end,
                    });
                    if candidate.id == item {
                        source_range = Some(candidate.source_range);
                        speed = candidate.retime.speed;
                    }
                }
            }
        }
        let Some(source_range) = source_range else {
            let _ = self.session.cancel_gesture();
            return;
        };
        self.drag = Some(DragState {
            item,
            mode,
            press_x,
            origins,
            source_range,
            speed,
            moved: false,
        });
    }
    /// Drive the active gesture from global pointer state. Release and update do not
    /// depend on the item or handle still being visible or hit-testable this frame.
    pub(in crate::editor::timeline) fn advance_drag(
        &mut self,
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
            let (_, max_scroll) = self.scroll_bounds(layout, self.session.project().duration());
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
            self.update_drag(pointer, layout.body.left(), bypass);
        }
        if !down {
            if let Some(pointer) = pointer
                && let Err(error) = self.drop_track(pointer, layout, rows, bypass)
            {
                self.error = Some(error.to_string());
                self.cancel_interaction();
                return;
            }
            self.finish_drag();
        }
    }
    pub(in crate::editor::timeline) fn drop_track(
        &mut self,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        bypass: bool,
    ) -> Result<()> {
        let Some(drag) = self.drag.clone() else {
            return Ok(());
        };
        if drag.mode != DragMode::Move || drag.origins.len() != 1 || !layout.body.contains(pointer)
        {
            return Ok(());
        }
        let Some(origin) = drag.origins.first() else {
            return Ok(());
        };
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
        let Some((target, placement)) = destination else {
            return Ok(());
        };
        if placement.is_none() && target == origin.track {
            return Ok(());
        }
        let raw = Tick::from_seconds_f64(
            f64::from(pointer.x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (delta, _) = self.snap_delta(raw, &drag.origins, bypass);
        let start = origin
            .start
            .checked_add(delta)
            .ok_or(engine::EditError::Invalid("time overflow"))?
            .max(Tick::ZERO);
        let target = if let Some(placement) = placement {
            let kind = self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .find(|t| t.id == origin.track)
                .ok_or(engine::EditError::NoTrack(origin.track))?
                .kind;
            let outcome = self.session.apply(Command::AddTrack {
                name: format!("{kind:?}").into(),
                kind,
                placement,
            })?;
            match outcome.created.first() {
                Some(CreatedId::Track(id)) => *id,
                _ => return Err(engine::EditError::Invalid("track allocation").into()),
            }
        } else {
            target
        };
        self.session.apply(Command::MoveItem {
            item: drag.item,
            track: target,
            start,
        })?;
        if let Some(drag) = &mut self.drag {
            drag.moved = true;
        }
        Ok(())
    }
    pub(in crate::editor) fn update_drag(
        &mut self,
        pointer: egui::Pos2,
        body_left: f32,
        bypass: bool,
    ) {
        let Some(mut drag) = self.drag.clone() else {
            return;
        };
        let result = match drag.mode {
            DragMode::Move => self.apply_drag_move(&drag, pointer.x, bypass),
            DragMode::TrimStart => {
                self.apply_drag_trim(&drag, pointer.x, body_left, bypass, DragMode::TrimStart)
            }
            DragMode::TrimEnd => {
                self.apply_drag_trim(&drag, pointer.x, body_left, bypass, DragMode::TrimEnd)
            }
        };
        let changed = match result {
            Ok(changed) => changed,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        };
        drag.moved |= changed;
        self.drag = Some(drag);
        if changed {
            self.sync_preview();
        }
    }
    pub(in crate::editor) fn apply_drag_move(
        &mut self,
        drag: &DragState,
        pointer_x: f32,
        bypass: bool,
    ) -> Result<bool> {
        let raw = Tick::from_seconds_f64(
            f64::from(pointer_x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (snapped, indicator) = self.snap_delta(raw, &drag.origins, bypass);
        let (low, high) = self.move_bounds(&drag.origins);
        let delta = snapped.clamp(low, high);
        self.snap_indicator = indicator.filter(|_| delta == snapped);
        // Always target the origin plus this delta. A return to the press point must
        // restore the gesture-entry positions rather than leave an earlier preview.
        let moves = drag
            .origins
            .iter()
            .map(|origin| engine::Move {
                item: origin.item,
                track: origin.track,
                start: origin.start + delta,
            })
            .collect();
        Ok(self.session.apply(Command::MoveItems { moves })?.changed)
    }
    pub(in crate::editor) fn apply_drag_trim(
        &mut self,
        drag: &DragState,
        pointer_x: f32,
        body_left: f32,
        bypass: bool,
        mode: DragMode,
    ) -> Result<bool> {
        let raw = Tick::from_seconds_f64(
            f64::from(self.scroll + (pointer_x - body_left) / self.zoom),
            Rounding::Nearest,
        )?;
        let exclude: BTreeSet<ItemId> = drag.origins.iter().map(|origin| origin.item).collect();
        let (snapped, indicator) = self.snap_tick(raw, &exclude, None, bypass);
        let (low, high) = self.trim_bounds(drag, mode);
        let at = snapped.clamp(low, high);
        self.snap_indicator = indicator.filter(|_| at == snapped);
        let command = match mode {
            DragMode::TrimStart => Command::TrimStart {
                item: drag.item,
                at,
            },
            DragMode::TrimEnd => Command::TrimEnd {
                item: drag.item,
                at,
            },
            DragMode::Move => return Ok(false),
        };
        Ok(self.session.apply(command)?.changed)
    }
    pub(in crate::editor) fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let mut edited = false;
        if self.session.gesture_active() {
            let outcome = if drag.moved {
                edited = true;
                self.session.commit_gesture()
            } else {
                self.session.cancel_gesture()
            };
            if let Err(error) = outcome {
                self.error = Some(error.to_string());
            }
        }
        if edited {
            self.sync_preview();
        }
        self.snap_indicator = None;
        self.prune_selection();
    }
    pub(in crate::editor) fn move_bounds(&self, origins: &[DragOrigin]) -> (Tick, Tick) {
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
        for track in self.session.project().data().scene().timeline.iter() {
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
    pub(in crate::editor) fn trim_bounds(&self, drag: &DragState, mode: DragMode) -> (Tick, Tick) {
        let minimum = self.frame_ticks();
        let origin = drag
            .origins
            .iter()
            .find(|origin| origin.item == drag.item)
            .copied();
        let Some(origin) = origin else {
            return (Tick::ZERO, Tick::ZERO);
        };
        let mut low = match mode {
            DragMode::TrimStart => {
                // Output time that the source before the trim point would play for.
                let before = drag
                    .speed
                    .output_duration(drag.source_range.start())
                    .unwrap_or(Tick::ZERO);
                (origin.start - before).max(Tick::ZERO)
            }
            DragMode::TrimEnd => origin.start + minimum,
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        let mut high = match mode {
            DragMode::TrimStart => origin.end - minimum,
            DragMode::TrimEnd => Tick(i64::MAX / 4),
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        for track in self.session.project().data().scene().timeline.iter() {
            if track.id != origin.track {
                continue;
            }
            for item in &track.items {
                if item.id == origin.item {
                    continue;
                }
                let Ok(end) = item.end() else {
                    continue;
                };
                match mode {
                    DragMode::TrimStart if end <= origin.start => low = low.max(end),
                    DragMode::TrimEnd if item.start >= origin.end => high = high.min(item.start),
                    _ => {}
                }
            }
        }
        if mode == DragMode::TrimEnd
            && let Some(asset) = self
                .session
                .project()
                .data()
                .assets
                .iter()
                .find(|asset| Some(asset.id()) == self.item_asset(drag.item))
            && let Some(duration) = asset.probe().duration
        {
            let after = drag
                .speed
                .output_duration(duration - drag.source_range.start())
                .unwrap_or(Tick::ZERO);
            high = high.min(origin.start + after);
        }
        if high < low {
            high = low;
        }
        (low, high)
    }
    pub(in crate::editor) fn item_asset(&self, item: ItemId) -> Option<media::AssetId> {
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item)
            .and_then(|candidate| candidate.source_asset)
    }
}
