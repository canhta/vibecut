//! Applies project edits commands to a candidate project.
use super::{
    BTreeSet, Bookmark, BookmarkId, CreatedId, EditError, ProjectData, Result, Tick, TrackId, alloc,
};
pub(super) fn execute_create_scene(
    mut data: ProjectData,
    name: captions::NfcText,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
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

    Ok(data)
}
pub(super) fn execute_delete_scenes(
    mut data: ProjectData,
    scenes: Vec<project::SceneId>,
) -> Result<ProjectData> {
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

    Ok(data)
}
pub(super) fn execute_rename_scene(
    mut data: ProjectData,
    scene: project::SceneId,
    name: captions::NfcText,
) -> Result<ProjectData> {
    data.scenes
        .get_mut(scene)
        .ok_or(EditError::Invalid("scene not found"))?
        .name = name;

    Ok(data)
}
pub(super) fn execute_rename_project(
    mut data: ProjectData,
    name: captions::NfcText,
) -> Result<ProjectData> {
    data.name = name;
    Ok(data)
}
pub(super) fn execute_set_canvas_background(
    mut data: ProjectData,
    color: project::Color,
) -> Result<ProjectData> {
    data.canvas.background = color;
    Ok(data)
}
pub(super) fn execute_add_bookmark(
    mut data: ProjectData,
    start: Tick,
    duration: Tick,
    note: captions::NfcText,
    color: project::Color,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    let id = BookmarkId(alloc(next)?);
    data.scene_mut().bookmarks.push(Bookmark {
        id,
        start,
        duration,
        note,
        color,
    });
    created.push(CreatedId::Bookmark(id));

    Ok(data)
}
pub(super) fn execute_move_bookmark(
    mut data: ProjectData,
    bookmark: BookmarkId,
    start: Tick,
) -> Result<ProjectData> {
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

    Ok(data)
}
