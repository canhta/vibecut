//! Applies the mask row's shape and field edits through the mask commands.
use super::super::mask_picker::{Action, Choice};
use super::super::{State, read};
use crate::editor::{Desk, ItemId, Result};
use project::MaskShape;

impl State {
    pub(super) fn mask_action(&mut self, desk: &mut Desk<'_>, action: Action) -> Result<()> {
        match action {
            Action::Close => {
                self.mask.close();
                Ok(())
            }
            Action::Choose { item, shape } => choose_mask(desk, item, shape),
            Action::Update { item, mask } => {
                desk.apply_result(engine::Command::UpdateMask { item, mask }, false)?;
                Ok(())
            }
        }
    }
}

/// One mask per clip from the panel: the first mask is the one the row
/// shows, so adding fills an empty list and changing edits that mask.
fn choose_mask(desk: &mut Desk<'_>, item: ItemId, shape: Option<MaskShape>) -> Result<()> {
    let current = read::item(desk.view().session.project(), item)
        .and_then(|item| item.masks.first())
        .cloned();
    match (shape, current) {
        (None, None) => return Ok(()),
        (None, Some(mask)) => {
            desk.apply_result(
                engine::Command::RemoveMask {
                    item,
                    mask: mask.id,
                },
                false,
            )?;
        }
        (Some(shape), Some(mut mask)) => {
            if mask.shape == shape {
                return Ok(());
            }
            mask.shape = shape;
            desk.apply_result(engine::Command::UpdateMask { item, mask }, false)?;
        }
        (Some(shape), None) => {
            let rotation = Choice::of(&shape).map_or(0.0, |choice| choice.default_rotation());
            let mut mask = project::Mask::new("mask1", shape);
            mask.rotation = rotation;
            desk.apply_result(engine::Command::AddMask { item, mask }, false)?;
        }
    }
    Ok(())
}
