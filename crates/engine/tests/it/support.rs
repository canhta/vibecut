//! Process-private generated-fixture directories with liveness pruning.
//!
//! Each test process writes beneath `target/fixtures/<area>/<pid>/` and holds an
//! exclusive lock on that directory's `.lock` file for its lifetime. At first
//! use it removes sibling directories whose lock it can take: their owner is
//! gone, so the files cannot be live sources.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::{
    fs::{File, TryLockError},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static OWNED: OnceLock<Mutex<Vec<(String, File)>>> = OnceLock::new();

/// Root of this process's generated fixtures for `area`, created and locked on
/// first use.
pub fn fixture_dir(area: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures")
        .join(area);
    let own = root.join(std::process::id().to_string());
    let mut owned = OWNED.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap();
    if owned.iter().any(|(name, _)| name == area) {
        return own;
    }
    std::fs::create_dir_all(&own).unwrap();
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(own.join(".lock"))
        .unwrap();
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => panic!(
            "fixture owner for {area} is already locked in process {}",
            std::process::id()
        ),
        Err(TryLockError::Error(error)) => panic!("cannot lock fixture owner {area}: {error}"),
    }
    prune(&root, &own);
    owned.push((area.to_owned(), lock));
    own
}

fn prune(root: &Path, own: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == own || !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.parse::<u32>().is_err() {
            continue;
        }
        let Ok(lock) = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join(".lock"))
        else {
            continue;
        };
        if lock.try_lock().is_ok() {
            drop(lock);
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}
