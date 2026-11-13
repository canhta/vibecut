//! Freezes the project frame at a playhead into a held still item.
//!
//! The frame is the production composite at that tick. It is published as a PNG named by
//! its content hash in the caller's directory and imported as an image asset; the same
//! frame again reuses that asset. The timeline edit runs as one transaction: split the
//! host item at the playhead, move the later pieces of its track right by the hold and
//! insert the still between the halves. No transition is created at the new cuts.
use super::{Error as ArtifactError, Result as ArtifactResult, frame_snapshot};
use crate::session::{EditError, Outcome};
use crate::{ByteBudget, ByteBudgetHooks, Command, NewItem, Placement, Session};
use media::{AssetId, ContentHash, SourceAsset};
use project::{ItemId, ItemParams, Project, ProjectData, VideoParams};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use time::{SourceRange, Tick};

/// Freezes the visible video item `item` at output time `at` for `duration`.
///
/// `dir` must be absolute; it is created when missing. Refusals are typed and change
/// nothing: a relative directory, a non-positive duration, an unknown or non-video item,
/// a locked track, or a playhead outside the item's output span.
pub fn freeze(
    session: &mut Session,
    item: ItemId,
    at: Tick,
    duration: Tick,
    dir: &Path,
    protected: &[PathBuf],
    budget: &ByteBudget,
) -> ArtifactResult<Outcome> {
    if !dir.is_absolute() {
        return Err(EditError::Invalid("freeze directory").into());
    }
    if duration <= Tick::ZERO {
        return Err(EditError::Invalid("freeze duration").into());
    }
    validate_host(session.project(), item, at)?;
    let frame = frame_snapshot::image(session.project(), at)?;
    let hash = ContentHash::from_bytes(&frame.png);
    let hex: String = hash.0.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = dir.join(format!("freeze-{hex}.png"));
    if !path.exists() {
        std::fs::create_dir_all(dir)?;
        let hooks = ByteBudgetHooks::new(budget, None);
        let len = u64::try_from(frame.png.len())
            .map_err(|_| ArtifactError::from(crate::session::wire::ControlError::Limit))?;
        crate::publication::Publication {
            project: session.project(),
            path: &path,
            protected,
            hooks: &hooks,
        }
        .write(
            project::atomic::Overwrite::Refuse,
            len,
            &project::atomic::Fs,
            |writer| -> crate::runtime::Result<()> {
                writer.write_all(&frame.png)?;
                Ok(())
            },
        )?;
    }
    let existing = session
        .project()
        .data()
        .assets
        .iter()
        .find(|asset| asset.content_hash() == hash)
        .map(SourceAsset::id);
    let import = match existing {
        Some(_) => None,
        None => {
            let id = next_asset_id(session.project().data())?;
            Some(SourceAsset::import(id, path.clone())?)
        }
    };
    let asset = match &import {
        Some(asset) => asset.id(),
        None => existing.ok_or(EditError::Invalid("freeze asset"))?,
    };
    session.transaction(|session| -> ArtifactResult<Outcome> {
        if let Some(asset) = import {
            session.apply(Command::AddAsset { asset })?;
        }
        session.apply(Command::SplitAt { item, at })?;
        let (track, later) = {
            let timeline = &session.project().data().scene().timeline;
            let track = timeline
                .iter()
                .find(|track| track.items.iter().any(|candidate| candidate.id == item))
                .ok_or(EditError::NoItem(item))?;
            let later = track
                .items
                .iter()
                .filter(|candidate| candidate.start >= at)
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            (track.id, later)
        };
        session.apply(Command::TimelineMoveItems {
            items: later,
            delta: duration,
            placement: None,
        })?;
        Ok(session.apply(Command::InsertItem {
            placement: Placement::Track { track },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(asset),
                source_range: SourceRange::new(Tick::ZERO, duration).map_err(EditError::from)?,
                start: at,
                content: ItemParams::Video(VideoParams::plain()),
            },
        })?)
    })
}

/// The host must be an unlocked video item whose output span strictly contains `at`.
fn validate_host(project: &Project, item: ItemId, at: Tick) -> ArtifactResult<()> {
    let timeline = &project.data().scene().timeline;
    let track = timeline
        .iter()
        .find(|track| track.items.iter().any(|candidate| candidate.id == item))
        .ok_or(EditError::NoItem(item))?;
    if track.locked {
        return Err(EditError::Locked(track.id).into());
    }
    let host = track
        .items
        .iter()
        .find(|candidate| candidate.id == item)
        .ok_or(EditError::NoItem(item))?;
    if !matches!(host.content, ItemParams::Video(_)) {
        return Err(EditError::Unsupported("freeze non-video item").into());
    }
    let end = host.end().map_err(EditError::from)?;
    if at <= host.start || at >= end {
        return Err(EditError::Invalid("freeze playhead outside item").into());
    }
    Ok(())
}

/// One past the largest asset ID; refuses when the ID space is exhausted.
fn next_asset_id(data: &ProjectData) -> ArtifactResult<AssetId> {
    let next = match data.assets.iter().map(|asset| asset.id().0).max() {
        Some(max) => max
            .checked_add(1)
            .ok_or(EditError::Invalid("asset id exhausted"))?,
        None => 0,
    };
    Ok(AssetId(next))
}
