//! Applies masks commands to a candidate project.
use super::{EditError, ItemId, ProjectData, Result, item};
pub(super) fn execute_add_mask(
    mut data: ProjectData,
    id: ItemId,
    mask: project::Mask,
) -> Result<ProjectData> {
    item(&mut data, id)?.masks.push(mask);

    Ok(data)
}
pub(super) fn execute_remove_mask(
    mut data: ProjectData,
    id: ItemId,
    mask: String,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let index = target
        .masks
        .iter()
        .position(|m| m.id == mask)
        .ok_or(EditError::Invalid("missing mask"))?;
    target.masks.remove(index);
    let prefix = format!("masks.{mask}.params.");
    let paths: Vec<_> = target
        .animations
        .iter()
        .filter(|(p, _)| p.as_str().starts_with(&prefix))
        .map(|(p, _)| p.clone())
        .collect();
    for path in paths {
        target.animations.remove(&path);
    }

    Ok(data)
}
pub(super) fn execute_update_mask(
    mut data: ProjectData,
    id: ItemId,
    mask: project::Mask,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let previous = target
        .masks
        .iter_mut()
        .find(|m| m.id == mask.id)
        .ok_or(EditError::Invalid("missing mask"))?;
    *previous = mask;

    Ok(data)
}
pub(super) fn execute_reorder_masks(
    mut data: ProjectData,
    id: ItemId,
    order: Vec<String>,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    if order.len() != target.masks.len() {
        return Err(EditError::Invalid("mask order length"));
    }
    let mut reordered = Vec::new();
    for id in order {
        let index = target
            .masks
            .iter()
            .position(|m| m.id == id)
            .ok_or(EditError::Invalid("mask order identity"))?;
        reordered.push(target.masks.remove(index));
    }
    target.masks = reordered;

    Ok(data)
}
