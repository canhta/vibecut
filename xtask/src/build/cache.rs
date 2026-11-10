//! Age and size-prune the shared build cache without touching held slots.
use super::{
    environment::GIB,
    queue::{open, try_lock},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(super) fn cache() -> Result<PathBuf, String> {
    // Same per-user cache owner as native provisioning.
    crate::ffmpeg::cache()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cache has no parent".into())
}

pub(super) const STALE_AGE: Duration = Duration::from_secs(2 * 24 * 60 * 60);
// A warm slot holds 14-21 GiB after one workspace test build; stay above it so cleanup keeps warm caches.
pub(crate) const CACHE_SLOT_LIMIT: u64 = 40 * GIB;

/// Prune the shared build cache: sessions and whole slots untouched for two
/// days go first; an over-budget free slot drops dependencies oldest first
/// and is cleared under its lock only if it still exceeds the budget.
/// Held slots are left alone because a build is using their artifacts.
pub(crate) fn prune_cache(dry_run: bool) -> Result<(), String> {
    prune_slots(&cache()?, SystemTime::now(), CACHE_SLOT_LIMIT, dry_run)
}

pub(super) fn older_than(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= STALE_AGE)
}

/// Cache entries are shared with live builds. A path that disappears or stops
/// being a directory between two calls is simply no longer ours to inspect.
pub(super) fn changed_under_us(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

pub(super) fn newest_modified(path: &Path) -> io::Result<Option<SystemTime>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if changed_under_us(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut newest = metadata.modified().ok();
    if metadata.is_dir() {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if changed_under_us(&error) => return Ok(newest),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if changed_under_us(&error) => continue,
                Err(error) => return Err(error),
            };
            if let Some(modified) = newest_modified(&entry.path())?
                && newest.is_none_or(|current| modified > current)
            {
                newest = Some(modified);
            }
        }
    }
    Ok(newest)
}

pub(super) fn remove_entry(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if changed_under_us(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let result = if metadata.is_dir() {
        remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Err(error) if changed_under_us(&error) => Ok(()),
        result => result,
    }
}

pub(super) fn prune_incrementals(slot: &Path, now: SystemTime, dry_run: bool) -> io::Result<()> {
    let profiles = match fs::read_dir(slot) {
        Ok(profiles) => profiles,
        Err(error) if changed_under_us(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    for profile in profiles {
        let profile = match profile {
            Ok(profile) => profile,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        let incremental = profile.path().join("incremental");
        let sessions = match fs::read_dir(&incremental) {
            Ok(sessions) => sessions,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        for session in sessions {
            let session = match session {
                Ok(session) => session,
                Err(error) if changed_under_us(&error) => continue,
                Err(error) => return Err(error),
            };
            let path = session.path();
            let modified = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata.modified().ok(),
                Err(error) if changed_under_us(&error) => continue,
                Err(error) => return Err(error),
            };
            if modified.is_some_and(|modified| older_than(modified, now)) {
                if dry_run {
                    eprintln!("[cleanup] would remove stale session {}", path.display());
                } else {
                    remove_entry(&path)?;
                }
            }
        }
    }
    Ok(())
}

/// One dependency artifact of an idle slot, found under a profile's `deps`.
struct Dep {
    modified: SystemTime,
    bytes: u64,
    path: PathBuf,
}

fn collect_deps(path: &Path, depth: usize, out: &mut Vec<Dep>) -> io::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if changed_under_us(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|name| name == "deps") {
            collect_dep_entries(&path, out)?;
        } else {
            collect_deps(&path, depth - 1, out)?;
        }
    }
    Ok(())
}

fn collect_dep_entries(deps: &Path, out: &mut Vec<Dep>) -> io::Result<()> {
    let entries = match fs::read_dir(deps) {
        Ok(entries) => entries,
        Err(error) if changed_under_us(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        let path = entry.path();
        let Some(modified) = newest_modified(&path)? else {
            continue;
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        let bytes = if metadata.is_dir() {
            used_bytes(&path)?
        } else {
            file_bytes(&metadata)
        };
        out.push(Dep {
            modified,
            bytes,
            path,
        });
    }
    Ok(())
}

/// Shrink an idle slot that holds `used` bytes toward `limit` by removing its
/// dependency artifacts oldest first. Every stale artifact goes, then fresher
/// ones only while the slot is still over the limit, so a slot is cleared whole
/// only when this cannot reach the limit. Returns the bytes removed (or that a
/// dry run would remove).
pub(super) fn prune_deps(
    slot: &Path,
    now: SystemTime,
    used: u64,
    limit: u64,
    dry_run: bool,
) -> io::Result<u64> {
    let mut deps = Vec::new();
    collect_deps(slot, 4, &mut deps)?;
    deps.sort_by_key(|dep| dep.modified);
    let mut freed = 0;
    for dep in deps {
        if used.saturating_sub(freed) <= limit && !older_than(dep.modified, now) {
            break;
        }
        if dry_run {
            eprintln!("[cleanup] would remove dependency {}", dep.path.display());
        } else {
            eprintln!("[cleanup] remove dependency {}", dep.path.display());
            remove_entry(&dep.path)?;
        }
        freed += dep.bytes;
    }
    Ok(freed)
}

pub(super) fn remove_slot(slot: &Path, dry_run: bool, reason: &str) -> Result<(), String> {
    if dry_run {
        eprintln!("[cleanup] would remove {} ({reason})", slot.display());
    } else {
        eprintln!("[cleanup] remove {} ({reason})", slot.display());
        remove_entry(slot).map_err(|error| format!("{}: {error}", slot.display()))?;
    }
    Ok(())
}

/// Parse existing `target/slot-N` directory names. Cleanup enumerates the
/// cache instead of a fixed range, so slots left by a larger past capacity
/// still age out and a later larger capacity reuses them.
pub(super) fn slot_numbers(target: &Path) -> io::Result<Vec<usize>> {
    let mut numbers = Vec::new();
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        if let Some(number) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix("slot-"))
            .and_then(|number| number.parse::<usize>().ok())
        {
            numbers.push(number);
        }
    }
    numbers.sort_unstable();
    Ok(numbers)
}

pub(super) fn prune_slots(
    base: &Path,
    now: SystemTime,
    size_limit: u64,
    dry_run: bool,
) -> Result<(), String> {
    let target = base.join("target");
    if !target.is_dir() {
        return Ok(());
    }
    let locks = base.join("locks");
    fs::create_dir_all(&locks).map_err(|error| format!("{}: {error}", locks.display()))?;
    let numbers =
        slot_numbers(&target).map_err(|error| format!("{}: {error}", target.display()))?;
    for number in numbers {
        let slot = target.join(format!("slot-{number}"));
        if !slot.is_dir() {
            continue;
        }
        let lock = open(&locks.join(format!("slot-{number}")))
            .and_then(try_lock)
            .map_err(|error| format!("slot-{number} lock: {error}"))?;
        if lock.is_none() {
            eprintln!("[cleanup] slot-{number} is in use; keeping it");
            continue;
        }
        let stale = newest_modified(&slot)
            .map_err(|error| format!("{}: {error}", slot.display()))?
            .is_some_and(|modified| older_than(modified, now));
        if stale {
            remove_slot(&slot, dry_run, "untouched for 2+ days")?;
            continue;
        }
        prune_incrementals(&slot, now, dry_run)
            .map_err(|error| format!("{}: {error}", slot.display()))?;
        let used = used_bytes(&slot).map_err(|error| format!("{}: {error}", slot.display()))?;
        if used > size_limit {
            let freed = prune_deps(&slot, now, used, size_limit, dry_run)
                .map_err(|error| format!("{}: {error}", slot.display()))?;
            let remaining = if dry_run {
                used.saturating_sub(freed)
            } else {
                used_bytes(&slot).map_err(|error| format!("{}: {error}", slot.display()))?
            };
            if remaining > size_limit {
                remove_slot(
                    &slot,
                    dry_run,
                    &format!(
                        "{:.1} GiB over the size budget after pruning dependencies",
                        remaining as f64 / GIB as f64
                    ),
                )?;
            } else if freed > 0 {
                eprintln!(
                    "[cleanup] slot retained after pruning {:.1} GiB of dependencies",
                    freed as f64 / GIB as f64
                );
            }
        }
    }
    Ok(())
}

pub(super) fn used_bytes(path: &Path) -> io::Result<u64> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if changed_under_us(&e) => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut total = 0u64;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) if changed_under_us(&e) => continue,
            Err(e) => return Err(e),
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if changed_under_us(&e) => continue,
            Err(e) => return Err(e),
        };
        let size = if metadata.is_dir() {
            used_bytes(&path)?
        } else {
            file_bytes(&metadata)
        };
        total = total.saturating_add(size);
    }
    Ok(total)
}

pub(super) fn file_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(windows)]
    {
        metadata.len()
    }
}

#[cfg(windows)]
pub(super) fn disk_free(path: &Path) -> Result<u64, String> {
    let mut available = 0;
    winsafe::GetDiskFreeSpaceEx(
        Some(path.to_str().ok_or("cache path is not UTF-8")?),
        Some(&mut available),
        None,
        None,
    )
    .map_err(|error| format!("disk free-space query: {error}"))?;
    Ok(available)
}

#[cfg(unix)]
pub(super) fn disk_free(path: &Path) -> Result<u64, String> {
    use std::process::Command;
    let result = Command::new("df").args(["-Pk"]).arg(path).output();
    let result = result.map_err(|e| format!("disk free-space query: {e}"))?;
    if !result.status.success() {
        return Err("disk free-space query failed".into());
    }
    let text = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
    let value = text
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|value| value.checked_mul(1024))
        .ok_or("cannot parse disk free space")?;
    Ok(value)
}

pub(super) fn remove_dir(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
