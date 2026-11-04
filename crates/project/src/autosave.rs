//! Monotonic snapshot names independent of wall-clock changes. Callers serialize
//! autosaves for one project path; a concurrent name collision refuses overwrite.
use crate::{Error, Project, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The project filename itself scopes recovery files, including spaces/Unicode.
pub fn directory(project_path: &Path) -> PathBuf {
    let mut name = project_path.as_os_str().to_os_string();
    name.push(".autosave");
    PathBuf::from(name)
}
/// Oldest first, capped directory enumeration; only regular canonical snapshot
/// filenames participate. Unrelated files, symlinks and directories are untouched.
pub fn list(project_path: &Path) -> Result<Vec<PathBuf>> {
    let dir = directory(project_path);
    if fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::Invalid("autosave directory is a symlink"));
    }
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut found = Vec::new();
    for (i, entry) in entries.enumerate() {
        if i >= 10000 {
            return Err(Error::TooLarge("autosave directory"));
        }
        let entry = entry?;
        if entry.file_type()?.is_file() && revision(&entry.path()).is_some() {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}
fn revision(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let digits = name.strip_prefix("snapshot-")?.strip_suffix(".vibecut")?;
    let n: u64 = digits.parse().ok()?;
    (format!("{n:020}") == digits).then_some(n)
}
/// Publish a new snapshot before deleting any old version. Retention is 1..=1000;
/// rotation failure is returned even though the complete new snapshot remains.
pub fn save(project_path: &Path, project: &Project, keep: usize) -> Result<PathBuf> {
    if !(1..=1000).contains(&keep) {
        return Err(Error::Invalid("autosave retention"));
    }
    let bytes = crate::encode(project, false)?;
    let mut paths = list(project_path)?;
    let next = paths
        .last()
        .and_then(|p| revision(p))
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(Error::Invalid("autosave sequence overflow"))?;
    let dir = directory(project_path);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("snapshot-{next:020}.vibecut"));
    crate::atomic::atomic_write_new(&path, &bytes)?;
    paths.push(path.clone());
    let remove = paths.len().saturating_sub(keep);
    for old in paths.into_iter().take(remove) {
        fs::remove_file(old)?;
    }
    Ok(path)
}
