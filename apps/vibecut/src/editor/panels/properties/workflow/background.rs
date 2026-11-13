//! Applies the Background tab's commits through `timeline.set_background`.
use crate::editor::{Desk, ItemId, Result};

/// One clip-background commit over `items`; the engine refuses the whole list
/// if any item is not a video item, so one undo step covers every target.
pub(super) fn set_clip_background(
    desk: &mut Desk<'_>,
    items: Vec<ItemId>,
    background: project::ClipBackground,
) -> Result<()> {
    desk.apply_result(
        engine::Command::SetItemsBackground { items, background },
        false,
    )?;
    Ok(())
}
