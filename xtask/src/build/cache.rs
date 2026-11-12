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
pub(crate) fn prune_cache(root: &Path, dry_run: bool, make_room: bool) -> Result<(), String> {
    let base = cache()?;
    prune_targets(
        &base,
        SystemTime::now(),
        disk_free(&base)?,
        dry_run,
        make_room,
        &configuration(root, &[])?,
    )?;
    if !make_room {
        prune_slots(&base, SystemTime::now(), CACHE_SLOT_LIMIT, dry_run)?;
    }
    Ok(())
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

const DISK_FLOOR: u64 = 30 * GIB;

/// Learned samples apply to default full-workspace test output under the same
/// compiler, manifests, Cargo configuration and target/profile environment.
/// Explicit target/profile/features and unknown commands use a distinct key,
/// so a default-workload sample cannot lower their conservative bootstrap.
pub(super) fn configuration(root: &Path, args: &[std::ffi::OsString]) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        ".cargo/config.toml",
        "rust-toolchain.toml",
        "rust-toolchain",
    ] {
        hash.update(name.as_bytes());
        match fs::read(root.join(name)) {
            Ok(bytes) => hash.update(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => hash.update(b"missing"),
            Err(error) => return Err(error.to_string()),
        }
    }
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".cargo"))
        });
    if let Some(home) = cargo_home {
        match fs::read(home.join("config.toml")) {
            Ok(bytes) => hash.update(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                hash.update(b"no user cargo configuration")
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = std::process::Command::new(&compiler)
        .current_dir(root)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    if !version.status.success() {
        return Err("cannot identify compiler for build capacity".into());
    }
    hash.update(version.stdout);
    hash.update(std::env::consts::OS.as_bytes());
    hash.update(std::env::consts::ARCH.as_bytes());
    let mut environment = std::env::vars_os()
        .filter(|(key, _)| {
            key.to_str().is_some_and(|key| {
                key.starts_with("CARGO_PROFILE_")
                    || (key.starts_with("CARGO_TARGET_") && key != "CARGO_TARGET_DIR")
                    || matches!(
                        key,
                        "CARGO_BUILD_TARGET"
                            | "CARGO_BUILD_RUSTFLAGS"
                            | "CARGO_BUILD_RUSTC"
                            | "RUSTC_WORKSPACE_WRAPPER"
                            | "RUSTC_BOOTSTRAP"
                            | "RUSTFLAGS"
                            | "CARGO_ENCODED_RUSTFLAGS"
                            | "RUSTC"
                            | "RUSTUP_TOOLCHAIN"
                    )
            })
        })
        .collect::<Vec<_>>();
    environment.sort();
    for (key, value) in environment {
        hash.update(key.as_encoded_bytes());
        hash.update([0]);
        hash.update(value.as_encoded_bytes());
        hash.update([0]);
    }
    let ordinary = args.is_empty()
        || args.first().is_some_and(|arg| arg == "cargo")
            && args.get(1).is_some_and(|arg| {
                matches!(arg.to_str(), Some("test" | "build" | "check" | "xtask"))
            });
    let explicit = args.iter().take_while(|arg| *arg != "--").any(|arg| {
        arg.to_str().is_some_and(|arg| {
            [
                "--config",
                "--release",
                "--profile",
                "--target",
                "--features",
                "--all-features",
                "--no-default-features",
            ]
            .iter()
            .any(|flag| arg == *flag || arg.starts_with(&format!("{flag}=")))
                || arg == "-r"
                || arg.starts_with("-F")
        })
    });
    if !ordinary || explicit {
        for arg in args {
            hash.update(arg.as_encoded_bytes());
            hash.update([0]);
        }
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) fn target_name(root: &Path) -> Result<String, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    Ok(recorded_name(&root))
}

fn recorded_name(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
    let hash: String = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("wt-{hash}")
}

pub(super) struct Target {
    pub(super) name: String,
    pub(super) root: PathBuf,
    pub(super) bytes: u64,
    pub(super) modified: SystemTime,
    pub(super) busy: bool,
    pub(super) valid_owner: bool,
}

pub(super) fn targets(base: &Path) -> Result<Vec<Target>, String> {
    let mut targets = Vec::new();
    let directory = base.join("target");
    if !directory.is_dir() {
        return Ok(targets);
    }
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name
            .strip_prefix("wt-")
            .is_some_and(|hash| hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            continue;
        }
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let root = fs::read_to_string(base.join("locks").join(format!("{name}.owner")))
            .map(PathBuf::from)
            .unwrap_or_default();
        let busy = open(&base.join("locks").join(&name))
            .and_then(try_lock)
            .map_err(|e| e.to_string())?
            .is_none();
        // Canonical owner records remain verifiable after their checkout is
        // removed. An unknown or mismatched owner is never an eviction candidate.
        let identity = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        let valid_owner = identity.is_absolute() && recorded_name(&identity) == name;
        targets.push(Target {
            valid_owner,
            busy,
            bytes: used_bytes(&entry.path()).map_err(|e| e.to_string())?,
            modified: fs::metadata(base.join("locks").join(format!("{name}.used")))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
            name,
            root,
        });
    }
    targets.sort_by_key(|target| (!target.valid_owner || target.root.is_dir(), target.modified));
    Ok(targets)
}

pub(super) fn directory_cap(free: u64, used: u64, reserve: u64) -> u64 {
    free.saturating_add(used).saturating_sub(DISK_FLOOR) / reserve.max(1)
}

#[derive(Default)]
struct Reserve {
    configuration: String,
    measured: u64,
    peak: u64,
}

impl Reserve {
    fn read(base: &Path) -> Self {
        let path = base.join("build-reserve.json");
        use std::io::Read;
        let state = fs::File::open(&path)
            .ok()
            .and_then(|file| {
                let mut bytes = Vec::new();
                file.take(513).read_to_end(&mut bytes).ok()?;
                (bytes.len() <= 512).then_some(bytes)
            })
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let Some(state) = state else {
            return Self::default();
        };
        match (
            state.get("version").and_then(serde_json::Value::as_u64),
            state
                .get("measured_bytes")
                .and_then(serde_json::Value::as_u64),
            state.get("peak_bytes").and_then(serde_json::Value::as_u64),
            state
                .get("configuration")
                .and_then(serde_json::Value::as_str),
        ) {
            (Some(1), Some(measured), Some(peak), Some(configuration))
                if peak >= measured
                    && (measured == 0
                        || configuration.len() == 64
                            && configuration.bytes().all(|byte| byte.is_ascii_hexdigit())) =>
            {
                Self {
                    configuration: configuration.to_owned(),
                    measured,
                    peak,
                }
            }
            _ => Self::default(),
        }
    }

    fn bytes(&self, configuration: &str) -> u64 {
        let warm = if self.measured > 0 && self.configuration == configuration {
            self.measured
        } else {
            CACHE_SLOT_LIMIT
        };
        warm.max(self.peak)
    }

    fn write(&self, base: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "configuration": self.configuration, "measured_bytes": self.measured, "peak_bytes": self.peak
        }))
        .map_err(|e| e.to_string())?;
        let pending = base.join("build-reserve.pending");
        fs::write(&pending, bytes)
            .and_then(|()| fs::rename(pending, base.join("build-reserve.json")))
            .map_err(|e| format!("publish build reserve: {e}"))
    }
}

/// Caller holds admission. Small and incomplete targets can raise the observed
/// peak, but cannot replace the conservative per-target policy bootstrap.
pub(super) fn warm_reserve(
    base: &Path,
    entries: &[Target],
    configuration: &str,
) -> Result<u64, String> {
    let mut state = Reserve::read(base);
    state.peak = state
        .peak
        .max(entries.iter().map(|target| target.bytes).max().unwrap_or(0));
    state.write(base)?;
    Ok(state.bytes(configuration))
}

/// Only an actual successful full-workspace test run can establish a smaller
/// warm sample for its workload and target configuration.
pub(super) fn record_size(
    base: &Path,
    root: &Path,
    target: &Path,
    args: &[std::ffi::OsString],
    success: bool,
) -> Result<(), String> {
    let _admission =
        super::queue::Queue::admission(&base.join("locks")).map_err(|e| e.to_string())?;
    let mut state = Reserve::read(base);
    let bytes = used_bytes(target).map_err(|e| e.to_string())?;
    state.peak = state.peak.max(bytes);
    let full = matches!(
        args.iter()
            .map(|arg| arg.to_str())
            .collect::<Vec<_>>()
            .as_slice(),
        [Some("cargo"), Some("test"), Some("--workspace")]
            | [Some("cargo"), Some("xtask"), Some("ci")]
            | [
                Some("cargo"),
                Some("xtask"),
                Some("ci"),
                Some("--require-oracles")
            ]
    );
    if success && full && bytes > 0 {
        let configuration = configuration(root, args)?;
        state.measured = if state.configuration == configuration {
            state.measured.max(bytes)
        } else {
            bytes
        };
        state.configuration = configuration;
    }
    state.write(base)
}

fn candidates(targets: &[Target]) -> String {
    targets
        .iter()
        .filter(|target| !target.busy && target.valid_owner)
        .map(|target| {
            format!(
                "{} ({}, {})",
                target.name,
                target.root.display(),
                if target.root.is_dir() {
                    "oldest idle first"
                } else {
                    "checkout removed"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A dispatcher reads precisely the same capacity calculation as admission.
pub(crate) fn capacity(root: &Path) -> Result<(), String> {
    let base = cache()?;
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    fs::create_dir_all(base.join("locks")).map_err(|e| e.to_string())?;
    let _admission =
        super::queue::Queue::admission(&base.join("locks")).map_err(|e| e.to_string())?;
    let targets = targets(&base)?;
    let free = disk_free(&base)?;
    let configuration = configuration(root, &[])?;
    let reserve = warm_reserve(&base, &targets, &configuration)?;
    let state = Reserve::read(&base);
    let qualified = state.measured > 0 && state.configuration == configuration;
    let used = targets
        .iter()
        .fold(0u64, |sum, target| sum.saturating_add(target.bytes));
    println!(
        "{}",
        serde_json::json!({
            "capacity": directory_cap(free, used, reserve), "directories": targets.len(),
            "warm_bytes": reserve, "bootstrap_bytes": CACHE_SLOT_LIMIT,
            "reserve_source": if qualified { "measured_workspace_tests" } else { "bootstrap" },
            "qualified_warm_bytes": if qualified { Some(state.measured) } else { None },
            "observed_peak_bytes": state.peak,
            "configuration": configuration, "free_bytes": free,
            "floor_bytes": DISK_FLOOR,
            "targets": targets.iter().map(|target| serde_json::json!({
                "directory": base.join("target").join(&target.name),
                "checkout": target.root, "bytes": target.bytes,
                "removed": !target.root.is_dir(), "owner_valid": target.valid_owner, "in_use": target.busy
            })).collect::<Vec<_>>()
        })
    );
    Ok(())
}

/// Caller holds this checkout's target lock. Admission serializes the capacity
/// check and publication, so concurrent first builds cannot exceed the cap.
pub(super) fn prepare(
    base: &Path,
    root: &Path,
    args: &[std::ffi::OsString],
    mut free_space: impl FnMut(&Path) -> Result<u64, String>,
) -> Result<PathBuf, String> {
    let name = target_name(root)?;
    let target = base.join("target").join(&name);
    let configuration = configuration(root, args)?;
    let mut waiting = false;
    let admission = loop {
        let admission =
            super::queue::Queue::admission(&base.join("locks")).map_err(|e| e.to_string())?;
        let entries = targets(base)?;
        let reserve = warm_reserve(base, &entries, &configuration)?;
        let free = free_space(base)?;
        let used = entries
            .iter()
            .fold(0u64, |sum, target| sum.saturating_add(target.bytes));
        let cap = directory_cap(free, used, reserve);
        if free < DISK_FLOOR || (cap == 0 && !target.is_dir()) {
            return Err(format!(
                "build refused: checkout target capacity {cap} ({} directories), 30 GiB disk floor; run cargo xtask prune. Candidates:\n{}",
                entries.len(),
                candidates(&entries)
            ));
        }
        if target.is_dir() || (entries.len() as u64) < cap {
            break admission;
        }
        // At capacity: evict the oldest idle verified target and create this
        // checkout's target under the same admission hold, so no other build
        // can take the freed room.
        if evict_one_idle(base, &entries)? {
            break admission;
        }
        if !waiting {
            eprintln!(
                "[build] checkout target capacity {cap} reached and every target is in use; waiting for one to go idle:\n{}",
                candidates(&entries)
            );
            waiting = true;
        }
        drop(admission);
        std::thread::sleep(TARGET_WAIT);
    };
    let fresh = !target.is_dir();
    if !fresh {
        let recorded = fs::read_to_string(base.join("locks").join(format!("{name}.owner")))
            .map_err(|e| format!("checkout target owner cannot be read: {e}"))?;
        let canonical = fs::canonicalize(root).map_err(|e| e.to_string())?;
        if fs::canonicalize(recorded).ok().as_ref() != Some(&canonical) {
            return Err(format!(
                "build refused: checkout hash collision or damaged owner for {name}; inspect its owner record"
            ));
        }
    }
    fs::write(
        base.join("locks").join(format!("{name}.owner")),
        fs::canonicalize(root)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("checkout path is not UTF-8")?,
    )
    .map_err(|e| e.to_string())?;
    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    open(&base.join("locks").join(format!("{name}.used")))
        .and_then(|file| file.set_modified(SystemTime::now()))
        .map_err(|e| e.to_string())?;
    drop(admission);
    if free_space(base)? < DISK_FLOOR {
        return Err(
            "build refused: disk below 30 GiB after checkout target preparation; run cargo xtask prune".into(),
        );
    }
    Ok(target)
}

/// How long a build at full target capacity waits before checking again.
pub(super) const TARGET_WAIT: Duration = Duration::from_secs(2);

/// Caller holds admission. Removes the first idle target with a verified owner,
/// in `targets` order (removed checkouts first, then least recently used).
fn evict_one_idle(base: &Path, entries: &[Target]) -> Result<bool, String> {
    for entry in entries.iter().filter(|entry| entry.valid_owner) {
        let Some(_held) = open(&base.join("locks").join(&entry.name))
            .and_then(try_lock)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        remove_slot(
            &base.join("target").join(&entry.name),
            false,
            "making room for a checkout at target capacity",
        )?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn prune_targets(
    base: &Path,
    now: SystemTime,
    free: u64,
    dry_run: bool,
    make_room: bool,
    configuration: &str,
) -> Result<(), String> {
    let _admission =
        super::queue::Queue::admission(&base.join("locks")).map_err(|e| e.to_string())?;
    let entries = targets(base)?;
    let reserve = warm_reserve(base, &entries, configuration)?;
    let used = entries
        .iter()
        .fold(0u64, |sum, target| sum.saturating_add(target.bytes));
    let cap = directory_cap(free, used, reserve);
    let mut count = entries.len() as u64;
    let mut room_made = false;
    for entry in entries {
        if !entry.valid_owner {
            eprintln!(
                "[cleanup] {} has an unverified owner; keeping it",
                entry.name
            );
            continue;
        }
        let Some(_held) = open(&base.join("locks").join(&entry.name))
            .and_then(try_lock)
            .map_err(|e| e.to_string())?
        else {
            eprintln!("[cleanup] {} is in use; keeping it", entry.name);
            continue;
        };
        let path = base.join("target").join(&entry.name);
        let extra = make_room && !room_made && cap > 0 && count >= cap;
        if !entry.root.is_dir() || (!make_room && count > cap) || extra {
            remove_slot(
                &path,
                dry_run,
                if extra && entry.root.is_dir() {
                    "explicitly making room for one checkout"
                } else {
                    "checkout removed or above directory capacity"
                },
            )?;
            count = count.saturating_sub(1);
            room_made |= extra && entry.root.is_dir();
        } else if !make_room {
            prune_incrementals(&path, now, dry_run).map_err(|e| e.to_string())?;
            if entry.bytes > CACHE_SLOT_LIMIT {
                let freed = prune_deps(&path, now, entry.bytes, CACHE_SLOT_LIMIT, dry_run)
                    .map_err(|e| e.to_string())?;
                if entry.bytes.saturating_sub(freed) > CACHE_SLOT_LIMIT {
                    remove_slot(&path, dry_run, "above 40 GiB after dependency pruning")?;
                }
            }
        }
    }
    Ok(())
}
