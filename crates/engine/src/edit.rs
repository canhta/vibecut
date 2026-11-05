use crate::{
    AssetUsage, Command, CreatedId, EditError, Move, NewItem, Placement, Result, TrackPlacement,
};
use project::{
    Bookmark, BookmarkId, Item, ItemId, ItemParams, ProjectData, Track, TrackId, TrackKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use time::{SourceRange, Tick};

pub(crate) type TrimSources = BTreeMap<ItemId, Arc<captions::CueTrack>>;

fn apply_patch(
    target: &mut Item,
    patch: &crate::ItemParamPatch,
    copied_text_bytes: &mut usize,
) -> Result<()> {
    if let Some((field, value)) = patch.transform() {
        target.animations.set(
            project::AnimationPath::new(field.path())?,
            project::Param::new(project::ParamValue::Scalar(value))?,
        )?;
        return Ok(());
    }
    if let crate::ItemParamPatch::TextContent(value) | crate::ItemParamPatch::TextFont(value) =
        patch
    {
        *copied_text_bytes = copied_text_bytes
            .checked_add(value.as_str().len())
            .filter(|bytes| *bytes <= crate::control::MAX_TEXT_BYTES)
            .ok_or(EditError::Limit)?;
    }
    match patch {
        crate::ItemParamPatch::BlendMode(mode) => target.blend_mode = *mode,
        crate::ItemParamPatch::Opacity(value) => {
            target.animations.set(
                project::AnimationPath::new("opacity")?,
                project::Param::new(project::ParamValue::Scalar(*value))?,
            )?;
        }
        crate::ItemParamPatch::AudioGain(v) => {
            match &mut target.content {
                ItemParams::Audio(p) => p.volume = *v,
                ItemParams::Video(p) => p.volume = *v,
                _ => {}
            }
            target.animations.set(
                project::AnimationPath::new("volume")?,
                project::Param::new(project::ParamValue::Scalar(f64::from(*v)))?,
            )?;
        }
        crate::ItemParamPatch::AudioMuted(v) => {
            match &mut target.content {
                ItemParams::Audio(p) => p.muted = *v,
                ItemParams::Video(p) => p.muted = *v,
                _ => {}
            }
            target.animations.set(
                project::AnimationPath::new("muted")?,
                project::Param::new(project::ParamValue::Bool(*v))?,
            )?;
        }
        crate::ItemParamPatch::TextContent(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.content = v.clone();
            }
        }
        crate::ItemParamPatch::TextFont(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.font = v.clone();
            }
        }
        crate::ItemParamPatch::TextWeight(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.weight = *v;
            }
        }
        crate::ItemParamPatch::TextItalic(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.italic = *v;
            }
        }
        crate::ItemParamPatch::TextLetterSpacing(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.letter_spacing = *v;
            }
        }
        crate::ItemParamPatch::TextLineHeight(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.line_height = *v;
            }
        }
        crate::ItemParamPatch::TextBackground(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background = v.clone();
            }
        }
        crate::ItemParamPatch::TextBackgroundColor(value) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).color = *value;
            }
        }
        crate::ItemParamPatch::TextPositionX(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.position.x = *v;
            }
        }
        crate::ItemParamPatch::TextPositionY(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.position.y = *v;
            }
        }
        crate::ItemParamPatch::TextSize(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.font_size = *v;
            }
        }
        crate::ItemParamPatch::TextColor(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.color = *v;
            }
        }
        crate::ItemParamPatch::GraphicFill(v) => {
            if let ItemParams::Overlay(p) = &mut target.content {
                p.color = *v;
            }
        }
        crate::ItemParamPatch::GraphicStroke(v) => {
            let [r, g, b, a] = v.0;
            target.animations.set(
                project::AnimationPath::new("params.strokeColor")?,
                project::Param::new(project::ParamValue::Text(
                    format!("#{r:02X}{g:02X}{b:02X}{a:02X}").into(),
                ))?,
            )?;
        }
        _ => return Err(EditError::Invalid("item patch")),
    }
    Ok(())
}

pub(crate) fn check_budget(command: &Command) -> Result<()> {
    let mut stack = vec![(command, 0)];
    let mut count = 0;
    while let Some((command, depth)) = stack.pop() {
        count += 1;
        if count > crate::control::MAX_BATCH_COMMANDS || depth > crate::control::MAX_BATCH_DEPTH {
            return Err(EditError::Limit);
        }
        match command {
            Command::Batch { commands } => {
                if commands.len() > crate::control::MAX_BATCH_COMMANDS {
                    return Err(EditError::Limit);
                }
                stack.extend(commands.iter().map(|c| (c, depth + 1)));
            }
            Command::MoveItems { moves } if moves.len() > crate::control::MAX_COLLECTION => {
                return Err(EditError::Limit);
            }
            Command::Delete { items } | Command::SetItemParamMany { items, .. }
                if items.len() > crate::control::MAX_COLLECTION =>
            {
                return Err(EditError::Limit);
            }
            Command::RemoveAssets { assets } if assets.len() > crate::control::MAX_COLLECTION => {
                return Err(EditError::Limit);
            }
            Command::Inspect {}
            | Command::Preview { .. }
            | Command::Snapshot { .. }
            | Command::Export { .. }
            | Command::JobStatus { .. }
            | Command::JobCancel { .. }
            | Command::Catalog {}
            | Command::ProviderCatalog {}
            | Command::FontsList { .. }
            | Command::FontsLoad { .. }
            | Command::FontsDownload { .. }
            | Command::ExportSubtitles { .. }
            | Command::BurnSubtitles { .. } => {
                return Err(EditError::Invalid("export commands require job submission"));
            }
            Command::DuplicateProject { .. }
            | Command::DeleteProject {}
            | Command::ThumbnailProject {}
            | Command::CreateProject { .. }
            | Command::SaveProject {}
            | Command::SetViewState { .. }
            | Command::SwitchScene { .. }
            | Command::Undo {}
            | Command::Redo {}
            | Command::BeginGesture {}
            | Command::CommitGesture {}
            | Command::CancelGesture {} => {
                return Err(EditError::Invalid("session control inside batch"));
            }
            _ => {}
        }
    }
    Ok(())
}
// Candidate mutations are committed only after full project validation; batches
// traverse commands in caller order, so the first successful visual insert wins.
fn ratchet_fps(canvas: &mut project::Canvas, asset: &media::SourceAsset) {
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

fn alloc(next: &mut Option<u64>) -> Result<u64> {
    let id = next.ok_or(EditError::IdExhausted)?;
    *next = id.checked_add(1);
    Ok(id)
}
fn track(data: &mut ProjectData, id: TrackId) -> Result<&mut Track> {
    data.scene_mut()
        .timeline
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or(EditError::NoTrack(id))
}
fn unlocked(track: &Track) -> Result<()> {
    if track.locked {
        Err(EditError::Locked(track.id))
    } else {
        Ok(())
    }
}
/// Remove unused asset records atomically. Every referenced asset refuses with
/// the exact item list; no clip or scene is ever changed by a removal.
fn remove_assets(data: &mut ProjectData, assets: Vec<media::AssetId>) -> Result<()> {
    if assets.is_empty() {
        return Err(EditError::Invalid("media removal list"));
    }
    if assets.len() > crate::control::MAX_COLLECTION {
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
fn item(data: &mut ProjectData, id: ItemId) -> Result<&mut Item> {
    for t in data.scene_mut().timeline.iter_mut() {
        if t.items.iter().any(|i| i.id == id) {
            unlocked(t)?;
            return t
                .items
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or(EditError::NoItem(id));
        }
    }
    Err(EditError::NoItem(id))
}
fn effect_item(data: &mut ProjectData, id: ItemId) -> Result<&mut Item> {
    let target = item(data, id)?;
    if matches!(target.content, ItemParams::Audio(_)) {
        return Err(EditError::Unsupported("visual effect on audio item"));
    }
    Ok(target)
}
fn take_item(data: &mut ProjectData, id: ItemId) -> Result<Item> {
    for t in data.scene_mut().timeline.iter_mut() {
        if let Some(index) = t.items.iter().position(|i| i.id == id) {
            unlocked(t)?;
            return Ok(t.items.remove(index));
        }
    }
    Err(EditError::NoItem(id))
}
fn add(a: Tick, b: Tick) -> Result<Tick> {
    a.checked_add(b).ok_or(EditError::Invalid("time overflow"))
}
fn sub(a: Tick, b: Tick) -> Result<Tick> {
    a.checked_sub(b).ok_or(EditError::Invalid("time overflow"))
}

/// Fades of an audible item.
fn fades_mut(content: &mut ItemParams) -> Option<&mut project::Fades> {
    match content {
        ItemParams::Audio(project::AudioParams { fades, .. })
        | ItemParams::Video(project::VideoParams { fades, .. }) => Some(fades),
        _ => None,
    }
}

/// Keep caption clipping with its mapping owner, retaining source cue identity and text.
/// The caller supplies authoritative cues, never a previously clipped preview.
fn set_range(item: &mut Item, range: SourceRange, sources: &mut TrimSources) -> Result<()> {
    if let ItemParams::Caption(p) = &mut item.content {
        let mapping = captions::EditMapping::new(vec![(range, range.start())])?;
        let source = sources
            .entry(item.id)
            .or_insert_with(|| Arc::new(p.cues.clone()));
        let mut cues = mapping.map_cues(source)?;
        for cue in &mut cues.cues {
            cue.id = captions::CueFragmentId::from_id(&cue.id)?.source_id;
        }
        p.cues = cues;
    }
    let shift = sub(item.source_range.start(), range.start())?;
    item.animations.shift(shift)?;
    item.source_range = range;
    Ok(())
}
// Preserve negative input for validation; first placement onto empty main has no leading gap.
fn placement_start(timeline: &project::Timeline, target: TrackId, start: Tick) -> Tick {
    if target == timeline.main.id && timeline.main.items.is_empty() && start > Tick::ZERO {
        Tick::ZERO
    } else {
        start
    }
}
fn move_items(data: &mut ProjectData, moves: Vec<Move>) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut items = Vec::with_capacity(moves.len());
    for m in moves {
        if !seen.insert(m.item) {
            return Err(EditError::Invalid("duplicate move item"));
        }
        let mut item = take_item(data, m.item)?;
        item.start = m.start;
        items.push((m.track, item));
    }
    let main = data.scene().timeline.main.id;
    let shift = items
        .iter()
        .filter(|(id, _)| *id == main)
        .map(|(_, item)| item.start)
        .min()
        .map(|start| sub(start, placement_start(&data.scene().timeline, main, start)))
        .transpose()?
        .unwrap_or(Tick::ZERO);
    for (id, mut item) in items {
        if id == main {
            item.start = sub(item.start, shift)?;
        }
        let dest = track(data, id)?;
        unlocked(dest)?;
        dest.items.push(item);
    }
    Ok(())
}
/// Move a secondary track to a screen-order index inside its own section.
/// `index` counts from the top of the overlay or audio section; the mandatory
/// main track cannot move. Track names, order and lock state are project data.
fn reorder_track(data: &mut ProjectData, id: TrackId, index: u32) -> Result<()> {
    let timeline = &mut data.scene_mut().timeline;
    if timeline.main.id == id {
        return Err(EditError::Invalid("main track cannot be moved"));
    }
    let index = index as usize;
    if let Some(position) = timeline.overlays.iter().position(|track| track.id == id) {
        if index >= timeline.overlays.len() {
            return Err(EditError::Invalid("track index"));
        }
        let track = timeline.overlays.remove(position);
        timeline.overlays.insert(index, track);
        return Ok(());
    }
    if let Some(position) = timeline.audio.iter().position(|track| track.id == id) {
        if index >= timeline.audio.len() {
            return Err(EditError::Invalid("track index"));
        }
        let track = timeline.audio.remove(position);
        timeline.audio.insert(index, track);
        return Ok(());
    }
    Err(EditError::NoTrack(id))
}

/// Resolve a screen-order insertion without crossing the main/audio boundary.
fn add_track(
    data: &mut ProjectData,
    name: captions::NfcText,
    kind: TrackKind,
    placement: TrackPlacement,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<TrackId> {
    let audio = kind == TrackKind::Audio;
    let group = if audio {
        &data.scene().timeline.audio
    } else {
        &data.scene().timeline.overlays
    };
    let index = match placement {
        TrackPlacement::Auto => 0,
        TrackPlacement::Above { track } | TrackPlacement::Below { track } => {
            let below = matches!(placement, TrackPlacement::Below { .. });
            if track == data.scene().timeline.main.id {
                match (audio, below) {
                    (false, false) => group.len(),
                    (true, true) => 0,
                    _ => return Err(EditError::Invalid("track group boundary")),
                }
            } else {
                let reference = data
                    .scene()
                    .timeline
                    .iter()
                    .find(|t| t.id == track)
                    .ok_or(EditError::NoTrack(track))?;
                if (reference.kind == TrackKind::Audio) != audio {
                    return Err(EditError::Invalid("track group boundary"));
                }
                group
                    .iter()
                    .position(|t| t.id == track)
                    .ok_or(EditError::NoTrack(track))?
                    + usize::from(below)
            }
        }
    };
    let id = TrackId(alloc(next)?);
    let track = Track {
        id,
        name,
        kind,
        muted: false,
        hidden: false,
        locked: false,
        items: Vec::new(),
    };
    if matches!(placement, TrackPlacement::Auto) {
        data.scene_mut().timeline.insert_track(track);
    } else {
        let group = if audio {
            &mut data.scene_mut().timeline.audio
        } else {
            &mut data.scene_mut().timeline.overlays
        };
        group.insert(index, track);
    }
    created.push(CreatedId::Track(id));
    Ok(id)
}
fn placement_track(
    data: &mut ProjectData,
    placement: Placement,
    kind: TrackKind,
    start: Tick,
    duration: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<(TrackId, bool)> {
    let secondary = match placement {
        Placement::Track { track } => return Ok((track, false)),
        Placement::Above { track } => TrackPlacement::Above { track },
        Placement::Below { track } => TrackPlacement::Below { track },
        Placement::Auto => {
            let main = &data.scene().timeline.main;
            if kind == TrackKind::Video
                && !main.locked
                && (data.scene().timeline.magnet || main.items.is_empty())
            {
                return Ok((main.id, data.scene().timeline.magnet));
            }
            let end = add(start, duration)?;
            // Main first, then overlays top-to-bottom, then audio top-to-bottom.
            if let Some(track) = std::iter::once(main)
                .chain(&data.scene().timeline.overlays)
                .chain(&data.scene().timeline.audio)
                .find(|t| {
                    t.kind == kind
                        && !t.locked
                        && t.items
                            .iter()
                            .all(|item| item.end().is_ok_and(|e| e <= start || item.start >= end))
                })
            {
                return Ok((track.id, false));
            }
            TrackPlacement::Auto
        }
    };
    let id = add_track(
        data,
        format!("{kind:?}").into(),
        kind,
        secondary,
        created,
        next,
    )?;
    Ok((id, false))
}
/// Import parsed ASS cues as one caption item on a text track. The asset,
/// optional track allocation and item insertion form one command and therefore
/// one undo step. File parsing and asset probing stay with the adapter.
#[allow(clippy::too_many_arguments)]
fn import_captions(
    mut data: ProjectData,
    asset: media::SourceAsset,
    language: captions::LanguageTag,
    start: Tick,
    track: Option<TrackId>,
    name: Option<captions::NfcText>,
    cues: Vec<captions::AssCue>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    if cues.is_empty() || cues.len() > captions::ass::MAX_ASS_CUES {
        return Err(EditError::Limit);
    }
    if start < Tick::ZERO {
        return Err(EditError::Invalid("negative output start"));
    }
    let asset_id = asset.id();
    let mut end = Tick::ZERO;
    let mut converted = Vec::with_capacity(cues.len());
    for (index, cue) in cues.into_iter().enumerate() {
        if !cue.style.is_valid() || cue.range.start() < Tick::ZERO {
            return Err(EditError::Invalid("imported cue style or timing"));
        }
        end = end.max(cue.range.end());
        let id = format!("ass-{}", index.saturating_add(1)).into();
        converted.push(captions::Cue {
            id,
            range: cue.range,
            source_lang: language.clone(),
            texts: std::collections::BTreeMap::from([(
                language.clone(),
                captions::CueText {
                    text: cue.text,
                    edited: false,
                    translation_ref: None,
                },
            )]),
            style: Some(cue.style),
        });
    }
    let source_range = SourceRange::new(Tick::ZERO, end)?;

    data = apply(
        data,
        Command::AddAsset { asset },
        created,
        next,
        sources,
        copied_text_bytes,
    )?
    .into_data();
    let target = match track {
        Some(track) => {
            let value = data
                .scene()
                .timeline
                .iter()
                .find(|value| value.id == track)
                .ok_or(EditError::Invalid("import track not found"))?;
            if value.kind != TrackKind::Text {
                return Err(EditError::Invalid("import requires a text track"));
            }
            track
        }
        None => {
            let mark = created.len();
            data = apply(
                data,
                Command::AddTrack {
                    name: name.unwrap_or_else(|| "Captions".into()),
                    kind: TrackKind::Text,
                    placement: TrackPlacement::Auto,
                },
                created,
                next,
                sources,
                copied_text_bytes,
            )?
            .into_data();
            created
                .get(mark..)
                .unwrap_or_default()
                .iter()
                .find_map(|id| match id {
                    CreatedId::Track(track) => Some(*track),
                    _ => None,
                })
                .ok_or(EditError::Invalid("import track was not created"))?
        }
    };
    data = apply(
        data,
        Command::InsertItem {
            placement: Placement::Track { track: target },
            item: NewItem {
                animations: project::Animations::default(),
                source_asset: Some(asset_id),
                source_range,
                start,
                content: ItemParams::Caption(project::CaptionParams {
                    cues: captions::CueTrack { cues: converted },
                }),
            },
        },
        created,
        next,
        sources,
        copied_text_bytes,
    )?
    .into_data();
    // Per-cue styles are large: a result that cannot be encoded could not be
    // saved or re-opened, so refuse it before the session accepts it.
    project::encode(&project::Project::new(data.clone())?, false).map_err(|_| EditError::Limit)?;
    Ok(data)
}

pub(crate) fn apply(
    mut data: ProjectData,
    command: Command,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
    copied_text_bytes: &mut usize,
) -> Result<project::Project> {
    let previous_cuts = (!data.scene().timeline.transitions.is_empty()
        && !matches!(
            &command,
            Command::AddTransition { .. }
                | Command::MoveTransition { .. }
                | Command::SetTransitionDuration { .. }
                | Command::RemoveTransition { .. }
                | Command::Batch { .. }
        ))
    .then(|| crate::transitions::cuts(&data.scene().timeline));
    match command {
        Command::AddTransition {
            track,
            from,
            to,
            kind,
            duration,
            alignment,
        } => {
            project::transition_definition(&kind)?;
            if alignment == project::TransitionAlignment::Custom {
                return Err(EditError::Invalid("custom alignment requires move"));
            }
            let mut tr = project::Transition {
                id: project::TransitionId(alloc(next)?),
                track,
                from,
                to,
                kind,
                start: Tick::ZERO,
                duration: Tick::ZERO,
                alignment,
            };
            crate::transitions::unlocked(&data.scene().timeline, &tr)?;
            let bounds = tr.bounds(&data.scene().timeline, &data)?;
            tr.set_duration(duration, bounds)?;
            created.push(CreatedId::Transition(tr.id));
            data.scene_mut().timeline.transitions.push(tr);
        }
        Command::MoveTransition { transition, start } => {
            let n = crate::transitions::find(&data, transition)?;
            let mut tr = data
                .scene()
                .timeline
                .transitions
                .get(n)
                .cloned()
                .ok_or(EditError::Invalid("missing transition"))?;
            let bounds = tr.bounds(&data.scene().timeline, &data)?;
            crate::transitions::move_span(&mut tr, start, bounds)?;
            *data
                .scene_mut()
                .timeline
                .transitions
                .get_mut(n)
                .ok_or(EditError::Invalid("missing transition"))? = tr;
        }
        Command::SetTransitionDuration {
            transition,
            duration,
        } => {
            let n = crate::transitions::find(&data, transition)?;
            let mut tr = data
                .scene()
                .timeline
                .transitions
                .get(n)
                .cloned()
                .ok_or(EditError::Invalid("missing transition"))?;
            let bounds = tr.bounds(&data.scene().timeline, &data)?;
            tr.set_duration(duration, bounds)?;
            *data
                .scene_mut()
                .timeline
                .transitions
                .get_mut(n)
                .ok_or(EditError::Invalid("missing transition"))? = tr;
        }
        Command::RemoveTransition { transition } => {
            let n = crate::transitions::find(&data, transition)?;
            data.scene_mut().timeline.transitions.remove(n);
        }
        Command::CreateScene { name } => {
            let id = project::SceneId(alloc(next)?);
            let mut timeline = project::Timeline::default();
            timeline.main.id = TrackId(alloc(next)?);
            data.scenes.insert(project::Scene {
                id,
                name,
                is_main: false,
                timeline,
                bookmarks: Vec::new(),
            })?;
            created.push(CreatedId::Scene(id));
        }
        Command::DeleteScenes { scenes } => {
            if scenes.len() > 256 {
                return Err(EditError::Limit);
            }
            let count = scenes.len();
            let ids: BTreeSet<_> = scenes.into_iter().collect();
            if ids.len() != count {
                return Err(EditError::Invalid("duplicate scene ID"));
            }
            data.scenes.delete(&ids)?;
            if ids.contains(&data.current_scene_id) {
                data.current_scene_id = data.scenes.main().id;
            }
        }
        Command::RenameScene { scene, name } => {
            data.scenes
                .get_mut(scene)
                .ok_or(EditError::Invalid("scene not found"))?
                .name = name;
        }
        Command::RenameProject { name } => data.name = name,
        Command::AddAsset { asset } => {
            ratchet_fps(&mut data.canvas, &asset);
            data.assets.push(asset);
        }
        Command::RemoveAssets { assets } => remove_assets(&mut data, assets)?,
        Command::AddTrack {
            name,
            kind,
            placement,
        } => {
            add_track(&mut data, name, kind, placement, created, next)?;
        }
        Command::RemoveTrack { track: id } => {
            if data.scene_mut().timeline.main.id == id {
                return Err(EditError::Invalid("main track cannot be removed"));
            }
            unlocked(track(&mut data, id)?)?;
            data.scene_mut().timeline.overlays.retain(|t| t.id != id);
            data.scene_mut().timeline.audio.retain(|t| t.id != id);
        }
        Command::SetMaster { volume, muted } => {
            if !project::valid_volume(volume) {
                return Err(EditError::Invalid("master volume"));
            }
            data.settings.master_volume = volume;
            data.settings.master_muted = muted;
        }
        Command::SetFade {
            item: id,
            edge,
            duration,
            curve,
        } => {
            let target = item(&mut data, id)?;
            let clip = target.source_range.duration();
            let Some(fades) = fades_mut(&mut target.content) else {
                return Err(EditError::Invalid("fade needs an audio or video item"));
            };
            fades.set(edge, duration, curve, clip)?;
        }
        Command::SetTrackMuted { track: id, muted } => {
            let t = track(&mut data, id)?;
            if !t.kind.can_mute() {
                return Err(EditError::Invalid("track has no audio"));
            }
            t.muted = muted;
        }
        Command::SetTrackHidden { track: id, hidden } => {
            let t = track(&mut data, id)?;
            if !t.kind.can_hide() {
                return Err(EditError::Invalid("audio track has no video"));
            }
            t.hidden = hidden;
        }
        Command::SetTrackMagnet { enabled } => data.scene_mut().timeline.magnet = enabled,
        Command::SetTrackFlags {
            track: id,
            muted,
            hidden,
            locked,
        } => {
            let t = track(&mut data, id)?;
            t.muted = muted;
            t.hidden = hidden;
            t.locked = locked;
        }
        Command::SetTrackLocked { track: id, locked } => {
            track(&mut data, id)?.locked = locked;
        }
        Command::RenameTrack { track: id, name } => {
            // Renaming is project metadata, not an item structural edit; a
            // locked track can still be labelled (and unlocked) here.
            track(&mut data, id)?.name = name;
        }
        Command::ReorderTrack { track: id, index } => {
            reorder_track(&mut data, id, index)?;
        }
        Command::ImportCaptions {
            asset,
            language,
            start,
            track,
            name,
            cues,
        } => {
            data = import_captions(
                data,
                asset,
                language,
                start,
                track,
                name,
                cues,
                created,
                next,
                sources,
                copied_text_bytes,
            )?;
        }
        // Session commits this as one grouped transaction before reaching edit.
        Command::AddLibrarySound { .. } => {
            return Err(EditError::Invalid("library sound requires a session"));
        }
        Command::InsertItem {
            placement,
            item: new,
        } => {
            if new.start < Tick::ZERO {
                return Err(EditError::Invalid("negative output start"));
            }
            if matches!(new.content, ItemParams::Video(_))
                && let Some(asset) = new
                    .source_asset
                    .and_then(|id| data.assets.iter().find(|a| a.id() == id))
            {
                ratchet_fps(&mut data.canvas, asset);
                if data.original_canvas_size.is_none()
                    && data
                        .scenes
                        .iter()
                        .all(|scene| scene.timeline.iter().all(|track| track.items.is_empty()))
                    && let Some(video) = asset
                        .probe()
                        .streams
                        .iter()
                        .find_map(|stream| stream.video.as_ref())
                {
                    let size = project::CanvasSize {
                        width: video.width,
                        height: video.height,
                    };
                    data.canvas.width = size.width;
                    data.canvas.height = size.height;
                    data.original_canvas_size = Some(size);
                }
            }
            let kind = new.content.kind();
            let (target, magnetic) = placement_track(
                &mut data,
                placement,
                kind,
                new.start,
                new.source_range.duration(),
                created,
                next,
            )?;
            let start = placement_start(&data.scene().timeline, target, new.start);
            let t = track(&mut data, target)?;
            unlocked(t)?;
            if t.kind != kind {
                return Err(EditError::Invalid("incompatible track"));
            }
            let id = ItemId(alloc(next)?);
            let index = if magnetic {
                // Choose the nearest edge of the clip under the pointer; ties go before.
                let mut index = t.items.len();
                for (i, item) in t.items.iter().enumerate() {
                    let end = item.end()?;
                    if start < end {
                        index =
                            if start <= item.start || sub(start, item.start)? <= sub(end, start)? {
                                i
                            } else {
                                i + 1
                            };
                        break;
                    }
                }
                index
            } else {
                t.items.partition_point(|item| item.start <= start)
            };
            t.items.insert(
                index,
                Item {
                    blend_mode: Default::default(),
                    masks: Vec::new(),
                    effects: Vec::new(),
                    animations: new.animations,
                    id,
                    source_asset: new.source_asset,
                    source_range: new.source_range,
                    start,
                    content: new.content,
                },
            );
            if magnetic {
                let mut at = Tick::ZERO;
                for item in &mut t.items {
                    item.start = at;
                    at = item.end()?;
                }
            }
            created.push(CreatedId::Item(id));
        }
        Command::MoveItem { item, track, start } => {
            move_items(&mut data, vec![Move { item, track, start }])?
        }
        Command::MoveItems { moves } => move_items(&mut data, moves)?,
        Command::SplitAt { item: id, at } => {
            let left = item(&mut data, id)?;
            if at <= left.start || at >= left.end()? {
                return Err(EditError::Invalid("split must be inside item"));
            }
            let cut = add(left.source_range.start(), sub(at, left.start)?)?;
            let (start, end_tick) = (left.start, left.end()?);
            let mut right = left.clone();
            let end = left.source_range.end();
            set_range(
                left,
                SourceRange::new(left.source_range.start(), cut)?,
                sources,
            )?;
            set_range(&mut right, SourceRange::new(cut, end)?, sources)?;
            // A split establishes independent retained content for both new pieces.
            if let ItemParams::Caption(p) = &left.content {
                sources.insert(left.id, Arc::new(p.cues.clone()));
            }
            // The cut is non-destructive: both pieces keep the whole shape of any fade reaching
            // them, so together they replay the original gain at every sample.
            if let (Some(l), Some(r)) =
                (fades_mut(&mut left.content), fades_mut(&mut right.content))
            {
                (*l, *r) = l.split(start, at, end_tick)?;
            }
            right.id = ItemId(alloc(next)?);
            right.start = at;
            if let ItemParams::Caption(p) = &right.content {
                sources.insert(right.id, Arc::new(p.cues.clone()));
            }
            created.push(CreatedId::Item(right.id));
            let t = data
                .scene_mut()
                .timeline
                .iter_mut()
                .find(|t| t.items.iter().any(|i| i.id == id))
                .ok_or(EditError::NoItem(id))?;
            let right_id = right.id;
            t.items.push(right);
            for tr in &mut data.scene_mut().timeline.transitions {
                if tr.from == id {
                    tr.from = right_id;
                }
            }
        }
        Command::TrimStart { item: id, at } => {
            let item = item(&mut data, id)?;
            let moved = item.start != at;
            let source = add(item.source_range.start(), sub(at, item.start)?)?;
            let range = SourceRange::new(source, item.source_range.end())?;
            set_range(item, range, sources)?;
            item.start = at;
            // A trimmed edge starts a fresh fade of the audible length, yielding to the other.
            let clip = item.source_range.duration();
            if moved && let Some(fades) = fades_mut(&mut item.content) {
                let curve = fades.fade_in.curve;
                let window = fades.fade_in.window();
                fades.set(project::FadeEdge::In, window, curve, clip)?;
            }
        }
        Command::TrimEnd { item: id, at } => {
            let item = item(&mut data, id)?;
            let moved = item.end()? != at;
            let end = add(item.source_range.start(), sub(at, item.start)?)?;
            set_range(
                item,
                SourceRange::new(item.source_range.start(), end)?,
                sources,
            )?;
            let clip = item.source_range.duration();
            if moved && let Some(fades) = fades_mut(&mut item.content) {
                let curve = fades.fade_out.curve;
                let window = fades.fade_out.window();
                fades.set(project::FadeEdge::Out, window, curve, clip)?;
            }
        }
        Command::Delete { items } => {
            let mut ids = BTreeSet::new();
            for id in items {
                if !ids.insert(id) {
                    return Err(EditError::Invalid("duplicate delete item"));
                }
                take_item(&mut data, id)?;
            }
        }
        Command::RippleDelete { item: id } => {
            let t = data
                .scene_mut()
                .timeline
                .iter_mut()
                .find(|t| t.items.iter().any(|i| i.id == id))
                .ok_or(EditError::NoItem(id))?;
            unlocked(t)?;
            let index = t
                .items
                .iter()
                .position(|i| i.id == id)
                .ok_or(EditError::NoItem(id))?;
            let removed = t.items.remove(index);
            let end = removed.end()?;
            for item in &mut t.items {
                if item.start >= end {
                    item.start = sub(item.start, removed.source_range.duration())?;
                }
            }
        }
        Command::SetItemParam { item: id, content } => {
            item(&mut data, id)?.content = content;
            // Explicit content replacement becomes authoritative for later previews.
            sources.remove(&id);
        }
        Command::SetItemField { item: id, patch } => {
            patch.validate()?;
            let target = item(&mut data, id)?;
            if !patch.compatible(&target.content) {
                return Err(EditError::Invalid("incompatible item parameter"));
            }
            apply_patch(target, &patch, copied_text_bytes)?;
        }
        Command::SetCanvasBackground { color } => data.canvas.background = color,
        Command::SetItemParamMany { items, patch } => {
            patch.validate()?;
            if items.is_empty() {
                return Err(EditError::Invalid("empty item selection"));
            }
            let mut remaining: BTreeSet<_> = items.iter().copied().collect();
            if remaining.len() != items.len() {
                return Err(EditError::Invalid("duplicate item selection"));
            }
            // Walk the candidate once: large selections must not rescan the timeline per ID.
            for track in data.scene_mut().timeline.iter_mut() {
                for target in &mut track.items {
                    if !remaining.remove(&target.id) || !patch.compatible(&target.content) {
                        continue;
                    }
                    if track.locked {
                        return Err(EditError::Locked(track.id));
                    }
                    apply_patch(target, &patch, copied_text_bytes)?;
                }
            }
            if let Some(id) = remaining.into_iter().next() {
                return Err(EditError::NoItem(id));
            }
        }
        Command::AddMask { item: id, mask } => {
            item(&mut data, id)?.masks.push(mask);
        }
        Command::RemoveMask { item: id, mask } => {
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
        }
        Command::UpdateMask { item: id, mask } => {
            let target = item(&mut data, id)?;
            let previous = target
                .masks
                .iter_mut()
                .find(|m| m.id == mask.id)
                .ok_or(EditError::Invalid("missing mask"))?;
            *previous = mask;
        }
        Command::ReorderMasks { item: id, order } => {
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
        }
        Command::AddEffect { item: id, effect } => {
            let item = effect_item(&mut data, id)?;
            item.effects.push(effect);
        }
        Command::RemoveEffect { item: id, effect } => {
            let item = effect_item(&mut data, id)?;
            let index = item
                .effects
                .iter()
                .position(|e| e.id == effect)
                .ok_or(EditError::Invalid("missing effect"))?;
            item.effects.remove(index);
            let prefix = format!("effects.{effect}.params.");
            let paths: Vec<_> = item
                .animations
                .iter()
                .filter(|(p, _)| p.as_str().starts_with(&prefix))
                .map(|(p, _)| p.clone())
                .collect();
            for path in paths {
                item.animations.remove(&path);
            }
        }
        Command::UpdateEffectParam {
            item: id,
            effect,
            key,
            value,
        } => {
            let target = effect_item(&mut data, id)?;
            let effect = target
                .effects
                .iter_mut()
                .find(|e| e.id == effect)
                .ok_or(EditError::Invalid("missing effect"))?;
            effect.params.set_scalar(&key, value)?;
        }
        Command::ToggleEffect {
            item: id,
            effect,
            enabled,
        } => {
            let target = effect_item(&mut data, id)?;
            target
                .effects
                .iter_mut()
                .find(|e| e.id == effect)
                .ok_or(EditError::Invalid("missing effect"))?
                .enabled = enabled;
        }
        Command::ReorderEffects { item: id, order } => {
            let target = effect_item(&mut data, id)?;
            if order.len() != target.effects.len() {
                return Err(EditError::Invalid("effect order length"));
            }
            let mut reordered = Vec::with_capacity(order.len());
            for id in order {
                let index = target
                    .effects
                    .iter()
                    .position(|e| e.id == id)
                    .ok_or(EditError::Invalid("effect order identity"))?;
                reordered.push(target.effects.remove(index));
            }
            target.effects = reordered;
        }
        Command::SetAnimatedParam {
            item: id,
            path,
            param,
        } => {
            item(&mut data, id)?.animations.set(path, param)?;
        }
        Command::AddKeyframe {
            item: id,
            path,
            component,
            key,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .put(key)?;
        }
        Command::RemoveKeyframe {
            item: id,
            path,
            component,
            time,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .remove(time)?;
        }
        Command::RetimeKeyframe {
            item: id,
            path,
            component,
            from,
            to,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .retime_key(from, to)?;
        }
        Command::SetKeyframeHandle {
            item: id,
            path,
            component,
            time,
            side,
            handle,
            tangent,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .set_handle(time, side, handle, tangent)?;
        }
        Command::ScaleKeyframeTime {
            item: id,
            numerator,
            denominator,
        } => {
            item(&mut data, id)?
                .animations
                .retime(numerator, denominator)?;
        }
        Command::AddBookmark {
            start,
            duration,
            note,
            color,
        } => {
            let id = BookmarkId(alloc(next)?);
            data.scene_mut().bookmarks.push(Bookmark {
                id,
                start,
                duration,
                note,
                color,
            });
            created.push(CreatedId::Bookmark(id));
        }
        Command::MoveBookmark { bookmark, start } => {
            if start < Tick::ZERO {
                return Err(EditError::Invalid("bookmark start"));
            }
            let candidate = data
                .scene_mut()
                .bookmarks
                .iter_mut()
                .find(|candidate| candidate.id == bookmark)
                .ok_or(EditError::Invalid("missing bookmark"))?;
            if start.checked_add(candidate.duration).is_none() {
                return Err(EditError::Invalid("bookmark end"));
            }
            candidate.start = start;
        }
        Command::Retranslate {
            item: id,
            target_language,
            texts,
            translation_ref,
            force,
        } => {
            if texts.is_empty()
                || texts.len() > ai::MAX_SEGMENTS
                || translation_ref.as_str().trim().is_empty()
                || translation_ref.as_str().len() > 256
            {
                return Err(EditError::Invalid("translation application"));
            }
            let item = item(&mut data, id)?;
            let ItemParams::Caption(params) = &mut item.content else {
                return Err(EditError::Invalid("translation requires caption item"));
            };
            if texts
                .keys()
                .any(|id| !params.cues.cues.iter().any(|c| &c.id == id))
                || texts.values().any(|t| t.as_str().trim().is_empty())
            {
                return Err(EditError::Invalid("translation cue ids or text"));
            }
            // A selected text edit does not replace the full caption content.
            // Update the retained track, then let the mapping owner rebuild the
            // clipped view so reversal can restore hidden cues and original ranges.
            let retained = sources.contains_key(&id);
            let cues = match sources.get_mut(&id) {
                Some(source) => Arc::make_mut(source),
                None => &mut params.cues,
            };
            if texts
                .keys()
                .any(|id| !cues.cues.iter().any(|cue| &cue.id == id))
            {
                return Err(EditError::Invalid("missing retained translation cue"));
            }
            for cue in &mut cues.cues {
                if let Some(text) = texts.get(&cue.id) {
                    if cue.source_lang == target_language {
                        return Err(EditError::Invalid("cannot replace source with translation"));
                    }
                    cue.set_text(
                        target_language.clone(),
                        captions::CueText {
                            text: text.clone(),
                            edited: false,
                            translation_ref: Some(translation_ref.clone()),
                        },
                        force,
                    );
                }
            }
            if retained {
                set_range(item, item.source_range, sources)?;
            }
        }
        Command::Batch { commands } => {
            for command in commands {
                // Consume each candidate before applying the next child; validation must
                // not clone an unvalidated allocation or publish a partial batch.
                data = apply(data, command, created, next, sources, copied_text_bytes)?.into_data();
            }
        }
        Command::Inspect {}
        | Command::Preview { .. }
        | Command::Snapshot { .. }
        | Command::Export { .. }
        | Command::JobStatus { .. }
        | Command::JobCancel { .. }
        | Command::Catalog {}
        | Command::ProviderCatalog {}
        | Command::FontsList { .. }
        | Command::FontsLoad { .. }
        | Command::FontsDownload { .. }
        | Command::ExportSubtitles { .. }
        | Command::BurnSubtitles { .. } => {
            return Err(EditError::Invalid("export commands require job submission"));
        }
        Command::DuplicateProject { .. }
        | Command::DeleteProject {}
        | Command::ThumbnailProject {}
        | Command::CreateProject { .. }
        | Command::SaveProject {}
        | Command::SetViewState { .. }
        | Command::SwitchScene { .. }
        | Command::Undo {}
        | Command::Redo {}
        | Command::BeginGesture {}
        | Command::CommitGesture {}
        | Command::CancelGesture {} => {
            return Err(EditError::Invalid("session control inside batch"));
        }
    }
    check_timeline(&mut data)?;
    if let Some(previous) = previous_cuts {
        crate::transitions::revalidate(&mut data, &previous);
    }
    Ok(project::Project::new(data)?)
}
fn check_timeline(data: &mut ProjectData) -> Result<()> {
    for track in data.scene_mut().timeline.iter_mut() {
        track.items.sort_by_key(|i| i.start);
        let mut end = Tick::ZERO;
        for item in &track.items {
            if item.start < Tick::ZERO {
                return Err(EditError::Invalid("negative output start"));
            }
            if item.start < end {
                return Err(EditError::Overlap(track.id));
            }
            end = item.end()?;
        }
    }
    Ok(())
}
