//! Reconciles published short artifacts.
use super::{ClipPaths, EngineError, Mutex, Path, Published};
fn artifact_present(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Drop publication marks whose artifact disappeared; keep every artifact that
/// still exists so a retry does not overwrite its own completed output. Reads the
/// marks before touching the filesystem and holds no lock across that IO.
pub(super) fn reconcile_published(
    published: &Mutex<Published>,
    paths: &ClipPaths,
) -> Result<(bool, Vec<bool>), EngineError> {
    let (video, mut subtitles) = {
        let state = published
            .lock()
            .map_err(|_| engine::RuntimeError::Poisoned)?;
        (state.video, state.subtitles.clone())
    };
    let video = video && artifact_present(&paths.video);
    for (index, subtitle) in paths.subtitles.iter().enumerate() {
        if subtitles.get(index).copied().unwrap_or(false)
            && !artifact_present(&subtitle.path)
            && let Some(done) = subtitles.get_mut(index)
        {
            *done = false;
        }
    }
    {
        let mut state = published
            .lock()
            .map_err(|_| engine::RuntimeError::Poisoned)?;
        state.video = video;
        state.subtitles.clone_from(&subtitles);
    }
    Ok((video, subtitles))
}

pub(super) fn mark_video(published: &Mutex<Published>) -> Result<(), EngineError> {
    published
        .lock()
        .map_err(|_| engine::RuntimeError::Poisoned)?
        .video = true;
    Ok(())
}

pub(super) fn mark_subtitle(published: &Mutex<Published>, index: usize) -> Result<(), EngineError> {
    let mut state = published
        .lock()
        .map_err(|_| engine::RuntimeError::Poisoned)?;
    let done =
        state
            .subtitles
            .get_mut(index)
            .ok_or(EngineError::Edit(engine::EditError::Invalid(
                "clip artifact index",
            )))?;
    *done = true;
    Ok(())
}
