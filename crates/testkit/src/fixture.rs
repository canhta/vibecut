//! Process-private generated-fixture directories with liveness pruning.
//!
//! Each test process writes beneath `target/fixtures/<area>/<pid>/` and holds an
//! exclusive lock on the sibling `<pid>.lock` file for its lifetime. The lock is
//! taken before the directory exists, so a directory another process can see
//! always has a live or a dead owner, never one that is still starting. At first
//! use a process removes sibling directories whose lock it can take: their owner
//! is gone, so the files cannot be live sources.
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static OWNED: OnceLock<Mutex<Vec<(String, File)>>> = OnceLock::new();
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Root of this process's generated fixtures for `area`, created and locked on
/// first use.
pub fn dir(area: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures")
        .join(area);
    let own = root.join(std::process::id().to_string());
    let mut owned = OWNED.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap();
    if owned.iter().any(|(name, _)| name == area) {
        return own;
    }
    std::fs::create_dir_all(&root).unwrap();
    let lock = open_lock(&root, &own).unwrap();
    // Blocking: the only other holder can be a pruner removing a dead owner's
    // folder under a recycled pid, and it releases once that folder is gone.
    lock.lock()
        .unwrap_or_else(|error| panic!("cannot lock fixture owner {area}: {error}"));
    std::fs::create_dir_all(&own).unwrap();
    prune(&root, &own);
    owned.push((area.to_owned(), lock));
    own
}

/// Publishes `dir/name` by writing a temporary sibling through `write` and
/// renaming it into place, so readers never observe a partial file. Returns an
/// existing non-empty fixture without running `write`.
pub fn publish(dir: &Path, name: &str, write: impl FnOnce(&Path)) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    if nonempty(&path) {
        return path;
    }
    let temp = dir.join(format!(
        ".{}.{}.{name}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&temp);
    write(&temp);
    match std::fs::rename(&temp, &path) {
        Ok(()) => {}
        Err(_) if nonempty(&path) => {
            let _ = std::fs::remove_file(&temp);
        }
        Err(error) => panic!("publish fixture {name}: {error}"),
    }
    path
}

fn nonempty(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.len() > 0)
        .unwrap_or(false)
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
        let Ok(lock) = open_lock(root, &path) else {
            continue;
        };
        if lock.try_lock().is_ok() {
            drop(lock);
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Opens the `<pid>.lock` file beside the owner folder `dir`.
fn open_lock(root: &Path, dir: &Path) -> std::io::Result<File> {
    let mut name = dir.file_name().unwrap_or_default().to_owned();
    name.push(".lock");
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(name))
}
