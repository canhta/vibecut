//! Keeps one project's chat transcript in app data, outside the project file.
use super::transcript::Entry;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file layout version. A different version is an unreadable history, never migrated.
const VERSION: u32 = 1;
/// Lines kept per project; the oldest go first.
pub(super) const MAX_ENTRIES: usize = 400;
/// Largest history file read back.
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    entries: Vec<Entry>,
}

fn file(dir: &Path, project: u64) -> PathBuf {
    dir.join(format!("{project}.json"))
}

/// The saved transcript of `project`; a missing, oversized or unreadable file is an empty one.
pub(super) fn load(dir: &Path, project: u64) -> Vec<Entry> {
    let path = file(dir, project);
    let small = std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_BYTES);
    if !small {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    match serde_json::from_slice::<Document>(&bytes) {
        Ok(document) if document.version == VERSION => document.entries,
        _ => Vec::new(),
    }
}

/// Write the newest `MAX_ENTRIES` lines through a temporary file, so a crash keeps the
/// previous history.
pub(super) fn save(dir: &Path, project: u64, entries: &[Entry]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let start = entries.len().saturating_sub(MAX_ENTRIES);
    let document = Document {
        version: VERSION,
        entries: entries.get(start..).unwrap_or_default().to_vec(),
    };
    let bytes = serde_json::to_vec(&document).map_err(std::io::Error::other)?;
    let path = file(dir, project);
    let temporary = dir.join(format!("{project}.json.tmp"));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

/// Remove the saved transcript; none saved is fine.
pub(super) fn clear(dir: &Path, project: u64) -> std::io::Result<()> {
    match std::fs::remove_file(file(dir, project)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_round_trips_per_project_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![
            Entry::User { text: "cut".into() },
            Entry::Assistant {
                text: "done".into(),
            },
        ];
        save(dir.path(), 5, &entries).unwrap();
        assert_eq!(load(dir.path(), 5), entries);
        assert!(load(dir.path(), 6).is_empty(), "another project");
        clear(dir.path(), 5).unwrap();
        assert!(load(dir.path(), 5).is_empty());
        clear(dir.path(), 5).unwrap();
    }

    #[test]
    fn a_foreign_version_and_garbage_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(file(dir.path(), 1), br#"{"version":2,"entries":[]}"#).unwrap();
        assert!(load(dir.path(), 1).is_empty());
        std::fs::write(file(dir.path(), 2), b"not json").unwrap();
        assert!(load(dir.path(), 2).is_empty());
    }

    #[test]
    fn only_the_newest_lines_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<Entry> = (0..MAX_ENTRIES + 3)
            .map(|index| Entry::User {
                text: index.to_string(),
            })
            .collect();
        save(dir.path(), 1, &entries).unwrap();
        let loaded = load(dir.path(), 1);
        assert_eq!(loaded.len(), MAX_ENTRIES);
        assert_eq!(loaded.first(), entries.get(3));
    }
}
