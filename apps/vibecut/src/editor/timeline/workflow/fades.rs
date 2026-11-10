//! Drives an audio clip's fade handles as one `audio.set_fade` undo step.
//!
//! The handle drag is relative to the press point, so grabbing a handle never
//! resets the fade it already holds: the pointer's horizontal travel grows or
//! shrinks the edge, and the engine clamps the result to the clip and the other
//! fade.

use crate::editor::timeline::{Action, ItemView, Rounding, State, Tick, egui, state::FadeDrag};

impl State {
    /// Start a fade drag from the pointer's press point over an audio handle.
    pub(in crate::editor::timeline) fn begin_fade_drag(
        &mut self,
        ui: &egui::Ui,
        item: &ItemView,
        edge: project::FadeEdge,
    ) {
        let Some(press_x) = ui
            .ctx()
            .input(|input| input.pointer.press_origin().map(|origin| origin.x))
        else {
            return;
        };
        let Some(fades) = item.fades else {
            return;
        };
        let fade = match edge {
            project::FadeEdge::In => fades.fade_in,
            project::FadeEdge::Out => fades.fade_out,
        };
        let other = match edge {
            project::FadeEdge::In => fades.fade_out,
            project::FadeEdge::Out => fades.fade_in,
        };
        let clip = item.end - item.start;
        let room = clip
            .saturating_sub(other.window().min(clip))
            .max(Tick::ZERO);
        let base = fade.window().min(room);
        self.push(Action::BeginGesture { select: None });
        self.fade_drag = Some(FadeDrag {
            item: item.id,
            edge,
            curve: fade.curve,
            base,
            press_x,
            room,
            duration: base,
        });
    }

    /// Stage one `audio.set_fade` per changed pointer position; commit on release.
    pub(in crate::editor::timeline) fn advance_fade_drag(&mut self, ui: &egui::Ui) {
        let Some(fade) = self.fade_drag else {
            return;
        };
        let (pointer, down) = ui
            .ctx()
            .input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        if let Some(pointer) = pointer {
            let travel = match fade.edge {
                project::FadeEdge::In => pointer.x - fade.press_x,
                project::FadeEdge::Out => fade.press_x - pointer.x,
            };
            let delta =
                Tick::from_seconds_f64(f64::from(travel) / f64::from(self.zoom), Rounding::Nearest)
                    .unwrap_or(Tick::ZERO);
            let duration = Tick(fade.base.0.saturating_add(delta.0))
                .max(Tick::ZERO)
                .min(fade.room);
            if duration != fade.duration {
                let commands = if duration == fade.base {
                    // An empty candidate restores the exact split offset too.
                    Vec::new()
                } else {
                    vec![engine::Command::SetFade {
                        item: fade.item,
                        edge: fade.edge,
                        duration,
                        curve: fade.curve,
                    }]
                };
                self.push(Action::Preview(Box::new(
                    engine::Command::TimelinePreview { commands },
                )));
                if let Some(state) = self.fade_drag.as_mut() {
                    state.duration = duration;
                }
            }
        }
        if !down && self.fade_drag.take().is_some() {
            self.push(Action::EndGesture);
        }
    }
}
