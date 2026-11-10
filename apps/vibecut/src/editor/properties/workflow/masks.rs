//! Applies the mask row's shape and field edits through the mask commands.
use super::super::mask_picker::{Action, Choice};
use crate::editor::properties::{Editor, Result, read};
use project::MaskShape;

impl Editor {
    pub(in crate::editor::properties) fn mask_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Close => {
                self.properties.mask.close();
                Ok(())
            }
            Action::Choose { item, shape } => self.choose_mask(item, shape),
            Action::Update { item, mask } => {
                self.desk()
                    .apply_result(engine::Command::UpdateMask { item, mask }, false)?;
                Ok(())
            }
        }
    }

    /// One mask per clip from the panel: the first mask is the one the row
    /// shows, so adding fills an empty list and changing edits that mask.
    fn choose_mask(&mut self, item: project::ItemId, shape: Option<MaskShape>) -> Result<()> {
        let current = read::item(self.session.project(), item)
            .and_then(|item| item.masks.first())
            .cloned();
        match (shape, current) {
            (None, None) => return Ok(()),
            (None, Some(mask)) => {
                self.desk().apply_result(
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
                self.desk()
                    .apply_result(engine::Command::UpdateMask { item, mask }, false)?;
            }
            (Some(shape), None) => {
                let rotation = Choice::of(&shape).map_or(0.0, |choice| choice.default_rotation());
                let mut mask = project::Mask::new("mask1", shape);
                mask.rotation = rotation;
                self.desk()
                    .apply_result(engine::Command::AddMask { item, mask }, false)?;
            }
        }
        Ok(())
    }
}
