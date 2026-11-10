//! Applies validated editing commands to project candidates.
use super::EditCommand;
use crate::{
    AssetUsage, Command, CreatedId, EditError, Effect, Move, NewItem, Placement, Result,
    TrackPlacement,
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
            Command::TimelineEdit { .. } if depth != 0 => {
                return Err(EditError::Invalid("nested ripple policy"));
            }
            Command::Batch { commands } | Command::TimelineEdit { commands, .. } => {
                if commands.len() > super::wire::MAX_BATCH_COMMANDS {
                    return Err(EditError::Limit);
                }
                stack.extend(commands.iter().map(|c| (c, depth + 1)));
            }
            Command::MoveItems { moves } if moves.len() > super::wire::MAX_COLLECTION => {
                return Err(EditError::Limit);
            }
            Command::TimelineMoveItems { items, .. }
            | Command::TimelineTrim { items, .. }
            | Command::TimelineSplitLeft { items, .. }
            | Command::TimelineSplitRight { items, .. }
            | Command::DuplicateItems { items }
            | Command::SetItemsHidden { items, .. }
            | Command::SetItemsMuted { items, .. }
            | Command::ExtractAudio { items }
            | Command::RecoverAudio { items }
            | Command::Delete { items }
            | Command::SetItemParamMany { items, .. }
            | Command::SetSpeed { items, .. }
                if items.len() > super::wire::MAX_COLLECTION =>
            {
                return Err(EditError::Limit);
            }
            Command::TranscriptDeleteRange { word_ids }
                if word_ids.len() > super::wire::MAX_COLLECTION =>
            {
                return Err(EditError::Limit);
            }
            Command::TranscriptMergeCues { cue_ids }
                if cue_ids.len() > super::wire::MAX_COLLECTION =>
            {
                return Err(EditError::Limit);
            }
            Command::RemoveFillers { ids, .. } if ids.len() > super::wire::MAX_COLLECTION => {
                return Err(EditError::Limit);
            }
            Command::RemoveAssets { assets } if assets.len() > super::wire::MAX_COLLECTION => {
                return Err(EditError::Limit);
            }
            other => outside_edit(other)?,
        }
    }
    Ok(())
}
/// Only plain edits run inside a batch or a candidate; the declaration's effect says which.
fn outside_edit(command: &Command) -> Result<()> {
    match command.effect() {
        Effect::Edit => Ok(()),
        Effect::SessionControl => Err(EditError::Invalid("session control inside batch")),
        Effect::Read | Effect::FilePublish | Effect::JobControl => {
            Err(EditError::Invalid("export commands require job submission"))
        }
    }
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
                | Command::SetTransitionParam { .. }
                | Command::RemoveTransition { .. }
                | Command::Batch { .. }
        ))
    .then(|| crate::transitions::cuts(&data.scene().timeline));
    let command = match command.into_edit() {
        Ok(command) => command,
        Err(command) => {
            outside_edit(&command)?;
            return Err(EditError::Invalid("command is not an edit"));
        }
    };
    match command {
        EditCommand::AddTransition {
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
        EditCommand::MoveTransition { transition, start } => {
            data = transitions::execute_move_transition(data, transition, start)?;
        }
        EditCommand::SetTransitionDuration {
            transition,
            duration,
        } => {
            data = transitions::execute_set_transition_duration(data, transition, duration)?;
        }
        EditCommand::SetTransitionParam {
            transition,
            param,
            value,
        } => {
            data = transitions::execute_set_transition_param(data, transition, &param, value)?;
        }
        EditCommand::RemoveTransition { transition } => {
            data = transitions::execute_remove_transition(data, transition)?;
        }
        EditCommand::CreateScene { name } => {
            data = project_edits::execute_create_scene(data, name, created, next)?;
        }
        EditCommand::DeleteScenes { scenes } => {
            data = project_edits::execute_delete_scenes(data, scenes)?;
        }
        EditCommand::RenameScene { scene, name } => {
            data = project_edits::execute_rename_scene(data, scene, name)?;
        }
        EditCommand::RenameProject { name } => {
            data = project_edits::execute_rename_project(data, name)?;
        }
        EditCommand::AddAsset { asset } => {
            data = media_edits::execute_add_asset(data, asset)?;
        }
        EditCommand::RemoveAssets { assets } => {
            data = media_edits::execute_remove_assets(data, assets)?;
        }
        EditCommand::AddTrack {
            name,
            kind,
            placement,
        } => {
            data = tracks::execute_add_track(data, name, kind, placement, created, next)?;
        }
        EditCommand::RemoveTrack { track: id } => {
            data = tracks::execute_remove_track(data, id)?;
        }
        EditCommand::SetMaster { volume, muted } => {
            data = audio::execute_set_master(data, volume, muted)?;
        }
        EditCommand::SetFade {
            item: id,
            edge,
            duration,
            curve,
        } => {
            data = audio::execute_set_fade(data, id, edge, duration, curve)?;
        }
        EditCommand::SetLoudness {
            scope,
            target_lufs,
            true_peak_db,
        } => {
            data = audio::execute_set_loudness(data, scope, target_lufs, true_peak_db)?;
        }
        EditCommand::ClearLoudness { scope } => {
            data = audio::execute_clear_loudness(data, scope)?;
        }
        EditCommand::ApplyLoudness { gains } => {
            data = audio::execute_apply_loudness(data, gains)?;
        }
        EditCommand::SetTrackMuted { track: id, muted } => {
            data = tracks::execute_set_track_muted(data, id, muted)?;
        }
        EditCommand::SetTrackHidden { track: id, hidden } => {
            data = tracks::execute_set_track_hidden(data, id, hidden)?;
        }
        EditCommand::SetTrackMagnet { enabled } => {
            data = tracks::execute_set_track_magnet(data, enabled)?;
        }
        EditCommand::SetTrackFlags {
            track: id,
            muted,
            hidden,
            locked,
        } => {
            data = tracks::execute_set_track_flags(data, id, muted, hidden, locked)?;
        }
        EditCommand::SetTrackLocked { track: id, locked } => {
            data = tracks::execute_set_track_locked(data, id, locked)?;
        }
        EditCommand::RenameTrack { track: id, name } => {
            data = tracks::execute_rename_track(data, id, name)?;
        }
        EditCommand::ReorderTrack { track: id, index } => {
            data = tracks::execute_reorder_track(data, id, index)?;
        }
        EditCommand::ImportCaptions {
            asset,
            language,
            start,
            track,
            name,
            styles,
            cues,
        } => {
            data = text::execute_import_captions(
                data,
                asset,
                language,
                start,
                track,
                name,
                styles,
                cues,
                created,
                next,
                sources,
                copied_text_bytes,
            )?;
        }
        // Session commits this as one grouped transaction before reaching edit.
        EditCommand::AddLibrarySound {
            placement: _placement,
            start: _start,
            asset: _asset,
            sound: _sound,
        } => {
            return Err(EditError::Invalid("library sound requires a session"));
        }
        EditCommand::InsertItem {
            placement,
            item: new,
        } => {
            data = clips::execute_insert_item(data, placement, new, created, next)?;
        }
        EditCommand::MoveItem { item, track, start } => {
            data = clips::execute_move_item(data, item, track, start)?;
        }
        EditCommand::MoveItems { moves } => {
            data = clips::execute_move_items(data, moves)?;
        }
        EditCommand::TimelineMoveItems {
            items,
            delta,
            placement,
        } => {
            data = clips::move_group(data, items, delta, placement, created, next)?;
        }
        EditCommand::TimelineTrim { items, edge, delta } => {
            data = clips::trim_group(data, items, edge, delta, sources)?;
        }
        EditCommand::RemovePauses {
            spans,
            crossfade_ms,
        } => {
            let spans = spans
                .iter()
                .map(|span| SourceRange::new(span.start, span.end).map_err(EditError::from))
                .collect::<Result<Vec<_>>>()?;
            data = clips::remove_output_ranges(
                data,
                &spans,
                crossfade_ms,
                clips::WordCut::Clear,
                created,
                next,
                sources,
            )?;
        }
        EditCommand::TranscriptDeleteRange { word_ids } => {
            data = text::execute_transcript_delete(data, word_ids, created, next, sources)?;
        }
        EditCommand::TranscriptMergeCues { cue_ids } => {
            data = text::execute_transcript_merge(data, cue_ids, sources)?;
        }
        EditCommand::RemoveFillers {
            ids,
            confirm_edited,
        } => {
            data = text::execute_remove_fillers(data, ids, confirm_edited, created, next, sources)?;
        }
        EditCommand::SplitAt { item: id, at } => {
            data = clips::execute_split_at(data, id, at, created, next, sources)?;
        }
        EditCommand::TimelineSplitLeft { items, at } => {
            data = clips::execute_split_left(data, items, at, created, next, sources)?;
        }
        EditCommand::TimelineSplitRight { items, at } => {
            data = clips::execute_split_right(data, items, at, created, next, sources)?;
        }
        EditCommand::DuplicateItems { items } => {
            data = clips::execute_duplicate(data, items, created, next)?;
        }
        EditCommand::SetItemsHidden { items, hidden } => {
            data = clips::execute_set_items_hidden(data, items, hidden)?;
        }
        EditCommand::SetItemsMuted { items, muted } => {
            data = clips::execute_set_items_muted(data, items, muted)?;
        }
        EditCommand::ExtractAudio { items } => {
            data = clips::execute_extract_audio(data, items, created, next)?;
        }
        EditCommand::RecoverAudio { items } => {
            data = clips::execute_recover_audio(data, items)?;
        }
        EditCommand::TrimStart { item: id, at } => {
            data = clips::execute_trim_start(data, id, at, sources)?;
        }
        EditCommand::TrimEnd { item: id, at } => {
            data = clips::execute_trim_end(data, id, at, sources)?;
        }
        EditCommand::Delete { items } => {
            data = clips::execute_delete(data, items)?;
        }
        EditCommand::RippleDelete { item: id } => {
            data = clips::execute_ripple_delete(data, id)?;
        }
        EditCommand::SetItemParam { item: id, content } => {
            data = parameters::execute_set_item_param(data, id, content, sources)?;
        }
        EditCommand::SetItemField { item: id, patch } => {
            data = parameters::execute_set_item_field(data, id, patch, copied_text_bytes)?;
        }
        EditCommand::SetCanvas {
            width,
            height,
            preset,
        } => {
            data = project_edits::execute_set_canvas(data, width, height, preset)?;
        }
        EditCommand::SetFps { num, den } => {
            data = project_edits::execute_set_fps(data, num, den)?;
        }
        EditCommand::SetBackground { color } => {
            data = project_edits::execute_set_background(data, color)?;
        }
        EditCommand::SetSpeed {
            items,
            speed,
            maintain_pitch,
        } => {
            data = retime::execute_set_speed(data, items, speed, maintain_pitch)?;
        }
        EditCommand::SetItemParamMany { items, patch } => {
            data = parameters::execute_set_item_param_many(data, items, patch, copied_text_bytes)?;
        }
        EditCommand::AddMask { item: id, mask } => {
            data = masks::execute_add_mask(data, id, mask)?;
        }
        EditCommand::RemoveMask { item: id, mask } => {
            data = masks::execute_remove_mask(data, id, mask)?;
        }
        EditCommand::UpdateMask { item: id, mask } => {
            data = masks::execute_update_mask(data, id, mask)?;
        }
        EditCommand::ReorderMasks { item: id, order } => {
            data = masks::execute_reorder_masks(data, id, order)?;
        }
        EditCommand::AddEffect { item: id, effect } => {
            data = effects::execute_add_effect(data, id, effect)?;
        }
        EditCommand::RemoveEffect { item: id, effect } => {
            data = effects::execute_remove_effect(data, id, effect)?;
        }
        EditCommand::UpdateEffectParam {
            item: id,
            effect,
            key,
            value,
        } => {
            data = effects::execute_update_effect_param(data, id, effect, key, value)?;
        }
        EditCommand::ToggleEffect {
            item: id,
            effect,
            enabled,
        } => {
            data = effects::execute_toggle_effect(data, id, effect, enabled)?;
        }
        EditCommand::ReorderEffects { item: id, order } => {
            data = effects::execute_reorder_effects(data, id, order)?;
        }
        EditCommand::SetAnimatedParam {
            item: id,
            path,
            param,
        } => {
            data = keyframes::execute_set_animated_param(data, id, path, param)?;
        }
        EditCommand::AddKeyframe {
            item: id,
            path,
            component,
            key,
        } => {
            data = keyframes::execute_add_keyframe(data, id, path, component, key)?;
        }
        EditCommand::ToggleKeyframe {
            item: id,
            path,
            component,
            time,
            value,
        } => {
            data = keyframes::execute_toggle_keyframe(data, id, path, component, time, value)?;
        }
        EditCommand::DeleteKeyframes { keys } => {
            data = keyframes::execute_delete_keyframes(data, keys)?;
        }
        EditCommand::SetKeyframeExtrapolation {
            item: id,
            path,
            component,
            before,
            after,
        } => {
            data = keyframes::execute_set_keyframe_extrapolation(
                data, id, path, component, before, after,
            )?;
        }
        EditCommand::RemoveKeyframe {
            item: id,
            path,
            component,
            time,
        } => {
            data = keyframes::execute_remove_keyframe(data, id, path, component, time)?;
        }
        EditCommand::RetimeKeyframe {
            item: id,
            path,
            component,
            from,
            to,
        } => {
            data = keyframes::execute_retime_keyframe(data, id, path, component, from, to)?;
        }
        EditCommand::SetKeyframeHandle {
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
        EditCommand::ScaleKeyframeTime {
            item: id,
            numerator,
            denominator,
        } => {
            data = keyframes::execute_scale_keyframe_time(data, id, numerator, denominator)?;
        }
        EditCommand::BookmarkAdd {
            start,
            duration,
            note,
            color,
        } => {
            data = project_edits::execute_bookmark_add(
                data, start, duration, note, color, created, next,
            )?;
        }
        EditCommand::MoveBookmark { bookmark, start } => {
            data = project_edits::execute_move_bookmark(data, bookmark, start)?;
        }
        EditCommand::ToggleBookmark { at } => {
            data = project_edits::execute_toggle_bookmark(data, at, created, next)?;
        }
        EditCommand::UpdateBookmark {
            bookmark,
            note,
            color,
            duration,
        } => {
            data = project_edits::execute_update_bookmark(data, bookmark, note, color, duration)?;
        }
        EditCommand::RemoveBookmark { bookmark } => {
            data = project_edits::execute_remove_bookmark(data, bookmark)?;
        }
        EditCommand::Retranslate {
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
        EditCommand::Batch { commands } => {
            for command in commands {
                // Consume each candidate before applying the next child; validation must
                // not clone an unvalidated allocation or publish a partial batch.
                data = apply(data, command, created, next, sources, copied_text_bytes)?.into_data();
            }
        }
        EditCommand::TimelineEdit {
            commands: _commands,
            ripple: _ripple,
        } => return Err(EditError::Invalid("nested ripple policy")),
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

pub use project_edits::DEFAULT_BOOKMARK_COLOR;

pub(crate) use clips::close_vacated_tails as ripple;

/// A global ripple pass replaces command-local shifts, including inside batches.
/// Run only after the command budget is admitted.
pub(crate) fn lift_ripple_deletes(command: &mut Command) {
    let mut stack = vec![command];
    while let Some(command) = stack.pop() {
        match command {
            Command::RippleDelete { item } => *command = Command::Delete { items: vec![*item] },
            Command::Batch { commands } => stack.extend(commands),
            _ => {}
        }
    }
}
