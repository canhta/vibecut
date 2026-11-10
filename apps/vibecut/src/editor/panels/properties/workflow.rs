//! Applies the properties workflows.
mod appearance;
mod canvas;
mod commands;
mod effects;
mod fonts;
mod masks;
mod number_edit;
mod speed;

pub(in crate::editor) use commands::{set_item_field, set_shared_field};
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
        let result = match action {
            Action::Activate(id) => {
                // Activation runs on the editor, which holds a placeholder while
                // this pass carries the area state. A refusal did not write it.
                if desk.act(id) {
                    state.set_details(desk.editor().properties.details());
                }
                Ok(())
            }
            Action::SetField { item, field, value } => {
                commands::set_item_field(desk, item, &field, &value)
            }
            Action::SetShared { field, value } => commands::set_shared_field(desk, &field, &value),
            Action::SetSpeed {
                item,
                speed,
                maintain_pitch,
            } => speed::set_speed(desk, item, speed, maintain_pitch),
            Action::SetCanvas {
                width,
                height,
                preset,
            } => canvas::set_canvas(desk, width, height, preset),
            Action::SetFps { num, den } => canvas::set_canvas_fps(desk, num, den),
            Action::SetBackground(color) => canvas::set_canvas_background(desk, color),
            Action::ResetTransform(item) => {
                let mut result = Ok(());
                for field in project::TransformField::ALL {
                    if let Err(error) = commands::set_item_field(
                        desk,
                        item,
                        field.builtin().as_str(),
                        &field.default_value().to_string(),
                    ) {
                        result = Err(error);
                    }
                }
                result
            }
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
            Action::ColorPreview { command } => state.preview_color(desk, command),
            Action::ColorCommit { command, recent } => state.commit_color(desk, command, recent),
            Action::Font(action) => state.font_action(desk, action),
            Action::Mask(action) => state.mask_action(desk, action),
            Action::Error(message) => {
                desk.fail(message);
                Ok(())
            }
        };
        if let Err(error) = result {
            desk.fail(error.to_string());
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
        self.chroma_staged = None;
        let mut next = desk.view().preferences.properties_tabs.clone();
        next.insert(kind.to_owned(), tab.to_owned());
        desk.store_preference("properties_tabs", serde_json::to_value(next)?)
    }
}
