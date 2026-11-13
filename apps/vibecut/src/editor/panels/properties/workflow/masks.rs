//! Applies the mask row's shape and field edits through the mask commands.
use super::super::mask_picker::{Action, Choice};
use super::super::{State, read};
use crate::editor::{ItemId, Port, Result};
use project::MaskShape;

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
