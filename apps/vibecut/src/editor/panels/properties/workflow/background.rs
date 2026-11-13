//! Applies the Background tab's commits through `timeline.set_background`.
use crate::editor::{ItemId, Port, Result};

/// One clip-background commit over `items`; the engine refuses the whole list
/// if any item is not a video item, so one undo step covers every target.
pub(super) fn set_clip_background(
    port: &mut Port<'_>,
    items: Vec<ItemId>,
    background: project::ClipBackground,
) -> Result<()> {
    port.apply_result(engine::Command::SetItemsBackground { items, background })?;
    Ok(())
}
