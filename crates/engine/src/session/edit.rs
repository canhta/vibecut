//! Applies validated editing commands to project candidates.
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

pub(crate) fn check_budget(command: &Command) -> Result<()> {
    let mut stack = vec![(command, 0)];
    let mut count = 0;
    while let Some((command, depth)) = stack.pop() {
        count += 1;
        if count > super::wire::MAX_BATCH_COMMANDS || depth > super::wire::MAX_BATCH_DEPTH {
            return Err(EditError::Limit);
        }
        match command {
            Command::Batch { commands } => {
                if commands.len() > super::wire::MAX_BATCH_COMMANDS {
                    return Err(EditError::Limit);
                }
                stack.extend(commands.iter().map(|c| (c, depth + 1)));
            }
            Command::MoveItems { moves } if moves.len() > super::wire::MAX_COLLECTION => {
                return Err(EditError::Limit);
            }
            Command::Delete { items }
            | Command::SetItemParamMany { items, .. }
            | Command::SetSpeed { items, .. }
                if items.len() > super::wire::MAX_COLLECTION =>
            {
                return Err(EditError::Limit);
            }
            Command::RemoveAssets { assets } if assets.len() > super::wire::MAX_COLLECTION => {
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
use media_edits::ratchet_fps;

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
use audio::fades_mut;

use clips::set_range;

use tracks::placement_track;

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
            data = transitions::execute_add_transition(
                data, track, from, to, kind, duration, alignment, created, next,
            )?;
        }
        Command::MoveTransition { transition, start } => {
            data = transitions::execute_move_transition(data, transition, start)?;
        }
        Command::SetTransitionDuration {
            transition,
            duration,
        } => {
            data = transitions::execute_set_transition_duration(data, transition, duration)?;
        }
        Command::RemoveTransition { transition } => {
            data = transitions::execute_remove_transition(data, transition)?;
        }
        Command::CreateScene { name } => {
            data = project_edits::execute_create_scene(data, name, created, next)?;
        }
        Command::DeleteScenes { scenes } => {
            data = project_edits::execute_delete_scenes(data, scenes)?;
        }
        Command::RenameScene { scene, name } => {
            data = project_edits::execute_rename_scene(data, scene, name)?;
        }
        Command::RenameProject { name } => {
            data = project_edits::execute_rename_project(data, name)?;
        }
        Command::AddAsset { asset } => {
            data = media_edits::execute_add_asset(data, asset)?;
        }
        Command::RemoveAssets { assets } => {
            data = media_edits::execute_remove_assets(data, assets)?;
        }
        Command::AddTrack {
            name,
            kind,
            placement,
        } => {
            data = tracks::execute_add_track(data, name, kind, placement, created, next)?;
        }
        Command::RemoveTrack { track: id } => {
            data = tracks::execute_remove_track(data, id)?;
        }
        Command::SetMaster { volume, muted } => {
            data = audio::execute_set_master(data, volume, muted)?;
        }
        Command::SetFade {
            item: id,
            edge,
            duration,
            curve,
        } => {
            data = audio::execute_set_fade(data, id, edge, duration, curve)?;
        }
        Command::SetLoudness {
            scope,
            target_lufs,
            true_peak_db,
        } => {
            data = audio::execute_set_loudness(data, scope, target_lufs, true_peak_db)?;
        }
        Command::ClearLoudness { scope } => {
            data = audio::execute_clear_loudness(data, scope)?;
        }
        Command::ApplyLoudness { gains } => {
            data = audio::execute_apply_loudness(data, gains)?;
        }
        Command::SetTrackMuted { track: id, muted } => {
            data = tracks::execute_set_track_muted(data, id, muted)?;
        }
        Command::SetTrackHidden { track: id, hidden } => {
            data = tracks::execute_set_track_hidden(data, id, hidden)?;
        }
        Command::SetTrackMagnet { enabled } => {
            data = tracks::execute_set_track_magnet(data, enabled)?;
        }
        Command::SetTrackFlags {
            track: id,
            muted,
            hidden,
            locked,
        } => {
            data = tracks::execute_set_track_flags(data, id, muted, hidden, locked)?;
        }
        Command::SetTrackLocked { track: id, locked } => {
            data = tracks::execute_set_track_locked(data, id, locked)?;
        }
        Command::RenameTrack { track: id, name } => {
            data = tracks::execute_rename_track(data, id, name)?;
        }
        Command::ReorderTrack { track: id, index } => {
            data = tracks::execute_reorder_track(data, id, index)?;
        }
        Command::ImportCaptions {
            asset,
            language,
            start,
            track,
            name,
            cues,
        } => {
            data = text::execute_import_captions(
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
            data = clips::execute_insert_item(data, placement, new, created, next)?;
        }
        Command::MoveItem { item, track, start } => {
            data = clips::execute_move_item(data, item, track, start)?;
        }
        Command::MoveItems { moves } => {
            data = clips::execute_move_items(data, moves)?;
        }
        Command::SplitAt { item: id, at } => {
            data = clips::execute_split_at(data, id, at, created, next, sources)?;
        }
        Command::TrimStart { item: id, at } => {
            data = clips::execute_trim_start(data, id, at, sources)?;
        }
        Command::TrimEnd { item: id, at } => {
            data = clips::execute_trim_end(data, id, at, sources)?;
        }
        Command::Delete { items } => {
            data = clips::execute_delete(data, items)?;
        }
        Command::RippleDelete { item: id } => {
            data = clips::execute_ripple_delete(data, id)?;
        }
        Command::SetItemParam { item: id, content } => {
            data = parameters::execute_set_item_param(data, id, content, sources)?;
        }
        Command::SetItemField { item: id, patch } => {
            data = parameters::execute_set_item_field(data, id, patch, copied_text_bytes)?;
        }
        Command::SetCanvasBackground { color } => {
            data = project_edits::execute_set_canvas_background(data, color)?;
        }
        Command::SetSpeed {
            items,
            speed,
            maintain_pitch,
        } => {
            data = retime::execute_set_speed(data, items, speed, maintain_pitch)?;
        }
        Command::SetItemParamMany { items, patch } => {
            data = parameters::execute_set_item_param_many(data, items, patch, copied_text_bytes)?;
        }
        Command::AddMask { item: id, mask } => {
            data = masks::execute_add_mask(data, id, mask)?;
        }
        Command::RemoveMask { item: id, mask } => {
            data = masks::execute_remove_mask(data, id, mask)?;
        }
        Command::UpdateMask { item: id, mask } => {
            data = masks::execute_update_mask(data, id, mask)?;
        }
        Command::ReorderMasks { item: id, order } => {
            data = masks::execute_reorder_masks(data, id, order)?;
        }
        Command::AddEffect { item: id, effect } => {
            data = effects::execute_add_effect(data, id, effect)?;
        }
        Command::RemoveEffect { item: id, effect } => {
            data = effects::execute_remove_effect(data, id, effect)?;
        }
        Command::UpdateEffectParam {
            item: id,
            effect,
            key,
            value,
        } => {
            data = effects::execute_update_effect_param(data, id, effect, key, value)?;
        }
        Command::ToggleEffect {
            item: id,
            effect,
            enabled,
        } => {
            data = effects::execute_toggle_effect(data, id, effect, enabled)?;
        }
        Command::ReorderEffects { item: id, order } => {
            data = effects::execute_reorder_effects(data, id, order)?;
        }
        Command::SetAnimatedParam {
            item: id,
            path,
            param,
        } => {
            data = keyframes::execute_set_animated_param(data, id, path, param)?;
        }
        Command::AddKeyframe {
            item: id,
            path,
            component,
            key,
        } => {
            data = keyframes::execute_add_keyframe(data, id, path, component, key)?;
        }
        Command::RemoveKeyframe {
            item: id,
            path,
            component,
            time,
        } => {
            data = keyframes::execute_remove_keyframe(data, id, path, component, time)?;
        }
        Command::RetimeKeyframe {
            item: id,
            path,
            component,
            from,
            to,
        } => {
            data = keyframes::execute_retime_keyframe(data, id, path, component, from, to)?;
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
            data = keyframes::execute_set_keyframe_handle(
                data, id, path, component, time, side, handle, tangent,
            )?;
        }
        Command::ScaleKeyframeTime {
            item: id,
            numerator,
            denominator,
        } => {
            data = keyframes::execute_scale_keyframe_time(data, id, numerator, denominator)?;
        }
        Command::AddBookmark {
            start,
            duration,
            note,
            color,
        } => {
            data = project_edits::execute_add_bookmark(
                data, start, duration, note, color, created, next,
            )?;
        }
        Command::MoveBookmark { bookmark, start } => {
            data = project_edits::execute_move_bookmark(data, bookmark, start)?;
        }
        Command::Retranslate {
            item: id,
            target_language,
            texts,
            translation_ref,
            force,
        } => {
            data = text::execute_retranslate(
                data,
                id,
                target_language,
                texts,
                translation_ref,
                force,
                sources,
            )?;
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
mod audio;
mod clips;
mod effects;
mod keyframes;
mod masks;
mod media_edits;
mod parameters;
mod project_edits;
mod retime;
mod text;
mod tracks;
mod transitions;
