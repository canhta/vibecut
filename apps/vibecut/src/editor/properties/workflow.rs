//! Applies the properties workflows: the frame routes `Action`s here.
mod appearance;
mod canvas;
mod commands;
mod effects;
mod fonts;
mod masks;
mod number_edit;
mod speed;

pub(super) use number_edit::transform_scrub;

use super::Action;
use crate::Error;
use crate::editor::Desk;

/// Execute the requests `show` returned, in order. Failures surface through
/// the editor's error line; later requests still run.
pub(in crate::editor) fn apply(desk: &mut Desk<'_>, actions: Vec<Action>) {
    for action in actions {
        let result = match action {
            Action::Activate(id) => {
                desk.act(id);
                Ok(())
            }
            Action::SetField { item, field, value } => {
                desk.editor().set_item_field(item, &field, &value)
            }
            Action::SetShared { field, value } => desk.editor().set_shared_field(&field, &value),
            Action::SetSpeed {
                item,
                speed,
                maintain_pitch,
            } => desk.editor().set_speed(item, speed, maintain_pitch),
            Action::SetCanvas {
                width,
                height,
                preset,
            } => desk.editor().set_canvas(width, height, preset),
            Action::SetFps { num, den } => desk.editor().set_canvas_fps(num, den),
            Action::SetBackground(color) => desk.editor().set_canvas_background(color),
            Action::ResetTransform(item) => {
                let mut result = Ok(());
                for field in project::TransformField::ALL {
                    if let Err(error) = desk.editor().set_item_field(
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
                desk.editor().toggle_keyframes(item, &fields, time)
            }
            Action::SetKeyframeAt {
                item,
                field,
                time,
                value,
            } => desk.editor().set_keyframe_at(item, &field, time, value),
            Action::SetKeyColorPick(pick) => {
                desk.editor().player.key_color_pick = pick;
                Ok(())
            }
            Action::ToggleChromaKey { item, enabled } => {
                desk.editor().toggle_chroma_key(item, enabled);
                Ok(())
            }
            Action::ResetKeyColor { item, effect } => desk.editor().reset_key_color(item, &effect),
            Action::SetSection { key, open } => desk.editor().set_section_open(key, open),
            Action::SetTab { kind, tab } => {
                desk.editor().player.key_color_pick = None;
                desk.editor().properties.chroma_staged = None;
                let mut next = desk.editor().preferences.properties_tabs.clone();
                next.insert(kind.to_owned(), tab.to_owned());
                serde_json::to_value(next)
                    .map_err(|_| Error::Invalid("tabs"))
                    .and_then(|value| desk.editor().preferences.set("properties_tabs", value))
            }
            Action::ScrubBegin => desk.editor().begin_gesture(),
            Action::ScrubEnd { commit, changed } => desk.editor().end_gesture(commit, changed),
            Action::SetEffectParam {
                item,
                effect,
                key,
                value,
            } => desk.editor().set_effect_param(item, &effect, &key, value),
            Action::ToggleEffect {
                item,
                effect,
                enabled,
            } => {
                desk.editor().toggle_effect(item, &effect, enabled);
                Ok(())
            }
            Action::RemoveEffect { item, effect } => {
                desk.editor().remove_effect(item, &effect);
                Ok(())
            }
            Action::ReorderEffect {
                item,
                effect,
                target,
            } => {
                desk.editor().reorder_effect(item, &effect, &target);
                Ok(())
            }
            Action::SyncPreview => {
                desk.sync();
                Ok(())
            }
            Action::ColorPreview { command } => desk.editor().preview_color(command),
            Action::ColorCommit { command, recent } => desk.editor().commit_color(command, recent),
            Action::Font(action) => desk.editor().font_action(action),
            Action::Mask(action) => desk.editor().mask_action(action),
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
