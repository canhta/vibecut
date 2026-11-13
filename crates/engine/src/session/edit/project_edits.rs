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
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
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
pub(super) fn execute_set_canvas(
    mut data: ProjectData,
    width: u32,
    height: u32,
    preset: Option<project::AspectPreset>,
) -> Result<ProjectData> {
    let size = project::CanvasSize { width, height };
    // Custom entry commits the nearest even size so the canvas can always
    // export; fixed presets already carry even dimensions.
    let size = if preset.is_none() {
        size.rounded_even()
            .ok_or(EditError::Invalid("canvas dimensions"))?
    } else {
        size
    };
    if !size.is_valid() {
        return Err(EditError::Invalid("canvas dimensions"));
    }
    if let Some(preset) = preset {
        let expected = preset
            .resolved(data.original_canvas_size)
            .ok_or(EditError::Invalid("original canvas size"))?;
        if expected != size {
            return Err(EditError::Invalid("canvas preset size"));
        }
    }
    data.canvas.width = size.width;
    data.canvas.height = size.height;
    data.canvas.preset = preset;
    if preset.is_none() {
        data.canvas.last_custom_size = size;
    }
    Ok(data)
}
pub(super) fn execute_set_fps(mut data: ProjectData, num: i64, den: i64) -> Result<ProjectData> {
    data.canvas.fps = time::FrameRate::new(num, den)?;
    Ok(data)
}
pub(super) fn execute_set_background(
    mut data: ProjectData,
    color: project::Color,
) -> Result<ProjectData> {
    data.canvas.background = color;
    Ok(data)
}
/// Moves `color` to the front of the recent colours, dropping its older entry and any
/// overflow beyond the project bound.
pub(super) fn execute_record_recent_colour(
    mut data: ProjectData,
    color: project::Color,
) -> ProjectData {
    data.recent_colors.retain(|candidate| *candidate != color);
    data.recent_colors.insert(0, color);
    data.recent_colors.truncate(project::MAX_RECENT_COLORS);
    data
}
pub(super) fn execute_bookmark_add(
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
/// Default marker colour for a bookmark created by a toggle.
pub const DEFAULT_BOOKMARK_COLOR: project::Color = project::Color([0, 157, 255, 255]);
pub(super) fn execute_toggle_bookmark(
    mut data: ProjectData,
    at: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    if at < Tick::ZERO {
        return Err(EditError::Invalid("bookmark start"));
    }
    let at = data.canvas.fps.snap_nearest(at)?;
    let bookmarks = &mut data.scene_mut().bookmarks;
    if let Some(index) = bookmarks.iter().position(|candidate| candidate.start == at) {
        bookmarks.remove(index);
    } else {
        let id = BookmarkId(alloc(next)?);
        bookmarks.push(Bookmark {
            id,
            start: at,
            duration: Tick::ZERO,
            note: captions::NfcText::from(""),
            color: DEFAULT_BOOKMARK_COLOR,
        });
        created.push(CreatedId::Bookmark(id));
    }

    Ok(data)
}
pub(super) fn execute_update_bookmark(
    mut data: ProjectData,
    bookmark: BookmarkId,
    note: Option<captions::NfcText>,
    color: Option<project::Color>,
    duration: Option<Tick>,
) -> Result<ProjectData> {
    let scene_end = data.scene().duration();
    let candidate = data
        .scene_mut()
        .bookmarks
        .iter_mut()
        .find(|candidate| candidate.id == bookmark)
        .ok_or(EditError::Invalid("missing bookmark"))?;
    let duration = match duration {
        Some(duration) if duration < Tick::ZERO => {
            return Err(EditError::Invalid("bookmark duration"));
        }
        Some(duration) => {
            Some(duration.min(scene_end.saturating_sub(candidate.start).max(Tick::ZERO)))
        }
        None => None,
    };
    if note.is_none() && color.is_none() && duration.is_none() {
        return Err(EditError::Invalid("bookmark update"));
    }
    let next = Bookmark {
        id: candidate.id,
        start: candidate.start,
        duration: duration.unwrap_or(candidate.duration),
        note: note.unwrap_or_else(|| candidate.note.clone()),
        color: color.unwrap_or(candidate.color),
    };
    if next == *candidate {
        return Err(EditError::Invalid("bookmark update"));
    }
    *candidate = next;

    Ok(data)
}
pub(super) fn execute_remove_bookmark(
    mut data: ProjectData,
    bookmark: BookmarkId,
) -> Result<ProjectData> {
    let bookmarks = &mut data.scene_mut().bookmarks;
    let index = bookmarks
        .iter()
        .position(|candidate| candidate.id == bookmark)
        .ok_or(EditError::Invalid("missing bookmark"))?;
    bookmarks.remove(index);

    Ok(data)
}
