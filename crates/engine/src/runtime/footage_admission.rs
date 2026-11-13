//! Admits export only when every switched-on footage correction has its stored analysis.
use super::{Error, Result};
use project::{ItemId, ItemParams, Project};

/// Refuses naming each visible video item of the project's active scene that has stabilization
/// switched on and no correction in `render`, so export never differs silently from a preview
/// that drew the correction.
pub fn admit(project: &Project, render: &render::RenderSettings) -> Result<()> {
    let missing: Vec<ItemId> = project
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| !track.hidden)
        .flat_map(|track| &track.items)
        .filter(|item| {
            !item.hidden
                && matches!(&item.content, ItemParams::Video(p) if p.stabilize.is_some())
                && !render.stabilization.contains_key(&item.id)
        })
        .map(|item| item.id)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::CorrectionMissing(missing))
    }
}
