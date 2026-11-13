//! Applies the Audio tab's fade edits through `audio.set_fade`.
use super::super::read;
use crate::editor::{Desk, Error, ItemId, Result};

/// One fade edge in seconds. The engine clamps it to the clip and the other
/// fade and keeps the curve the edge already has.
pub(super) fn set_fade(
    desk: &mut Desk<'_>,
    item: ItemId,
    edge: project::FadeEdge,
    seconds: f64,
) -> Result<()> {
    let curve = read::fade_of(desk.view().session.project(), item, edge)
        .ok_or(Error::Invalid("fade needs an audio or video item"))?
        .curve;
    let duration = time::Tick::from_seconds_f64(seconds, time::Rounding::Nearest)?;
    desk.apply_result(
        engine::Command::SetFade {
            item,
            edge,
            duration,
            curve,
        },
        false,
    )?;
    Ok(())
}
