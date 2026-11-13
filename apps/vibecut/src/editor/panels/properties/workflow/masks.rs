//! Applies the mask row's shape and field edits through the mask commands.
use super::super::mask_picker::{Action, Choice};
use super::super::{State, read};
use super::commands;
use crate::editor::{ItemId, Port, Result};
use project::MaskShape;

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
            Action::Choose { item, shape } => choose_mask(port, item, shape),
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

/// One mask per clip from the panel: the first mask is the one the row
/// shows, so adding fills an empty list and changing edits that mask.
fn choose_mask(port: &mut Port<'_>, item: ItemId, shape: Option<MaskShape>) -> Result<()> {
    let current = read::item(port.view().session.project(), item)
        .and_then(|item| item.masks.first())
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
            let mut mask = project::Mask::new("mask1", shape);
            mask.rotation = rotation;
            port.apply_result(engine::Command::AddMask { item, mask })?;
        }
    }
    Ok(())
}
