//! Applies the properties workflows: the frame routes `Action`s here.
mod appearance;
mod canvas;
mod commands;
mod effects;
mod number_edit;
mod speed;

pub(super) use number_edit::transform_scrub;

use super::{Action, Editor, Error};

/// Execute the requests `show` returned, in order. Failures surface through
/// the editor's error line; later requests still run.
pub(in crate::editor) fn apply(editor: &mut Editor, actions: Vec<Action>) {
    for action in actions {
        let result = match action {
            Action::Activate(id) => {
                editor.action(id);
                Ok(())
            }
            Action::SetField { item, field, value } => editor.set_item_field(item, &field, &value),
            Action::SetShared { field, value } => editor.set_shared_field(&field, &value),
            Action::SetSpeed {
                item,
                speed,
                maintain_pitch,
            } => editor.set_speed(item, speed, maintain_pitch),
            Action::SetCanvas {
                width,
                height,
                preset,
            } => editor.set_canvas(width, height, preset),
            Action::SetFps { num, den } => editor.set_canvas_fps(num, den),
            Action::SetBackground(color) => editor.set_canvas_background(color),
            Action::ResetTransform(item) => {
                let mut result = Ok(());
                for field in project::TransformField::ALL {
                    if let Err(error) = editor.set_item_field(
                        item,
                        field.path(),
                        &field.default_value().to_string(),
                    ) {
                        result = Err(error);
                    }
                }
                result
            }
            Action::ToggleKeyframe {
                item,
                field,
                time,
                value,
            } => editor.toggle_keyframe(item, field, time, value),
            Action::SetKeyframeAt {
                item,
                field,
                time,
                value,
            } => editor.set_keyframe_at(item, field, time, value),
            Action::SetSection { key, open } => editor.set_section_open(key, open),
            Action::SetTab { kind, tab } => {
                let mut next = editor.preferences.properties_tabs.clone();
                next.insert(kind.to_owned(), tab.to_owned());
                serde_json::to_value(next)
                    .map_err(|_| Error::Invalid("tabs"))
                    .and_then(|value| editor.preferences.set("properties_tabs", value))
            }
            Action::ScrubBegin => editor.begin_gesture(),
            Action::ScrubEnd { commit, changed } => editor.end_gesture(commit, changed),
            Action::SetEffectParam {
                item,
                effect,
                key,
                value,
            } => editor.set_effect_param(item, &effect, &key, value),
            Action::ToggleEffect {
                item,
                effect,
                enabled,
            } => {
                editor.toggle_effect(item, &effect, enabled);
                Ok(())
            }
            Action::RemoveEffect { item, effect } => {
                editor.remove_effect(item, &effect);
                Ok(())
            }
            Action::ReorderEffect {
                item,
                effect,
                target,
            } => {
                editor.reorder_effect(item, &effect, &target);
                Ok(())
            }
            Action::SyncPreview => {
                editor.sync_preview();
                Ok(())
            }
            Action::ColorPreview { command, params } => editor.preview_color(command, params),
            Action::ColorCommit {
                command,
                params,
                recent,
            } => editor.commit_color(command, params, recent),
            Action::Error(message) => {
                editor.error = Some(message);
                Ok(())
            }
        };
        if let Err(error) = result {
            editor.error = Some(error.to_string());
        }
    }
}
