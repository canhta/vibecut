//! Drives the audio clip's draggable volume line as one 0.1 dB undo step.

use crate::editor::timeline::{
    Action, Context, ItemId, ItemView, State, Tick, TimelineLayout, audio, egui,
};

impl State {
    /// Start a volume drag from the pointer's press point over an audio clip.
    pub(in crate::editor::timeline) fn begin_volume_drag(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        inner: egui::Rect,
    ) {
        let Some(origin) = ui.input(|input| input.pointer.press_origin()) else {
            return;
        };
        let Some(time) = self.volume_time_at(cx, layout, item, origin.x) else {
            return;
        };
        let keyed = volume_channel(cx, item.id).is_some_and(|channel| !channel.keys().is_empty());
        self.push(Action::BeginGesture { select: None });
        self.volume_drag = Some(crate::editor::timeline::state::VolumeDrag {
            item: item.id,
            time,
            keyed,
            db: db_at_y(inner, origin.y),
            top: inner.top(),
            height: inner.height(),
        });
    }

    /// Apply one preview command while the pointer moves; commit on release.
    pub(in crate::editor::timeline) fn advance_volume_drag(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        _layout: &TimelineLayout,
    ) {
        let Some(volume) = self.volume_drag else {
            return;
        };
        let (pointer, down) = ui
            .ctx()
            .input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        if let Some(pointer) = pointer {
            let db = db_at_y(
                egui::Rect::from_min_size(
                    egui::pos2(0.0, volume.top),
                    egui::vec2(1.0, volume.height),
                ),
                pointer.y,
            );
            if (db - volume.db).abs() > f32::EPSILON {
                let Some(path) = volume_path() else {
                    return;
                };
                let command = if volume.keyed {
                    engine::Command::AddKeyframe {
                        item: volume.item,
                        path,
                        component: None,
                        key: project::Keyframe::new(
                            volume.time,
                            project::ParamValue::Scalar(f64::from(db)),
                        ),
                    }
                } else {
                    engine::Command::SetItemField {
                        item: volume.item,
                        patch: engine::ItemParamPatch::AudioGain(db),
                    }
                };
                self.push(Action::Preview(Box::new(command)));
                if let Some(state) = &mut self.volume_drag {
                    state.db = db;
                }
            }
        }
        if !down && self.volume_drag.take().is_some() {
            self.push(Action::EndGesture);
        }
        let _ = cx;
    }

    fn volume_time_at(
        &self,
        cx: &Context<'_>,
        layout: &TimelineLayout,
        item: &ItemView,
        x: f32,
    ) -> Option<Tick> {
        let at = super::frame_snap(cx, self.tick_at(x, layout.body.left()));
        at.checked_sub(item.start)
            .filter(|time| *time >= Tick::ZERO && *time <= item.end - item.start)
    }
}

fn volume_path() -> Option<project::AnimationPath> {
    project::AnimationPath::new("volume").ok()
}
fn volume_channel<'a>(cx: &'a Context<'_>, item: ItemId) -> Option<&'a project::Channel> {
    cx.session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| candidate.animations.get(&volume_path()?))
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
}
fn db_at_y(inner: egui::Rect, y: f32) -> f32 {
    audio::quantize_drag(audio::db_at_fraction(
        ((y - inner.top()) / inner.height().max(1.0)).clamp(0.0, 1.0),
    ))
}
