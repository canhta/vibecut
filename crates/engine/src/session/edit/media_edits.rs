//! Applies media commands to a candidate project.
use super::{
    AssetUsage, BTreeMap, BTreeSet, EditError, ItemId, ItemParams, ProjectData, Result, item,
};

/// Remove unused asset records atomically. Every referenced asset refuses with
/// the exact item list; no clip or scene is ever changed by a removal.
pub(super) fn remove_assets(data: &mut ProjectData, assets: Vec<media::AssetId>) -> Result<()> {
    if assets.is_empty() {
        return Err(EditError::Invalid("media removal list"));
    }
    if assets.len() > crate::session::wire::MAX_COLLECTION {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
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
/// Point an existing asset record at the file the caller found for it. Identity, content
/// hash and kind stay as they are, so every clip that names the asset follows the new
/// path and nothing else in the project changes. A record for another asset or a
/// different content hash refuses; the caller proved the content match when it built the
/// record with `SourceAsset::relink`.
pub(super) fn execute_relink_asset(
    mut data: ProjectData,
    asset: media::SourceAsset,
) -> Result<ProjectData> {
    let record = data
        .assets
        .iter_mut()
        .find(|record| record.id() == asset.id())
        .ok_or(EditError::NoAsset(asset.id()))?;
    if record.content_hash() != asset.content_hash() || record.ephemeral() != asset.ephemeral() {
        return Err(EditError::Invalid("relink content differs"));
    }
    *record = asset;
    Ok(data)
}
pub(super) fn execute_remove_assets(
    mut data: ProjectData,
    assets: Vec<media::AssetId>,
) -> Result<ProjectData> {
    remove_assets(&mut data, assets)?;
    Ok(data)
}

/// Replace each listed clip with another registered source, the way the reference's Replace
/// clip does: a clip keeps its place, output length, speed, pitch and reverse flags, params
/// (volume included), effects, masks, keyframes and transitions, and plays the new source from
/// its start (source in point zero) for the length it already has, so a clip trimmed into the
/// middle of the old media starts at the head of the new one. The retained cut window is dropped
/// because the source window it described no longer exists. The new source must carry the item's
/// stream kind and cover the window the clip needs when its duration is known. A clip under a
/// speed curve, or whose length the constant speed cannot reproduce from whole source ticks,
/// refuses. Targets are checked in order on the candidate; a refusal discards it, so project and
/// history stay unchanged. Partners (for example the audio extracted from a video clip) are
/// replaced only when the caller lists them.
pub(super) fn execute_replace_media(
    mut data: ProjectData,
    items: Vec<ItemId>,
    asset: media::AssetId,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let source = data
        .assets
        .iter()
        .find(|record| record.id() == asset)
        .cloned()
        .ok_or(EditError::NoAsset(asset))?;
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate replace item"));
        }
        let target = item(&mut data, id)?;
        let needed = match target.content {
            ItemParams::Video(_) => media::StreamKind::Video,
            ItemParams::Audio(_) => media::StreamKind::Audio,
            _ => return Err(EditError::Unsupported("replace media on non-media item")),
        };
        if !source
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == needed)
        {
            return Err(EditError::Invalid("replacement media kind mismatch"));
        }
        if target.retime.curve.is_some() {
            return Err(EditError::Unsupported("replace media under a speed curve"));
        }
        let length = target.duration()?;
        let span = target
            .retime
            .speed
            .source_offset(length, time::Rounding::Ceil)?;
        if target.retime.speed.output_duration(span)? != length {
            return Err(EditError::Invalid("clip length not representable"));
        }
        if source
            .probe()
            .duration
            .is_some_and(|duration| span > duration)
        {
            return Err(EditError::Invalid("replacement source too short"));
        }
        target.source_range = time::SourceRange::new(time::Tick::ZERO, span)
            .map_err(|_| EditError::Invalid("replacement source window"))?;
        target.retime.window = None;
        target.source_asset = Some(asset);
    }
    Ok(data)
}
