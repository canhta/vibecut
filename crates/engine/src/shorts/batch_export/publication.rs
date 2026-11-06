//! Reconciles published short artifacts.
use super::{ClipPaths, EditError, Mutex, Path, Published};
fn artifact_present(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Drop publication marks whose artifact disappeared; keep every artifact that
/// still exists so a retry does not overwrite its own completed output. Reads the
/// marks before touching the filesystem and holds no lock across that IO.
pub(super) fn reconcile_published(
    published: &Mutex<Published>,
    paths: &ClipPaths,
) -> Result<(bool, Vec<bool>), EditError> {
    let (video, mut subtitles) = {
        let state = published.lock().map_err(|_| EditError::JobPoisoned)?;
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
        let mut state = published.lock().map_err(|_| EditError::JobPoisoned)?;
        state.video = video;
        state.subtitles.clone_from(&subtitles);
    }
    Ok((video, subtitles))
}

pub(super) fn mark_video(published: &Mutex<Published>) -> Result<(), EditError> {
    published.lock().map_err(|_| EditError::JobPoisoned)?.video = true;
    Ok(())
}

pub(super) fn mark_subtitle(published: &Mutex<Published>, index: usize) -> Result<(), EditError> {
    let mut state = published.lock().map_err(|_| EditError::JobPoisoned)?;
    let done = state
        .subtitles
        .get_mut(index)
        .ok_or(EditError::Invalid("clip artifact index"))?;
    *done = true;
    Ok(())
}
