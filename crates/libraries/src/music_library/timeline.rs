//! Inserts cached music into a project.
use super::{
    CachedMusic, EngineError, Error, MAX_AUDIO_BYTES, Music, MusicLibrary, MusicResult, Path,
    Placement, Read, Result, Session, Tick, cached_from_path, fs, hex, private_directory,
    validate_result,
};
/// Copy a cached track into project-owned media so library unsave, clear-all and
/// URL invalidation can never remove bytes a project references. The file name is
/// the content hash: re-adding the same track reuses it and undo/redo keep a
/// valid path. Returns the project copy and whether this call created it.
fn import_into_project(cached: &CachedMusic, media_dir: &Path) -> Music<(CachedMusic, bool)> {
    private_directory(media_dir)?;
    let name: String = cached
        .content_hash
        .0
        .iter()
        .flat_map(|byte| [hex(byte >> 4), hex(*byte)])
        .collect();
    let path = media_dir.join(format!("{name}.audio"));
    let verified = |path: &Path| -> Music<CachedMusic> {
        let imported = cached_from_path(path.to_owned(), None)?;
        if imported.content_hash != cached.content_hash {
            return Err(Error::LibraryCorrupt);
        }
        Ok(imported)
    };
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => return Ok((verified(&path)?, false)),
        Ok(_) => return Err(Error::LibraryCorrupt),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let imported = project::atomic::stage_validated(
        &path,
        |output| {
            let mut input = fs::File::open(&cached.path)?.take(MAX_AUDIO_BYTES + 1);
            if std::io::copy(&mut input, output)? > MAX_AUDIO_BYTES {
                return Err(Error::Limit);
            }
            Ok(())
        },
        |staged| {
            let imported = verified(staged)?;
            Ok(CachedMusic {
                path: path.clone(),
                ..imported
            })
        },
    )?;
    Ok((imported, true))
}
/// Import a cached track into `media_dir` (project-owned media) and attach it as
/// one audio item and one asset in one undo step. The asset references the project
/// copy, never the library cache.
pub fn add_to_timeline(
    session: &mut Session,
    music: &MusicResult,
    cached: &CachedMusic,
    media_dir: &Path,
    placement: Placement,
    start: Tick,
) -> Result<engine::Outcome> {
    let music = music.library_music()?;
    let id = next_asset_id(session)?;
    let (imported, created) = import_into_project(cached, media_dir)?;
    let result = imported.asset(id).and_then(|asset| {
        session
            .apply(engine::Command::AddLibraryMusic {
                placement,
                start,
                asset,
                music,
            })
            .map_err(EngineError::from)
    });
    if result.is_err() && created {
        let _ = fs::remove_file(&imported.path);
    }
    result
}
/// Offline add: the saved record must have a cached file for its exact URL.
pub fn add_saved(
    session: &mut Session,
    library: &MusicLibrary,
    id: &str,
    media_dir: &Path,
    placement: Placement,
    start: Tick,
) -> Result<engine::Outcome> {
    let music = library
        .saved()?
        .into_iter()
        .find(|saved| saved.music.id == id)
        .map(|saved| saved.music)
        .ok_or(Error::Invalid("saved music"))?;
    validate_result(&music)?;
    let cached = library.cached(&music.audio_url)?.ok_or(Error::NotCached)?;
    add_to_timeline(session, &music, &cached, media_dir, placement, start)
}
fn next_asset_id(session: &Session) -> Result<media::AssetId> {
    let next = session
        .project()
        .data()
        .assets
        .iter()
        .map(|asset| asset.id().0)
        .max()
        .map_or(Some(0), |id| id.checked_add(1))
        .ok_or(EngineError::Session(engine::SessionError::IdExhausted))?;
    Ok(media::AssetId(next))
}
