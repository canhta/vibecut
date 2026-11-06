//! Applies bookmark gestures.

use crate::editor::timeline::{
    BTreeSet, BookmarkDrag, BookmarkId, Command, Editor, Id, Rect, Rounding, Tick, TimelineLayout,
    egui,
};

impl Editor {
    pub(in crate::editor) fn begin_bookmark_drag(&mut self, bookmark: BookmarkId, ui: &egui::Ui) {
        if self.drag.is_some() {
            self.finish_drag();
        }
        if self.bookmark_drag.is_some() {
            self.finish_bookmark_drag();
        }
        let Some(press_x) = ui
            .ctx()
            .input(|input| input.pointer.press_origin().map(|origin| origin.x))
        else {
            return;
        };
        let Some(start) = self
            .session
            .project()
            .data()
            .scene()
            .bookmarks
            .iter()
            .find(|candidate| candidate.id == bookmark)
            .map(|candidate| candidate.start)
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
        if let Err(error) = self.session.begin_gesture() {
            self.error = Some(error.to_string());
            return;
        }
        self.bookmark_drag = Some(BookmarkDrag {
            bookmark,
            press_x,
            start,
            moved: false,
        });
    }
    /// Preview a bookmark move through the engine gesture. Snapping reads the
    /// current viewport scale and excludes the dragged bookmark's own edges.
    pub(in crate::editor) fn advance_bookmark_drag(&mut self, ui: &egui::Ui) {
        let Some(mut drag) = self.bookmark_drag else {
            return;
        };
        let (pointer, bypass, down) = ui.ctx().input(|input| {
            (
                input.pointer.interact_pos(),
                input.modifiers.shift,
                input.pointer.primary_down(),
            )
        });
        if let Some(pointer) = pointer {
            let raw = Tick::from_seconds_f64(
                drag.start.seconds() + f64::from(pointer.x - drag.press_x) / f64::from(self.zoom),
                Rounding::Nearest,
            )
            .unwrap_or(drag.start);
            let exclude = BTreeSet::new();
            let (snapped, indicator) = self.snap_tick(raw, &exclude, Some(drag.bookmark), bypass);
            let at = snapped.clamp(Tick::ZERO, self.session.project().duration());
            self.snap_indicator = indicator.filter(|_| at == snapped);
            match self.session.apply(Command::MoveBookmark {
                bookmark: drag.bookmark,
                start: at,
            }) {
                Ok(outcome) => {
                    if outcome.changed {
                        drag.moved = true;
                        self.sync_preview();
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        self.bookmark_drag = Some(drag);
        if !down {
            self.finish_bookmark_drag();
        }
    }
    pub(in crate::editor) fn finish_bookmark_drag(&mut self) {
        let Some(drag) = self.bookmark_drag.take() else {
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
    }
    pub(in crate::editor) fn cancel_interaction(&mut self) {
        if self.media_marquee.take().is_some() {
            self.media_selection.clear();
            self.media_anchor = None;
            self.session.end_merge();
            return;
        }
        if let Some(drag) = self.bookmark_drag.take() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            if drag.moved {
                self.sync_preview();
            }
            self.snap_indicator = None;
            return;
        }
        if let Some(scrub) = self.scrub.take() {
            let changed = scrub.last != scrub.start;
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            self.property_drafts.remove(&scrub.target.draft_key());
            self.property_focus = None;
            if changed {
                self.sync_preview();
            }
            return;
        }
        let track_drag = self.track_drag.take().is_some();
        if self.track_rename.take().is_some() || track_drag {
            self.track_rename_focus = false;
            return;
        }
        if self.effect_drag.take().is_some() {
            return;
        }
        if self.effect_reorder.take().is_some() {
            return;
        }
        if let Some(drag) = self.drag.take() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            if drag.moved {
                self.sync_preview();
            }
            self.snap_indicator = None;
        } else if let Some((_, before, _)) = self.marquee.take() {
            self.selection = before;
            self.selection_changed();
        } else {
            self.selection_scopes.clear_specific(&mut self.selection);
            if let Err(e) = self.session.set_selection(self.selection.clone()) {
                self.error = Some(e.to_string());
            }
            if !self.media_selection.is_empty() {
                self.media_selection.clear();
                self.media_anchor = None;
                self.session.end_merge();
            }
        }
    }
    pub(in crate::editor::timeline) fn timeline_playhead_interaction(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 6.0 || x > layout.body.right() + 6.0 {
            return;
        }
        // Keep the playhead draggable from the ruler so item automation IDs stay
        // clickable when the line crosses a clip.
        let rect = Rect::from_min_max(
            egui::pos2(x - 6.0, layout.ruler.top()),
            egui::pos2(x + 6.0, layout.ruler.bottom()),
        );
        let response = ui.interact(
            rect,
            Id::unique("timeline.playhead"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.playhead", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = self.frame_snap(self.tick_at(pointer.x, layout.body.left()));
            self.seek_to(at);
        }
    }
}
