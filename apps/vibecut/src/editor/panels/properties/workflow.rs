//! Applies the properties workflows.
mod appearance;
mod audio;
mod background;
mod canvas;
mod commands;
mod effects;
mod fonts;
pub(in crate::editor) mod lut;
mod masks;
mod number_edit;
mod speed;
mod transition;

pub(in crate::editor) use commands::{set_item_field, set_shared_field};
pub(in crate::editor) use effects::{add_effect_to_item, effect_refusal};
pub(super) use number_edit::transform_scrub;

use super::{Action, State};
use crate::editor::{Port, Result};

/// Execute the requests `show` returned, in order. Failures surface through
/// the editor's error line; later requests still run.
pub(in crate::editor) fn apply(state: &mut State, port: &mut Port<'_>, actions: Vec<Action>) {
    route(state, port, actions);
}

fn route(state: &mut State, port: &mut Port<'_>, actions: Vec<Action>) {
    for action in actions {
        // Advance only the captured scrub's own mutation. A different snapshot,
        // selection or clock must still be rejected by the next frame's guard.
        let refresh_scrub_snapshot = match &action {
            Action::SetField { item, field, .. } | Action::SetKeyframeAt { item, field, .. } => {
                state.scrub.as_ref().is_some_and(|scrub| {
                    scrub.item == *item
                        && matches!(&scrub.target, super::ScrubTarget::Item(active)
                        if *active == field.as_str()
                            && matches!(
                                *active,
                                "opacity"
                                    | "params.strokeWidth"
                                    | "params.cornerRadius"
                                    | "params.sides"
                                    | "params.points"
                                    | "params.depth"
                            ))
                }) && state
                    .opacity_edit
                    .as_ref()
                    .is_some_and(|(snapshot, bound_item, at)| {
                        let view = port.view();
                        bound_item == item
                            && *at == view.playhead
                            && crate::editor::context::single(view.selection()) == Some(*item)
                            && std::sync::Arc::ptr_eq(snapshot, view.session.project())
                    })
            }
            _ => false,
        };
        let result = match action {
            Action::Cache(action) => {
                port.request(crate::editor::context::Request::Cache(action));
                Ok(())
            }
            Action::Proxy(action) => {
                port.request(crate::editor::context::Request::Proxy(action));
                Ok(())
            }
            Action::SetClipAnimation {
                item,
                slot,
                preset,
                duration,
                snapshot,
            } => {
                commands::set_clip_animation(state, port, item, (slot, preset, duration), snapshot)
            }
            Action::ApplyTextPreset {
                item,
                preset,
                snapshot,
            } => commands::apply_text_preset(state, port, item, preset, snapshot),
            Action::SaveTextPreset { item, snapshot } => {
                commands::save_text_preset(state, port, item, snapshot)
            }
            Action::Activate(id) => {
                // Activation runs on the editor once this step returns the area state.
                port.request(crate::editor::context::Request::Action(id.to_owned()));
                Ok(())
            }
            Action::ImportLut { item, expected } => state.request_lut(port, item, expected),
            Action::SelectLut { item, id } => state.select_lut(port, item, id),
            Action::SetField { item, field, value } => {
                commands::set_item_field(port, item, &field, &value)
            }
            Action::SetShared { field, value } => commands::set_shared_field(port, &field, &value),
            Action::SetSpeed {
                item,
                speed,
                maintain_pitch,
            } => speed::set_speed(port, item, speed, maintain_pitch),
            Action::SetFade {
                item,
                edge,
                seconds,
            } => audio::set_fade(port, item, edge, seconds),
            Action::SetNoiseReduction { item, enabled } => {
                audio::set_noise_reduction(port, item, enabled)
            }
            Action::SetReverse { item, reverse } => speed::set_reverse(port, item, reverse),
            Action::SetTransitionDuration {
                transition,
                duration,
            } => transition::set_duration(port, transition, duration),
            Action::SetTransitionAlignment {
                transition,
                alignment,
            } => transition::set_alignment(port, transition, alignment),
            Action::RemoveTransition(transition) => transition::remove(port, transition),
            Action::SetCanvas {
                width,
                height,
                preset,
            } => canvas::set_canvas(port, width, height, preset),
            Action::SetFps { num, den } => canvas::set_canvas_fps(port, num, den),
            Action::SetBackground(color) => canvas::set_canvas_background(port, color),
            Action::SetClipBackground { items, background } => {
                background::set_clip_background(port, items, background)
            }
            Action::ResetTransform(item) => commands::reset_transform(port, item),
            Action::ResetBlend(item) => commands::reset_blend(port, item),
            Action::ToggleKeyframe { item, fields, time } => {
                commands::toggle_keyframes(port, item, &fields, time)
            }
            // The frame routes keyword intentions to the Captions panel before this match.
            Action::Keywords(_) => Ok(()),
            Action::SeekToKey(at) => {
                port.request(crate::editor::context::Request::Seek(at));
                Ok(())
            }
            Action::SetKeyframeAt {
                item,
                field,
                time,
                value,
            } => commands::set_keyframe_at(port, item, &field, time, value),
            Action::SetKeyColorPick(pick) => {
                port.request(crate::editor::context::Request::ArmKeyColor(pick));
                Ok(())
            }
            Action::ToggleChromaKey { item, enabled } => {
                state.toggle_chroma_key(port, item, enabled);
                Ok(())
            }
            Action::ResetKeyColor { item, effect } => state.reset_key_color(port, item, &effect),
            Action::SetSection { key, open } => port.set_section_open(key, open),
            Action::SetTab { kind, tab } => state.set_tab(port, kind, tab),
            Action::ScrubBegin => state.begin_gesture(port),
            Action::ScrubEnd { commit, changed } => state.end_gesture(port, commit, changed),
            Action::SetEffectParam {
                item,
                effect,
                key,
                value,
            } => effects::set_effect_param(port, item, &effect, &key, value),
            Action::SetEffectValues {
                item,
                effect,
                values,
            } => effects::set_effect_values(port, item, &effect, &values),
            Action::ApplyGradeToAll { item } => effects::apply_grade_to_all(port, item),
            Action::SaveGradePreset { item } => effects::save_grade_preset(port, item),
            Action::ToggleEffect {
                item,
                effect,
                enabled,
            } => {
                effects::toggle_effect(port, item, &effect, enabled);
                Ok(())
            }
            Action::RemoveEffect { item, effect } => {
                effects::remove_effect(port, item, &effect);
                Ok(())
            }
            Action::ReorderEffect {
                item,
                effect,
                target,
            } => {
                effects::reorder_effect(port, item, &effect, &target);
                Ok(())
            }
            Action::SyncPreview => {
                port.sync();
                Ok(())
            }
            Action::SetAnimatedParam { item, path, param } => port
                .apply_result(engine::Command::SetAnimatedParam { item, path, param })
                .map(|_| ()),
            Action::ColorPreview { command } => state.preview_color(port, command),
            Action::ColorCommit { command, recent } => state.commit_color(port, command, recent),
            Action::Font(action) => state.font_action(port, action),
            Action::Mask(action) => state.mask_action(port, action),
            Action::Error(message) => {
                port.fail(message);
                Ok(())
            }
        };
        let succeeded = result.is_ok();
        if let Err(error) = result {
            port.fail(error.to_string());
        }
        if refresh_scrub_snapshot
            && succeeded
            && let Some((snapshot, _, _)) = &mut state.opacity_edit
        {
            *snapshot = port.view().session.project().clone();
        }
    }
}

impl State {
    /// Open the shared colour picker on a target the workspace owns.
    pub(in crate::editor) fn open_color_picker(
        &mut self,
        port: &mut Port<'_>,
        target: super::ColorTarget,
    ) -> Result<()> {
        let actions = self.open_picker(port.view().session.project(), target, None)?;
        route(self, port, actions);
        Ok(())
    }

    /// Complete the source picker; a refusal drops the captured LUT target.
    pub(in crate::editor) fn lut_picked_file(
        &mut self,
        port: &mut Port<'_>,
        path: Option<std::path::PathBuf>,
    ) -> Result<()> {
        let result = self.picked_lut(port, path);
        if result.is_err() {
            self.abort_lut();
        }
        result
    }

    /// Switch one element kind's remembered tab as one preference edit.
    fn set_tab(
        &mut self,
        port: &mut Port<'_>,
        kind: &'static str,
        tab: &'static str,
    ) -> Result<()> {
        port.request(crate::editor::context::Request::ArmKeyColor(None));
        self.cancel_font_import(port);
        self.font_picker.close();
        self.chroma_staged = None;
        let mut next = port.view().preferences.properties_tabs.clone();
        next.insert(kind.to_owned(), tab.to_owned());
        port.store_preference_result("properties_tabs", serde_json::to_value(next)?)
    }
}
