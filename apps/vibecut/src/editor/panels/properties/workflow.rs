//! Applies the properties workflows.
mod appearance;
mod canvas;
mod commands;
mod effects;
mod fonts;
pub(in crate::editor) mod lut;
mod masks;
mod number_edit;
mod speed;
mod transition;

pub(in crate::editor) use commands::{apply_text_animation, set_item_field, set_shared_field};
pub(in crate::editor) use effects::{add_effect_to_item, effect_refusal};
pub(super) use number_edit::transform_scrub;

use super::{Action, State};
use crate::editor::{Desk, Result};

/// Execute the requests `show` returned, in order. Failures surface through
/// the editor's error line; later requests still run. The area's own state is
/// borrowed apart from the editor for the duration of the routing pass.
pub(in crate::editor) fn apply(desk: &mut Desk<'_>, actions: Vec<Action>) {
    if actions.is_empty() {
        return;
    }
    let mut state = std::mem::take(&mut desk.editor().properties);
    route(&mut state, desk, actions);
    desk.editor().properties = state;
}

fn route(state: &mut State, desk: &mut Desk<'_>, actions: Vec<Action>) {
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
                        let view = desk.view();
                        bound_item == item
                            && *at == view.playhead
                            && crate::editor::context::single(view.selection) == Some(*item)
                            && std::sync::Arc::ptr_eq(snapshot, view.session.project())
                    })
            }
            _ => false,
        };
        let result = match action {
            Action::Cache(action) => {
                desk.request(crate::editor::context::Request::Cache(action));
                Ok(())
            }
            Action::ApplyTextAnimation {
                item,
                preset,
                snapshot,
            } => commands::apply_text_animation(state, desk, item, preset, snapshot),
            Action::ApplyTextPreset {
                item,
                preset,
                snapshot,
            } => commands::apply_text_preset(state, desk, item, preset, snapshot),
            Action::SaveTextPreset { item, snapshot } => {
                commands::save_text_preset(state, desk, item, snapshot)
            }
            Action::Activate(id) => {
                // Activation runs on the editor, which holds a placeholder while
                // this pass carries the area state. A refusal did not write it.
                if desk.act(id) {
                    state.set_details(desk.editor().properties.details());
                }
                Ok(())
            }
            Action::ImportLut { item, expected } => state.request_lut(desk, item, expected),
            Action::SelectLut { item, id } => state.select_lut(desk, item, id),
            Action::SetField { item, field, value } => {
                commands::set_item_field(desk, item, &field, &value)
            }
            Action::SetShared { field, value } => commands::set_shared_field(desk, &field, &value),
            Action::SetSpeed {
                item,
                speed,
                maintain_pitch,
            } => speed::set_speed(desk, item, speed, maintain_pitch),
            Action::SetTransitionDuration {
                transition,
                duration,
            } => transition::set_duration(desk, transition, duration),
            Action::SetTransitionAlignment {
                transition,
                alignment,
            } => transition::set_alignment(desk, transition, alignment),
            Action::RemoveTransition(transition) => transition::remove(desk, transition),
            Action::SetCanvas {
                width,
                height,
                preset,
            } => canvas::set_canvas(desk, width, height, preset),
            Action::SetFps { num, den } => canvas::set_canvas_fps(desk, num, den),
            Action::SetBackground(color) => canvas::set_canvas_background(desk, color),
            Action::ResetTransform(item) => commands::reset_transform(desk, item),
            Action::ResetBlend(item) => commands::reset_blend(desk, item),
            Action::ToggleKeyframe { item, fields, time } => {
                commands::toggle_keyframes(desk, item, &fields, time)
            }
            Action::SetKeyframeAt {
                item,
                field,
                time,
                value,
            } => commands::set_keyframe_at(desk, item, &field, time, value),
            Action::SetKeyColorPick(pick) => {
                desk.arm_key_color_pick(pick);
                Ok(())
            }
            Action::ToggleChromaKey { item, enabled } => {
                state.toggle_chroma_key(desk, item, enabled);
                Ok(())
            }
            Action::ResetKeyColor { item, effect } => state.reset_key_color(desk, item, &effect),
            Action::SetSection { key, open } => desk.set_section_open(key, open),
            Action::SetTab { kind, tab } => state.set_tab(desk, kind, tab),
            Action::ScrubBegin => state.begin_gesture(desk),
            Action::ScrubEnd { commit, changed } => state.end_gesture(desk, commit, changed),
            Action::SetEffectParam {
                item,
                effect,
                key,
                value,
            } => effects::set_effect_param(desk, item, &effect, &key, value),
            Action::SetEffectValues {
                item,
                effect,
                values,
            } => effects::set_effect_values(desk, item, &effect, &values),
            Action::ApplyGradeToAll { item } => effects::apply_grade_to_all(desk, item),
            Action::SaveGradePreset { item } => effects::save_grade_preset(desk, item),
            Action::ToggleEffect {
                item,
                effect,
                enabled,
            } => {
                effects::toggle_effect(desk, item, &effect, enabled);
                Ok(())
            }
            Action::RemoveEffect { item, effect } => {
                effects::remove_effect(desk, item, &effect);
                Ok(())
            }
            Action::ReorderEffect {
                item,
                effect,
                target,
            } => {
                effects::reorder_effect(desk, item, &effect, &target);
                Ok(())
            }
            Action::SyncPreview => {
                desk.sync();
                Ok(())
            }
            Action::SetAnimatedParam { item, path, param } => desk
                .apply_result(
                    engine::Command::SetAnimatedParam { item, path, param },
                    false,
                )
                .map(|_| ()),
            Action::ColorPreview { command } => state.preview_color(desk, command),
            Action::ColorCommit { command, recent } => state.commit_color(desk, command, recent),
            Action::Font(action) => state.font_action(desk, action),
            Action::Mask(action) => state.mask_action(desk, action),
            Action::Error(message) => {
                desk.fail(message);
                Ok(())
            }
        };
        let succeeded = result.is_ok();
        if let Err(error) = result {
            desk.fail(error.to_string());
        }
        if refresh_scrub_snapshot
            && succeeded
            && let Some((snapshot, _, _)) = &mut state.opacity_edit
        {
            *snapshot = desk.view().session.project().clone();
        }
    }
}

impl State {
    /// Switch one element kind's remembered tab as one preference edit.
    fn set_tab(
        &mut self,
        desk: &mut Desk<'_>,
        kind: &'static str,
        tab: &'static str,
    ) -> Result<()> {
        desk.arm_key_color_pick(None);
        self.cancel_font_import(desk);
        self.font_picker.close();
        self.chroma_staged = None;
        let mut next = desk.view().preferences.properties_tabs.clone();
        next.insert(kind.to_owned(), tab.to_owned());
        desk.store_preference("properties_tabs", serde_json::to_value(next)?)
    }
}
