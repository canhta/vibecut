//! Admits export only when every switched-on footage correction has its stored analysis.
use super::{Error, Result};
use project::{ItemId, ItemParams, Project};

/// Refuses naming each visible video item of the project's active scene that has stabilization
/// or background removal switched on and no stored correction for it in `render`, so export never
/// differs silently from a preview that drew the correction.
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
                && match &item.content {
                    ItemParams::Video(p) => {
                        (p.stabilize.is_some() && !render.stabilization.contains_key(&item.id))
                            || (p.background_removal.is_some()
                                && !render.person_mattes.contains_key(&item.id))
                    }
                    _ => false,
                }
        })
        .map(|item| item.id)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::CorrectionMissing(missing))
    }
}
