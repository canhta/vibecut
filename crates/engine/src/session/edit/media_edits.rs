//! Applies media commands to a candidate project.
use super::{AssetUsage, BTreeMap, BTreeSet, EditError, ItemId, ProjectData, Result};

/// Remove unused asset records atomically. Every referenced asset refuses with
/// the exact item list; no clip or scene is ever changed by a removal.
pub(super) fn remove_assets(data: &mut ProjectData, assets: Vec<media::AssetId>) -> Result<()> {
    if assets.is_empty() {
        return Err(EditError::Invalid("media removal list"));
    }
    if assets.len() > crate::session::wire::MAX_COLLECTION {
        return Err(EditError::Limit);
    }
    let mut unique = BTreeSet::new();
    for id in &assets {
        if !unique.insert(*id) {
            return Err(EditError::Invalid("duplicate asset ID"));
        }
    }
    let existing: BTreeSet<_> = data.assets.iter().map(|asset| asset.id()).collect();
    if let Some(id) = assets.iter().find(|id| !existing.contains(id)) {
        return Err(EditError::NoAsset(*id));
    }
    let mut usage: BTreeMap<media::AssetId, Vec<ItemId>> = BTreeMap::new();
    for scene in data.scenes.iter() {
        for track in scene.timeline.iter() {
            for item in &track.items {
                if let Some(asset) = item.source_asset
                    && unique.contains(&asset)
                {
                    usage.entry(asset).or_default().push(item.id);
                }
            }
        }
    }
    if !usage.is_empty() {
        return Err(EditError::AssetsInUse(
            usage
                .into_iter()
                .map(|(asset, items)| AssetUsage { asset, items })
                .collect(),
        ));
    }
    data.assets.retain(|asset| !unique.contains(&asset.id()));
    Ok(())
}
// Candidate mutations are committed only after full project validation; batches
// traverse commands in caller order, so the first successful visual insert wins.
pub(super) fn ratchet_fps(canvas: &mut project::Canvas, asset: &media::SourceAsset) {
    if !asset.is_image() {
        for stream in &asset.probe().streams {
            if stream.kind == media::StreamKind::Video
                && let Some(rate) = stream.frame_rate
            {
                canvas.fps = canvas.fps.max(rate);
            }
        }
    }
}
pub(super) fn execute_add_asset(
    mut data: ProjectData,
    asset: media::SourceAsset,
) -> Result<ProjectData> {
    ratchet_fps(&mut data.canvas, &asset);
    data.assets.push(asset);

    Ok(data)
}
pub(super) fn execute_remove_assets(
    mut data: ProjectData,
    assets: Vec<media::AssetId>,
) -> Result<ProjectData> {
    remove_assets(&mut data, assets)?;
    Ok(data)
}
