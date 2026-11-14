//! Applies the mask row's shape and field edits through the mask commands.
use super::super::mask_picker::{self, Action, Choice};
use super::super::{State, read};
use super::commands;
use crate::editor::{ItemId, Port, Result};
use project::{MaskScope, MaskShape};

/// The animation path of one mask field: `masks.<id>.params.<key>`.
fn mask_path(mask: &str, key: &str) -> String {
    format!("masks.{mask}.params.{key}")
}

impl State {
    pub(super) fn mask_action(&mut self, port: &mut Port<'_>, action: Action) -> Result<()> {
        match action {
            Action::Close => {
                self.mask.close();
                Ok(())
            }
            Action::Choose { item, shape } => choose_mask(port, item, self.mask.scope(), shape),
            Action::Update { item, mask } => {
                port.apply_result(engine::Command::UpdateMask { item, mask })?;
                Ok(())
            }
            // The same seek the properties keyed sliders use.
            Action::Seek { at } => {
                port.request(crate::editor::context::Request::Seek(at));
                Ok(())
            }
            Action::ToggleKeyframe {
                item,
                mask,
                fields,
                time,
            } => {
                let fields: Vec<(String, f64)> = fields
                    .into_iter()
                    .map(|(key, value)| (mask_path(&mask, key), value))
                    .collect();
                commands::toggle_keyframes(port, item, &fields, time)
            }
            Action::SetKeyframe {
                item,
                mask,
                key,
                value,
                time,
            } => commands::set_keyframe_at(port, item, &mask_path(&mask, key), time, value),
            Action::SetBase {
                item,
                mask,
                key,
                value,
            } => {
                port.apply_result(engine::Command::SetAnimatedParam {
                    item,
                    path: project::AnimationPath::new(mask_path(&mask, key))?,
                    param: project::Param::new(project::ParamValue::Scalar(value))?,
                })?;
                Ok(())
            }
        }
    }
}

/// One mask per scope from the panel: the first mask of the open scope is the one the row
/// shows, so adding fills an empty scope and changing edits that mask.
fn choose_mask(
    port: &mut Port<'_>,
    item: ItemId,
    scope: MaskScope,
    shape: Option<MaskShape>,
) -> Result<()> {
    let current = read::item(port.view().session.project(), item)
        .and_then(|item| mask_picker::current(item, scope))
        .cloned();
    match (shape, current) {
        (None, None) => return Ok(()),
        (None, Some(mask)) => {
            port.apply_result(engine::Command::RemoveMask {
                item,
                mask: mask.id,
            })?;
        }
        (Some(shape), Some(mut mask)) => {
            if mask.shape == shape
                || Choice::of(&shape).is_some_and(|choice| choice.keeps(&mask.shape))
            {
                return Ok(());
            }
            mask.shape = shape;
            port.apply_result(engine::Command::UpdateMask { item, mask })?;
        }
        (Some(shape), None) => {
            let rotation = Choice::of(&shape).map_or(0.0, |choice| choice.default_rotation());
            let id = match scope {
                MaskScope::Clip => "mask1",
                MaskScope::Adjust => "adjust1",
            };
            let mut mask = project::Mask::new(id, shape);
            mask.scope = scope;
            mask.rotation = rotation;
            port.apply_result(engine::Command::AddMask { item, mask })?;
        }
    }
    Ok(())
}
