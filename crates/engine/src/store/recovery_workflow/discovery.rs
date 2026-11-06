//! Discovers abandoned recoverable sessions.
use super::{
    Candidate, Deserialize, Discovery, EditError, File, JOURNAL, JOURNAL_VERSION, Journal,
    MAX_META, MAX_PROJECT, Path, PathBuf, Read, Result, Session, SessionLock, TryLockError, fs,
};
impl Candidate {
    /// Open as never-saved work. Its original path remains informational metadata;
    /// save() therefore requires an explicit save-as destination.
    pub fn recover(&self, history_limit: usize) -> Result<Session> {
        Session::new(self.project.clone(), history_limit)
    }
    /// Removes only this journal and its canonical snapshots, preserving the source.
    /// Consumes the claimed candidate; unrelated files are never removed.
    pub fn discard(self) -> Result<()> {
        clear(&self.directory)
    }
}
fn regular(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(EditError::RecoveryCorrupt);
    }
    Ok(())
}
impl Drop for SessionLock {
    fn drop(&mut self) {
        // A concurrent fork may retain the open-file description until exec.
        // Release at the owner's boundary rather than its last descriptor close.
        let _ = self.0.unlock();
    }
}
pub(super) fn lock(path: &Path) -> Result<Option<SessionLock>> {
    regular(path)?;
    let file = File::options().read(true).write(true).open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(SessionLock(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(lock_error(e)),
    }
}
pub(super) fn lock_error(e: std::io::Error) -> EditError {
    if e.kind() == std::io::ErrorKind::Unsupported {
        EditError::Unsupported("session locking")
    } else {
        EditError::Io(e)
    }
}
pub(super) fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    regular(path)?;
    let mut bytes = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(EditError::RecoveryCorrupt);
    }
    Ok(bytes)
}
pub(super) fn candidate(directory: PathBuf, file: SessionLock) -> Result<Candidate> {
    let bytes = read_bounded(&directory.join(JOURNAL), MAX_META)?;
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    let header: Version = serde_json::from_slice(&bytes).map_err(|_| EditError::RecoveryCorrupt)?;
    if header.version != JOURNAL_VERSION {
        return Err(EditError::Unsupported("journal schema"));
    }
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|_| EditError::RecoveryCorrupt)?;
    if journal.revision == 0 {
        return Err(EditError::RecoveryCorrupt);
    }
    let base = directory.join("project");
    // Canonical list membership rejects traversal, links and unbounded filenames.
    let paths = project::autosave::list(&base)?;
    let path = paths
        .into_iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(journal.snapshot.as_str()))
        .ok_or(EditError::RecoveryCorrupt)?;
    let bytes = read_bounded(&path, MAX_PROJECT)?;
    if *blake3::hash(&bytes).as_bytes() != journal.content_hash {
        return Err(EditError::RecoveryCorrupt);
    }
    let project = project::decode(&bytes)?.project;
    if project.data().id != journal.project_id || project.data().name.as_str() != journal.name {
        return Err(EditError::RecoveryCorrupt);
    }
    Ok(Candidate {
        directory,
        _lock: file,
        journal,
        project,
    })
}
/// Scan the app-provided private recovery root, capped at 10,000 entries. Live
/// sessions are excluded, including other windows in this process. A candidate
/// remains exclusively claimed while the returned handle exists.
pub fn discover(root: &Path) -> Result<Discovery> {
    let mut result = Discovery {
        candidates: Vec::new(),
        diagnostics: Vec::new(),
    };
    if !root.exists() {
        return Ok(result);
    }
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(EditError::RecoveryCorrupt);
    }
    for (i, entry) in fs::read_dir(root)?.enumerate() {
        if i >= 10000 {
            return Err(EditError::Limit);
        }
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let directory = entry.path();
        let found = (|| {
            let Some(file) = lock(&directory.join("lock"))? else {
                return Ok(None);
            };
            if !directory.join(JOURNAL).try_exists()? {
                return Ok(None);
            }
            candidate(directory, file).map(Some)
        })();
        match found {
            Ok(Some(c)) => result.candidates.push(c),
            Ok(None) => {}
            Err(e) => result.diagnostics.push(e),
        }
    }
    Ok(result)
}
pub(super) fn clear(directory: &Path) -> Result<()> {
    // Descriptor first: interruption cannot resurrect a discarded candidate.
    match fs::remove_file(directory.join(JOURNAL)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    for path in project::autosave::list(&directory.join("project"))? {
        fs::remove_file(path)?;
    }
    Ok(())
}
