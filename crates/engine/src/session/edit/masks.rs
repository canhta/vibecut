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
/// Replace one mask's shape with an editable closed path, preserving every
/// other field and its animations. An equal path is a silent no-op.
pub(super) fn execute_set_mask_path(
    mut data: ProjectData,
    id: ItemId,
    mask_id: String,
    path: project::MaskPath,
) -> Result<ProjectData> {
    path.editable()?;
    let target = item(&mut data, id)?;
    let mask = target
        .masks
        .iter_mut()
        .find(|m| m.id == mask_id)
        .ok_or(EditError::Invalid("missing mask"))?;
    if let project::MaskShape::Path { path: current } = &mask.shape
        && current == &path
    {
        return Ok(data);
    }
    mask.shape = project::MaskShape::Path { path };

    Ok(data)
}
/// Insert one anchor into an editable closed path at parameter `t`.
pub(super) fn execute_add_mask_point(
    mut data: ProjectData,
    id: ItemId,
    mask_id: String,
    segment: usize,
    t: f64,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let mask = target
        .masks
        .iter_mut()
        .find(|m| m.id == mask_id)
        .ok_or(EditError::Invalid("missing mask"))?;
    let project::MaskShape::Path { path } = &mask.shape else {
        return Err(EditError::Invalid("mask is not a path"));
    };
    let next = path.split(segment, t)?;
    mask.shape = project::MaskShape::Path { path: next };

    Ok(data)
}
/// Remove the given anchors from an editable closed path in one undo step.
pub(super) fn execute_remove_mask_point(
    mut data: ProjectData,
    id: ItemId,
    mask_id: String,
    points: Vec<usize>,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let mask = target
        .masks
        .iter_mut()
        .find(|m| m.id == mask_id)
        .ok_or(EditError::Invalid("missing mask"))?;
    let project::MaskShape::Path { path } = &mask.shape else {
        return Err(EditError::Invalid("mask is not a path"));
    };
    let next = path.without(&points)?;
    mask.shape = project::MaskShape::Path { path: next };

    Ok(data)
}
/// Insert the clipboard's anchors after anchor `after` of an editable closed path in
/// one step. An empty clipboard is a silent no-op; an item snapshot refuses.
pub(super) fn execute_paste_mask_points(
    mut data: ProjectData,
    id: ItemId,
    mask_id: String,
    after: usize,
    payload: Option<&crate::ItemClipboard>,
) -> Result<ProjectData> {
    let Some(payload) = payload else {
        return Ok(data);
    };
    let vertices = payload.mask_vertices()?;
    let target = item(&mut data, id)?;
    let mask = target
        .masks
        .iter_mut()
        .find(|m| m.id == mask_id)
        .ok_or(EditError::Invalid("missing mask"))?;
    let project::MaskShape::Path { path } = &mask.shape else {
        return Err(EditError::Invalid("mask is not a path"));
    };
    let next = path.with_inserted(after, vertices)?;
    mask.shape = project::MaskShape::Path { path: next };

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
