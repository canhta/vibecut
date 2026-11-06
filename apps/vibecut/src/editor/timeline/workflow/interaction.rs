//! Applies bookmark and playhead gestures.

use crate::editor::timeline::{
    Action, BTreeSet, BookmarkDrag, BookmarkId, Command, Context, Id, Rect, Rounding, State, Tick,
    TimelineLayout, egui, frame_snap,
};

impl State {
    pub(in crate::editor::timeline) fn begin_bookmark_drag(
        &mut self,
        cx: &Context<'_>,
        bookmark: BookmarkId,
        ui: &egui::Ui,
    ) {
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
        let Some(start) = cx
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
        self.bookmark_drag = Some(BookmarkDrag {
            bookmark,
            press_x,
            start,
        });
        self.push(Action::BeginGesture { select: None });
    }
    /// Preview a bookmark move through the engine gesture. Snapping reads the
    /// current viewport scale and excludes the dragged bookmark's own edges.
    pub(in crate::editor::timeline) fn advance_bookmark_drag(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
    ) {
        let Some(drag) = self.bookmark_drag else {
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
            let (snapped, indicator) =
                self.snap_tick(cx, raw, &exclude, Some(drag.bookmark), bypass);
            let at = snapped.clamp(Tick::ZERO, cx.session.project().duration());
            self.snap_indicator = indicator.filter(|_| at == snapped);
            self.push(Action::Preview(Box::new(Command::MoveBookmark {
                bookmark: drag.bookmark,
                start: at,
            })));
        }
        if !down {
            self.finish_bookmark_drag();
        }
    }
    pub(in crate::editor::timeline) fn finish_bookmark_drag(&mut self) {
        if self.bookmark_drag.take().is_some() {
            self.push(Action::EndGesture);
        }
        self.snap_indicator = None;
    }
    pub(in crate::editor::timeline) fn playhead_interaction(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let x = self.x_at(cx.playhead, layout.body.left());
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
            let at = frame_snap(cx, self.tick_at(pointer.x, layout.body.left()));
            self.push(Action::Seek(at));
        }
    }
}
