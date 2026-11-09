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
                self.session.set_selection(self.selection.clone())?;
                self.session
                    .execute("masks.update", serde_json::json!({"item":item,"mask":mask}))?;
                self.sync_preview();
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
                self.session.set_selection(self.selection.clone())?;
                self.session.execute(
                    "masks.remove",
                    serde_json::json!({"item":item,"mask":mask.id}),
                )?;
            }
            (Some(shape), Some(mut mask)) => {
                if mask.shape == shape {
                    return Ok(());
                }
                mask.shape = shape;
                self.session.set_selection(self.selection.clone())?;
                self.session
                    .execute("masks.update", serde_json::json!({"item":item,"mask":mask}))?;
            }
            (Some(shape), None) => {
                let rotation = Choice::of(&shape).map_or(0.0, |choice| choice.default_rotation());
                let mut mask = project::Mask::new("mask1", shape);
                mask.rotation = rotation;
                self.session.set_selection(self.selection.clone())?;
                self.session
                    .execute("masks.add", serde_json::json!({"item":item,"mask":mask}))?;
            }
        }
        self.sync_preview();
        Ok(())
    }
}
