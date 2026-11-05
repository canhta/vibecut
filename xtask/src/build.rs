//! Cross-process FIFO build admission, warm slots and clean-tree CI receipts.
#[cfg(any(unix, test))]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::time::{Duration, Instant, SystemTime};

const SLOTS: usize = 3;
const MIN_FREE_PERCENT: u32 = 30;
const START_RAMP: Duration = Duration::from_secs(20);
const GIB: u64 = 1 << 30;

struct Lock {
    file: File,
    unlock_on_drop: bool,
}

impl Lock {
    fn new(file: File) -> Self {
        Self {
            file,
            unlock_on_drop: true,
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if self.unlock_on_drop {
            let _ = self.file.unlock();
        }
    }
}

fn open(path: &Path) -> io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn try_lock(file: File) -> io::Result<Option<Lock>> {
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock::new(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

struct Queue {
    locks: PathBuf,
    path: PathBuf,
    held: Option<Lock>,
}

impl Queue {
    fn enter(locks: &Path) -> io::Result<Self> {
        fs::create_dir_all(locks.join("build-queue"))?;
        // Serialize creation + locking, dead-entry pruning, and admission. No
        // observer can mistake a newly created, not-yet-locked waiter for dead.
        let admission = Self::admission(locks)?;
        // Derive the next number under the admission lock. No mutable counter
        // can be truncated by a crash, and no wall-clock adjustment reorders it.
        let mut largest = 0u64;
        for entry in fs::read_dir(locks.join("build-queue"))? {
            let name = entry?.file_name();
            let number = name
                .to_str()
                .ok_or_else(|| io::Error::other("invalid queue filename"))?
                .parse::<u64>()
                .map_err(io::Error::other)?;
            largest = largest.max(number);
        }
        let next = largest
            .checked_add(1)
            .ok_or_else(|| io::Error::other("build queue exhausted"))?;
        let path = locks.join("build-queue").join(format!("{next:020}"));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.lock()?;
        drop(admission);
        Ok(Self {
            locks: locks.to_path_buf(),
            path,
            held: Some(Lock::new(file)),
        })
    }

    fn admission(locks: &Path) -> io::Result<Lock> {
        let file = open(&locks.join("admission"))?;
        file.lock()?;
        Ok(Lock::new(file))
    }

    fn first(&self) -> io::Result<bool> {
        let mut paths = fs::read_dir(self.locks.join("build-queue"))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if path == self.path {
                return Ok(true);
            }
            // A held lock is the liveness signal. PIDs are
            // neither used nor trusted; a crashed waiter leaves an unlocked file.
            let file = File::options().read(true).write(true).open(&path)?;
            match try_lock(file)? {
                None => return Ok(false),
                Some(dead) => {
                    drop(dead);
                    fs::remove_file(path)?;
                }
            }
        }
        Err(io::Error::other("build queue entry disappeared"))
    }

    fn leave(&mut self) -> io::Result<()> {
        // Close before unlink. Caller holds admission.
        drop(self.held.take());
        fs::remove_file(&self.path)
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        if self.held.is_some()
            && let Ok(_admission) = Self::admission(&self.locks)
        {
            let _ = self.leave();
        }
    }
}

struct Slot {
    lock: Lock,
    number: usize,
}

impl Slot {
    #[cfg(unix)]
    fn execute(&mut self, command: &mut Command) -> io::Result<ExitStatus> {
        // Only this slot descriptor crosses exec. Queue/admission descriptors
        // retain CLOEXEC. Closing the launcher must not explicitly unlock the
        // shared open-file description while a child/descendant still holds it.
        self.lock.unlock_on_drop = false;
        let file = &self.lock.file;
        let flags = rustix::io::fcntl_getfd(file)?;
        rustix::io::fcntl_setfd(file, flags & !rustix::io::FdFlags::CLOEXEC)?;
        let child = command.spawn();
        // Descriptor flags are per-process; restore the parent's CLOEXEC before
        // spawning receipt commands, without changing the child's copy.
        let restored = rustix::io::fcntl_setfd(file, flags);
        // Even if flag restoration fails, wait for a successfully spawned child.
        let status = child?.wait();
        restored?;
        status
    }

    #[cfg(windows)]
    fn execute(&mut self, command: &mut Command) -> io::Result<ExitStatus> {
        // Keep the lease until wait observes exit, including forced child exit.
        // Windows launcher death is a documented limitation: std cannot safely
        // inherit this lease and winsafe has no Job Object interface yet.
        let _lease = &self.lock;
        command.status()
    }
}

fn owner(locks: &Path, number: usize) -> PathBuf {
    fs::read_to_string(locks.join(format!("slot-{number}.owner")))
        .map(PathBuf::from)
        .unwrap_or_default()
}

// Each platform reads free physical memory from exactly one source. The
// `memory_available` dispatcher below is the only admission path: every gated
// platform feeds the same `MIN_FREE_PERCENT` threshold and `START_RAMP` in
// `admit_extra`. macOS reads the `sysctl` level, Linux parses `/proc/meminfo`,
// Windows calls `GlobalMemoryStatusEx`. A source that cannot be read or parsed
// returns `false` (fail closed), which holds the extra build slot.

#[cfg(any(target_os = "macos", test))]
fn sysctl_memory_available(program: impl AsRef<OsStr>) -> bool {
    Command::new(program)
        .args(["-n", "kern.memorystatus_level"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u32>().ok())
        .is_some_and(|level| level >= MIN_FREE_PERCENT)
}

/// Read Linux free memory from `/proc/meminfo`. A missing or unreadable file
/// fails closed (holds the extra slot), as a failed macOS/Windows probe does.
#[cfg(any(target_os = "linux", test))]
fn proc_meminfo_available(path: impl AsRef<Path>) -> bool {
    fs::read_to_string(path).is_ok_and(|text| meminfo_memory_available(&text))
}

/// Pure `/proc/meminfo` parser: decide whether `MemAvailable` meets
/// `MIN_FREE_PERCENT` of `MemTotal`. A trustworthy line is exactly
/// `"Key: <u64> kB"`. Missing keys, a zero total, or any hostile value
/// (overflow, non-numeric, missing/wrong `kB` suffix, trailing tokens) fails
/// the whole probe rather than being ignored. `u128` keeps the comparison from
/// overflowing on adversarial `u64` values.
#[cfg(any(target_os = "linux", test))]
fn meminfo_memory_available(text: &str) -> bool {
    let (mut total_kb, mut available_kb) = (None, None);
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let slot = match fields.next() {
            Some("MemTotal:") => &mut total_kb,
            Some("MemAvailable:") => &mut available_kb,
            _ => continue,
        };
        let value = fields.next();
        let unit = fields.next();
        let parsed = match (value, unit) {
            (Some(value), Some("kB")) if fields.next().is_none() => value.parse::<u64>().ok(),
            _ => None,
        };
        match parsed {
            Some(kb) => *slot = Some(kb),
            None => return false,
        }
    }
    match (total_kb, available_kb) {
        (Some(total), Some(available)) if total > 0 && available <= total => {
            u128::from(available) * 100 >= u128::from(total) * u128::from(MIN_FREE_PERCENT)
        }
        _ => false,
    }
}

fn memory_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        sysctl_memory_available("sysctl")
    }
    #[cfg(target_os = "linux")]
    {
        proc_meminfo_available("/proc/meminfo")
    }
    #[cfg(windows)]
    {
        winsafe::GlobalMemoryStatusEx().is_ok_and(|memory| {
            memory.ullTotalPhys > 0
                && u128::from(memory.ullAvailPhys) * 100
                    >= u128::from(memory.ullTotalPhys) * u128::from(MIN_FREE_PERCENT)
        })
    }
    // Unix platforms without a memory probe keep the historical "admit by
    // available slots alone" behaviour: `run_in` passes `memory_gated = false`
    // there, so this dispatcher is never called. Fail closed regardless.
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        false
    }
}

fn admit_extra(
    memory_gated: bool,
    busy: bool,
    last_start: Option<SystemTime>,
    memory: impl FnOnce() -> bool,
) -> bool {
    if !memory_gated || !busy {
        return true;
    }
    last_start
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age >= START_RAMP)
        && memory()
}

fn claim(
    queue: &mut Queue,
    root: &Path,
    memory_gated: bool,
    memory: impl FnOnce() -> bool,
) -> io::Result<Option<Slot>> {
    let _admission = Queue::admission(&queue.locks)?;
    if !queue.first()? {
        return Ok(None);
    }
    let mut free = Vec::new();
    for number in 1..=SLOTS {
        if let Some(lock) = try_lock(open(&queue.locks.join(format!("slot-{number}")))?)? {
            free.push((number, lock));
        }
    }
    let busy = free.len() < SLOTS;
    let last = fs::metadata(queue.locks.join("last-start"))
        .and_then(|m| m.modified())
        .ok();
    if free.is_empty() || !admit_extra(memory_gated, busy, last, memory) {
        return Ok(None);
    }
    free.sort_by_key(|(number, _)| (owner(&queue.locks, *number) != root, *number));
    let (number, mut lock) = free
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("no build slot"))?;
    // Empty writes do not reliably advance mtime on Windows. The ramp depends
    // on the admission time, so set it explicitly while admission is locked.
    open(&queue.locks.join("last-start"))?.set_modified(SystemTime::now())?;
    lock.file.set_len(0)?;
    writeln!(lock.file, "{}", root.display())?;
    queue.leave()?;
    Ok(Some(Slot { lock, number }))
}

fn cache() -> Result<PathBuf, String> {
    // Same per-user cache owner as native provisioning.
    crate::ffmpeg::cache()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cache has no parent".into())
}

const STALE_AGE: Duration = Duration::from_secs(2 * 24 * 60 * 60);
// A warm slot holds 14-21 GiB after one workspace test build; stay above it so cleanup keeps warm caches.
pub(crate) const CACHE_SLOT_LIMIT: u64 = 40 * GIB;

/// Prune the shared build cache: sessions and whole slots untouched for two
/// days go first; an over-budget free slot drops stale dependencies before
/// being cleared under its lock if it still exceeds the budget.
/// Held slots are left alone because a build is using their artifacts.
pub(crate) fn prune_cache(dry_run: bool) -> Result<(), String> {
    prune_slots(&cache()?, SystemTime::now(), CACHE_SLOT_LIMIT, dry_run)
}

fn older_than(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= STALE_AGE)
}

/// Cache entries are shared with live builds. A path that disappears or stops
/// being a directory between two calls is simply no longer ours to inspect.
fn changed_under_us(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

fn newest_modified(path: &Path) -> io::Result<Option<SystemTime>> {
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

fn remove_entry(path: &Path) -> io::Result<()> {
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

fn prune_incrementals(slot: &Path, now: SystemTime, dry_run: bool) -> io::Result<()> {
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

/// Remove build artifacts older than the stale age under every profile's
/// `deps` directory. Runs only for an idle slot that is over budget, so a warm
/// slot is preserved when its stale entries alone bring it back under the
/// limit. Returns the bytes removed (or that a dry run would remove).
fn prune_stale_deps(path: &Path, depth: usize, now: SystemTime, dry_run: bool) -> io::Result<u64> {
    if depth == 0 {
        return Ok(0);
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if changed_under_us(&error) => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut freed = 0;
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
            freed += prune_stale_dep_entries(&path, now, dry_run)?;
        } else {
            freed += prune_stale_deps(&path, depth - 1, now, dry_run)?;
        }
    }
    Ok(freed)
}

fn prune_stale_dep_entries(deps: &Path, now: SystemTime, dry_run: bool) -> io::Result<u64> {
    let entries = match fs::read_dir(deps) {
        Ok(entries) => entries,
        Err(error) if changed_under_us(&error) => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut freed = 0;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if changed_under_us(&error) => continue,
            Err(error) => return Err(error),
        };
        let path = entry.path();
        let stale = newest_modified(&path)?.is_some_and(|modified| older_than(modified, now));
        if !stale {
            continue;
        }
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
        if dry_run {
            eprintln!("[cleanup] would remove stale dependency {}", path.display());
        } else {
            eprintln!("[cleanup] remove stale dependency {}", path.display());
            remove_entry(&path)?;
        }
        freed += bytes;
    }
    Ok(freed)
}

fn remove_slot(slot: &Path, dry_run: bool, reason: &str) -> Result<(), String> {
    if dry_run {
        eprintln!("[cleanup] would remove {} ({reason})", slot.display());
    } else {
        eprintln!("[cleanup] remove {} ({reason})", slot.display());
        remove_entry(slot).map_err(|error| format!("{}: {error}", slot.display()))?;
    }
    Ok(())
}

fn prune_slots(base: &Path, now: SystemTime, size_limit: u64, dry_run: bool) -> Result<(), String> {
    let target = base.join("target");
    if !target.is_dir() {
        return Ok(());
    }
    let locks = base.join("locks");
    fs::create_dir_all(&locks).map_err(|error| format!("{}: {error}", locks.display()))?;
    for number in 1..=SLOTS {
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
            let freed = prune_stale_deps(&slot, 4, now, dry_run)
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
                        "{:.1} GiB over the size budget",
                        remaining as f64 / GIB as f64
                    ),
                )?;
            } else if freed > 0 {
                eprintln!(
                    "[cleanup] slot retained after pruning {:.1} GiB of stale dependencies",
                    freed as f64 / GIB as f64
                );
            }
        }
    }
    Ok(())
}

fn used_bytes(path: &Path) -> io::Result<u64> {
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

fn file_bytes(metadata: &fs::Metadata) -> u64 {
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
fn disk_free(path: &Path) -> Result<u64, String> {
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
fn disk_free(path: &Path) -> Result<u64, String> {
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

fn remove_dir(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn prepare(
    base: &Path,
    root: &Path,
    number: usize,
    mut free_space: impl FnMut(&Path) -> Result<u64, String>,
) -> Result<PathBuf, String> {
    let locks = base.join("locks");
    let target = base.join(format!("target/slot-{number}"));
    let switched = owner(&locks, number) != root;
    let used = if switched {
        used_bytes(&base.join("target")).map_err(|e| e.to_string())?
    } else {
        0
    };
    if used > 50 * GIB || free_space(base)? < 30 * GIB {
        eprintln!("[build] cache above 50 GiB or disk below 30 GiB free; clearing slot-{number}");
        remove_dir(&target).map_err(|e| e.to_string())?;
        // Never start a build when reclaiming this slot cannot restore headroom.
        if free_space(base)? < 30 * GIB {
            return Err("build refused: disk still below 30 GiB free after slot cleanup".into());
        }
    }
    if switched {
        // Invalidate ownership before pruning/touching so interruption retries the
        // entire switch, rather than admitting stale cross-worktree artifacts.
        let owner_file = locks.join(format!("slot-{number}.owner"));
        fs::write(&owner_file, b"").map_err(|e| e.to_string())?;
        let manifests = crate::checked(root, "git", &["ls-files", "-z", "*/Cargo.toml"])?;
        let members: Vec<_> = paths(&manifests)?
            .iter()
            .filter_map(|path| path.parent()?.file_name()?.to_str())
            .map(|name| name.replace('-', "_"))
            .collect();
        let deps = target.join("debug/deps");
        if deps.is_dir() {
            for entry in fs::read_dir(&deps).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let stem = name
                    .strip_prefix("lib")
                    .unwrap_or(&name)
                    .split('-')
                    .next()
                    .unwrap_or("");
                // ponytail: name matching also drops the third-party time rlib;
                // exact package IDs if that small rebuild becomes material.
                if name.ends_with(".o")
                    || !name.contains('.')
                    || name.ends_with(".exe")
                    || members.iter().any(|member| member == stem)
                {
                    let result = if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                        remove_dir(&entry.path())
                    } else {
                        fs::remove_file(entry.path())
                    };
                    if let Err(e) = result
                        && e.kind() != io::ErrorKind::NotFound
                    {
                        return Err(e.to_string());
                    }
                }
            }
        }
        let tracked = crate::checked(root, "git", &["ls-files", "-z"])?;
        let now = SystemTime::now();
        for path in paths(&tracked)? {
            // The launcher has a per-worktree target and is already fresh.
            // Touching it would needlessly rebuild the executing launcher in
            // a nested `cargo xtask ci`.
            if path.starts_with("xtask") {
                continue;
            }
            let path = root.join(path);
            // Ignore deleted working-tree files and symlinks, never touch a
            // symlink's target outside the checkout.
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() => File::options()
                    .write(true)
                    .open(&path)
                    .and_then(|file| file.set_modified(now))
                    .map_err(|e| e.to_string())?,
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.to_string()),
            }
        }
        fs::write(
            owner_file,
            root.to_str().ok_or("checkout path is not UTF-8")?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(target)
}

#[cfg(unix)]
fn paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    use std::os::unix::ffi::OsStrExt;
    Ok(bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|bytes| PathBuf::from(OsStr::from_bytes(bytes)))
        .collect())
}

#[cfg(windows)]
fn paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    std::str::from_utf8(bytes)
        .map(|text| {
            text.split('\0')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .map_err(|error| format!("Git path is not UTF-8: {error}"))
}

fn clean_tree(root: &Path) -> Result<Option<String>, String> {
    if !crate::checked(
        root,
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty()
    {
        return Ok(None);
    }
    String::from_utf8(crate::checked(root, "git", &["rev-parse", "HEAD^{tree}"])?)
        .map(|tree| Some(tree.trim().to_owned()))
        .map_err(|e| e.to_string())
}

fn receipt(root: &Path, base: &Path, args: &[OsString]) -> Result<Option<PathBuf>, String> {
    if args.first().is_none_or(|s| s != "cargo")
        || args.get(1).is_none_or(|s| s != "xtask")
        || args.get(2).is_none_or(|s| s != "ci")
    {
        return Ok(None);
    }
    let mode = match args.get(3..) {
        Some([]) => "plain",
        Some([flag]) if flag == "--require-oracles" => "oracles",
        _ => return Ok(None),
    };
    Ok(clean_tree(root)?.map(|tree| base.join(format!("ci-receipts/{tree}-{mode}"))))
}

pub fn run(root: &Path, args: &[OsString]) -> Result<ExitCode, String> {
    run_in(
        root,
        args,
        cache()?,
        cfg!(any(target_os = "macos", target_os = "linux", windows)),
        disk_free,
        crate::ffmpeg::environment,
    )
}

fn run_in(
    root: &Path,
    args: &[OsString],
    base: PathBuf,
    memory_gated: bool,
    free_space: impl FnMut(&Path) -> Result<u64, String>,
    native: impl FnOnce(&mut Command) -> Result<(), String>,
) -> Result<ExitCode, String> {
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let before = receipt(root, &base, args)?;
    if let Some(path) = &before
        && let Ok(text) = fs::read_to_string(path)
    {
        eprintln!("[build] CI already green for this tree: {}", text.trim());
        return Ok(ExitCode::SUCCESS);
    }
    let started = Instant::now();
    let mut queue = Queue::enter(&base.join("locks")).map_err(|e| e.to_string())?;
    let mut said = false;
    let mut slot = loop {
        if let Some(slot) =
            claim(&mut queue, root, memory_gated, memory_available).map_err(|e| e.to_string())?
        {
            break slot;
        }
        if !said {
            eprintln!("[build] waiting (FIFO; extra builds need 30% free memory and 20s ramp)");
            said = true;
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let target = prepare(&base, root, slot.number, free_space)?;
    eprintln!(
        "[build] slot-{} after {:.0}s wait",
        slot.number,
        started.elapsed().as_secs_f64()
    );
    // Recheck after queueing and after completion: edits while waiting/building
    // cannot mint a receipt for a different or dirty tree.
    let before =
        before.filter(|path| receipt(root, &base, args).ok().flatten().as_ref() == Some(path));
    let mut command = Command::new(args.first().ok_or("missing build command")?);
    command
        .args(args.get(1..).unwrap_or(&[]))
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target)
        .env_remove("RUSTC_WRAPPER");
    if std::env::var_os("CARGO_BUILD_JOBS").is_none() {
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        command.env("CARGO_BUILD_JOBS", (cpus / SLOTS).max(2).to_string());
    }
    // The wrapper owns both the warm slot and the provisioned native
    // environment: compile-time discovery and runtime library loading must
    // agree with every command-specific xtask entry point.
    native(&mut command)?;
    let status = slot
        .execute(&mut command)
        .map_err(|e| format!("build command: {e}"))?;
    if status.success()
        && let Some(path) = before
        && receipt(root, &base, args)?.as_ref() == Some(&path)
    {
        fs::create_dir_all(base.join("ci-receipts")).map_err(|e| e.to_string())?;
        let sha = String::from_utf8(crate::checked(
            root,
            "git",
            &["rev-parse", "--short", "HEAD"],
        )?)
        .map_err(|e| e.to_string())?;
        fs::write(
            path,
            format!(
                "cargo xtask ci passed at {} ({:?})\n",
                sha.trim(),
                SystemTime::now()
            ),
        )
        .map_err(|e| e.to_string())?;
    }
    drop(slot);
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(
            status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(1),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "vibecut build thử-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn wait(path: &Path) {
        let start = Instant::now();
        while !path.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn worker(root: &Path, label: &str) -> Process {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "build::tests::build_process", "--nocapture"])
            .env("VIBECUT_BUILD_TEST_ROOT", root)
            .env("VIBECUT_BUILD_TEST_LABEL", label)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let process = Process(child);
        wait(&root.join(format!("{label}.ready")));
        process
    }

    // The same public test executable gives subprocesses independent OS handles.
    #[test]
    fn build_process() {
        let Some(root) = std::env::var_os("VIBECUT_BUILD_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let label = std::env::var("VIBECUT_BUILD_TEST_LABEL").unwrap();
        if label == "missing" {
            let args = vec![root.join("missing-command").into_os_string()];
            assert!(
                run_in(
                    &root,
                    &args,
                    root.clone(),
                    false,
                    |_| Ok(31 * GIB),
                    |_| Ok(())
                )
                .is_err()
            );
            return;
        }
        if label == "native" {
            let args = vec![
                std::env::current_exe().unwrap().into_os_string(),
                "--exact".into(),
                "build::tests::native_environment_probe".into(),
                "--nocapture".into(),
            ];
            assert_eq!(
                run_in(
                    &root,
                    &args,
                    root.clone(),
                    false,
                    |_| Ok(31 * GIB),
                    |command| {
                        command.env("VIBECUT_BUILD_TEST_NATIVE", "provisioned");
                        Ok(())
                    }
                )
                .unwrap(),
                ExitCode::SUCCESS
            );
            return;
        }
        if matches!(label.as_str(), "wrapped" | "detached" | "killed") {
            let args = vec![
                std::env::current_exe().unwrap().into_os_string(),
                "--exact".into(),
                "build::tests::controlled_command".into(),
                "--nocapture".into(),
            ];
            assert_eq!(
                // Lease/process lifetime is independent of host disk capacity.
                // The cache test below covers refusal with injected low space.
                run_in(
                    &root,
                    &args,
                    root.clone(),
                    false,
                    |_| Ok(31 * GIB),
                    |_| Ok(())
                )
                .unwrap()
                    == ExitCode::SUCCESS,
                label != "killed"
            );
            return;
        }
        let mut queue = Queue::enter(&root.join("locks")).unwrap();
        fs::write(root.join(format!("{label}.ready")), b"").unwrap();
        let slot = loop {
            if let Some(slot) = claim(&mut queue, &root, false, || false).unwrap() {
                break slot;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let acquired = root.join(format!("{label}.acquired"));
        let pending = acquired.with_extension("pending");
        fs::write(&pending, slot.number.to_string()).unwrap();
        fs::rename(pending, acquired).unwrap();
        wait(&root.join(format!("{label}.release")));
        drop(slot);
    }

    #[test]
    #[allow(
        clippy::zombie_processes,
        reason = "this subprocess exits immediately to exercise a descendant outliving its command; the test owns the descendant's input pipe"
    )]
    fn controlled_command() {
        use std::io::Read;
        let Some(root) = std::env::var_os("VIBECUT_BUILD_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        if std::env::var("VIBECUT_BUILD_TEST_LABEL").as_deref() == Ok("detached") {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "build::tests::controlled_command", "--nocapture"])
                .env("VIBECUT_BUILD_TEST_LABEL", "leaf")
                .spawn()
                .unwrap();
            return;
        }
        let _live = try_lock(open(&root.join("command-live")).unwrap())
            .unwrap()
            .unwrap();
        fs::write(root.join("command.pid"), std::process::id().to_string()).unwrap();
        fs::write(root.join("wrapped.ready"), b"").unwrap();
        // The parent owns the write end even after the wrapper dies. Closing it
        // also guarantees cleanup if an assertion unwinds in the parent test.
        let _ = std::io::stdin().read(&mut [0u8]);
    }

    #[test]
    fn native_environment_probe() {
        let Some(root) = std::env::var_os("VIBECUT_BUILD_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::write(
            root.join("native.environment"),
            std::env::var("VIBECUT_BUILD_TEST_NATIVE").unwrap_or_default(),
        )
        .unwrap();
        fs::write(
            root.join("native.jobs"),
            std::env::var("CARGO_BUILD_JOBS").unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("native.target"),
            std::env::var("CARGO_TARGET_DIR").unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn fifo_survives_crashed_waiter_and_holder_without_overbooking_last_slot() {
        let directory = Directory::new();
        let root = &directory.0;
        let locks = root.join("locks");
        let mut slots = Vec::new();
        for _ in 0..SLOTS {
            let mut queue = Queue::enter(&locks).unwrap();
            slots.push(claim(&mut queue, root, false, || false).unwrap().unwrap());
        }
        let mut dead = worker(root, "dead");
        let mut first = worker(root, "first");
        let mut second = worker(root, "second");
        assert!(!root.join("first.acquired").exists());
        assert!(!root.join("second.acquired").exists());
        dead.0.kill().unwrap();
        dead.0.wait().unwrap();
        let freed = slots.pop().unwrap().number;
        // Drop the popped lock above; the oldest live waiter must win it.
        wait(&root.join("first.acquired"));
        assert_eq!(
            fs::read_to_string(root.join("first.acquired")).unwrap(),
            freed.to_string()
        );
        assert!(!root.join("second.acquired").exists());
        first.0.kill().unwrap();
        first.0.wait().unwrap();
        wait(&root.join("second.acquired"));
        assert_eq!(
            fs::read_to_string(root.join("second.acquired")).unwrap(),
            freed.to_string()
        );
        fs::write(root.join("second.release"), b"").unwrap();
        assert!(second.0.wait().unwrap().success());
        assert_eq!(fs::read_dir(locks.join("build-queue")).unwrap().count(), 0);

        // Exercise the actual wrapper command path, not just a process holding
        // a Slot: its controlled build child survives a launcher-only crash.
        crate::checked(root, "git", &["init", "-q"]).unwrap();
        for jobs in [None, Some("7")] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "build::tests::build_process", "--nocapture"])
                .env("VIBECUT_BUILD_TEST_ROOT", root)
                .env("VIBECUT_BUILD_TEST_LABEL", "native")
                .env_remove("CARGO_BUILD_JOBS");
            if let Some(jobs) = jobs {
                command.env("CARGO_BUILD_JOBS", jobs);
            }
            let native = command.status().unwrap();
            assert!(native.success());
            assert_eq!(
                fs::read_to_string(root.join("native.environment")).unwrap(),
                "provisioned",
                "wrapper did not pass the native environment to its command"
            );
            let expected = jobs.map(str::to_owned).unwrap_or_else(|| {
                (std::thread::available_parallelism().map_or(1, usize::from) / SLOTS)
                    .max(2)
                    .to_string()
            });
            assert_eq!(
                fs::read_to_string(root.join("native.jobs")).unwrap(),
                expected
            );
            assert_eq!(
                PathBuf::from(fs::read_to_string(root.join("native.target")).unwrap()),
                root.join("target/slot-3")
            );
        }
        #[cfg(unix)]
        for label in ["wrapped", "detached"] {
            let _ = fs::remove_file(root.join("wrapped.ready"));
            let mut wrapped = Process(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "build::tests::build_process", "--nocapture"])
                    .env("VIBECUT_BUILD_TEST_ROOT", root)
                    .env("VIBECUT_BUILD_TEST_LABEL", label)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let input = wrapped.0.stdin.take().unwrap();
            wait(&root.join("wrapped.ready"));
            if label == "wrapped" {
                wrapped.0.kill().unwrap();
                wrapped.0.wait().unwrap();
            } else {
                assert!(wrapped.0.wait().unwrap().success());
            }
            let child_is_alive = try_lock(open(&root.join("command-live")).unwrap())
                .unwrap()
                .is_none();
            assert!(
                child_is_alive,
                "controlled build must still be waiting on its input pipe"
            );
            let mut queued = Queue::enter(&locks).unwrap();
            assert!(
                claim(&mut queued, root, false, || false).unwrap().is_none(),
                "wrapper death made the surviving command's slot reusable"
            );
            drop(input);
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if claim(&mut queued, root, false, || false).unwrap().is_some() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "slot did not release after command exit"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        // Production launches commands from a single-threaded process. Keep
        // this case out of the parallel test harness, where another test's
        // subprocess could inherit the briefly non-CLOEXEC slot descriptor.
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "build::tests::build_process", "--nocapture"])
            .env("VIBECUT_BUILD_TEST_ROOT", root)
            .env("VIBECUT_BUILD_TEST_LABEL", "missing")
            .status()
            .unwrap();
        assert!(status.success());
        let mut queued = Queue::enter(&locks).unwrap();
        assert!(
            claim(&mut queued, root, false, || false).unwrap().is_some(),
            "failed spawn retained its slot"
        );
    }

    #[test]
    fn memory_probe_failure_blocks_only_extra_slots_and_observes_ramp() {
        let directory = Directory::new();
        assert!(!sysctl_memory_available(directory.0.join("missing-sysctl")));
        let old = SystemTime::now() - Duration::from_secs(30);
        for (memory_gated, busy, start, memory, expected) in [
            (true, false, None, false, true),
            (true, true, Some(old), false, false),
            (true, true, Some(old), true, true),
            (true, true, Some(SystemTime::now()), true, false),
            (true, true, None, true, false),
            (false, true, None, false, true),
        ] {
            assert_eq!(admit_extra(memory_gated, busy, start, || memory), expected);
        }

        let root = &directory.0;
        let locks = root.join("locks");
        let mut first = Queue::enter(&locks).unwrap();
        let held = claim(&mut first, root, true, || false).unwrap().unwrap();
        let mut second = Queue::enter(&locks).unwrap();
        let mut third = Queue::enter(&locks).unwrap();
        File::options()
            .write(true)
            .open(locks.join("last-start"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(claim(&mut second, root, true, || false).unwrap().is_none());
        assert!(
            claim(&mut third, root, true, || true).unwrap().is_none(),
            "low memory must not let a later waiter overtake the second build"
        );
        let admitted = claim(&mut second, root, true, || true).unwrap().unwrap();
        assert_ne!(held.number, admitted.number);
        assert!(
            claim(&mut third, root, true, || true).unwrap().is_none(),
            "the next build must observe the ramp again"
        );
    }

    // Must 1/3: Linux reads `/proc/meminfo` `MemTotal`/`MemAvailable` with the
    // same 30% `MIN_FREE_PERCENT` as the other platforms. One parser covers the
    // normal file, missing keys, zero total, hostile values and the exact
    // threshold boundary; every malformed input fails closed.
    #[test]
    fn linux_meminfo_parser_covers_normal_missing_zero_hostile_and_threshold() {
        let normal = concat!(
            "MemTotal:       1000 kB\n",
            "MemFree:         100 kB\n",
            "MemAvailable:    900 kB\n",
            "Buffers:          50 kB\n",
            "Cached:          700 kB\n",
        );
        let meminfo = |total: &str, available: &str| {
            format!("MemTotal: {total} kB\nMemAvailable: {available} kB\n")
        };
        // 30% of 1000 = 300, so 300 admits (>=) and 299 holds the slot.
        // A realistic kB scale (host ~ 64 GiB) exercises the u128 comparison.
        for (label, text, expected) in [
            ("normal-high-free", normal.to_string(), true),
            ("normal-low-free", meminfo("1000", "100"), false),
            (
                "boundary-exactly-at-threshold",
                meminfo("1000", "300"),
                true,
            ),
            (
                "boundary-one-below-threshold",
                meminfo("1000", "299"),
                false,
            ),
            (
                "boundary-realistic-at-threshold",
                meminfo("64639456", "19391837"),
                true,
            ),
            (
                "boundary-realistic-one-below",
                meminfo("64639456", "19391836"),
                false,
            ),
            (
                "missing-MemTotal",
                "MemAvailable: 900 kB\n".to_string(),
                false,
            ),
            (
                "missing-MemAvailable",
                "MemTotal: 1000 kB\n".to_string(),
                false,
            ),
            ("empty-file", String::new(), false),
            ("no-relevant-keys", "MemFree: 500 kB\n".to_string(), false),
            ("zero-total", meminfo("0", "0"), false),
            ("zero-total-with-available", meminfo("0", "100"), false),
            (
                "overflow-total",
                meminfo("99999999999999999999999999999999", "900"),
                false,
            ),
            (
                "overflow-available",
                meminfo("1000", "99999999999999999999999999999999"),
                false,
            ),
            ("non-numeric-total", meminfo("not_a_number", "900"), false),
            ("non-numeric-available", meminfo("1000", "NaN"), false),
            (
                "negative-value",
                "MemTotal: -1 kB\nMemAvailable: 900 kB\n".to_string(),
                false,
            ),
            (
                "kB-suffix-absent-total",
                "MemTotal: 1000\nMemAvailable: 900 kB\n".to_string(),
                false,
            ),
            (
                "kB-suffix-absent-available",
                "MemTotal: 1000 kB\nMemAvailable: 900\n".to_string(),
                false,
            ),
            (
                "wrong-unit",
                "MemTotal: 1000 MB\nMemAvailable: 900 kB\n".to_string(),
                false,
            ),
            (
                "trailing-tokens",
                "MemTotal: 1000 kB junk\nMemAvailable: 900 kB\n".to_string(),
                false,
            ),
            ("available-exceeds-total", meminfo("1000", "2000"), false),
        ] {
            assert_eq!(meminfo_memory_available(&text), expected, "{label}");
        }
    }

    // Must 2: a missing, unreadable or non-UTF-8 `/proc/meminfo` holds the
    // extra slot (fail closed), while a readable file goes through the same
    // 30% threshold as the parser above.
    #[test]
    fn linux_proc_meminfo_source_fails_closed_on_bad_file() {
        let directory = Directory::new();
        let good = directory.0.join("meminfo");
        fs::write(&good, "MemTotal: 1000 kB\nMemAvailable: 900 kB\n").unwrap();
        assert!(proc_meminfo_available(&good));
        let low = directory.0.join("low-meminfo");
        fs::write(&low, "MemTotal: 1000 kB\nMemAvailable: 100 kB\n").unwrap();
        assert!(!proc_meminfo_available(&low));
        // Missing path, a directory and invalid UTF-8 all fail closed.
        assert!(!proc_meminfo_available(directory.0.join("missing-meminfo")));
        assert!(!proc_meminfo_available(&directory.0));
        let binary = directory.0.join("binary-meminfo");
        fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert!(!proc_meminfo_available(&binary));
    }

    // Must 1/2: the Linux probe feeds the one shared admission path, so a
    // fail-closed parse holds the extra slot and sufficient memory admits it
    // only after the 20-second ramp, exactly like macOS and Windows.
    #[test]
    fn linux_memory_probe_uses_shared_admission_path() {
        let old = SystemTime::now() - Duration::from_secs(30);
        let good = "MemTotal: 1000 kB\nMemAvailable: 900 kB\n";
        let bad = "MemTotal: 0 kB\nMemAvailable: 0 kB\n";
        assert!(!admit_extra(true, true, Some(old), || {
            meminfo_memory_available(bad)
        }));
        assert!(admit_extra(true, true, Some(old), || {
            meminfo_memory_available(good)
        }));
        assert!(
            !admit_extra(true, true, Some(SystemTime::now()), || {
                meminfo_memory_available(good)
            }),
            "the ramp still gates extra admission on Linux"
        );
        // The first build is never gated; only extra slots consult the probe.
        assert!(admit_extra(true, false, None, || meminfo_memory_available(
            bad
        )));
    }

    #[test]
    fn child_exit_or_kill_releases_slot_lease() {
        for label in ["wrapped", "killed"] {
            let directory = Directory::new();
            let root = &directory.0;
            crate::checked(root, "git", &["init", "-q"]).unwrap();
            let mut wrapper = Process(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "build::tests::build_process", "--nocapture"])
                    .env("VIBECUT_BUILD_TEST_ROOT", root)
                    .env("VIBECUT_BUILD_TEST_LABEL", label)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let input = wrapper.0.stdin.take().unwrap();
            wait(&root.join("wrapped.ready"));
            assert!(
                try_lock(open(&root.join("locks/slot-1")).unwrap())
                    .unwrap()
                    .is_none()
            );
            if label == "killed" {
                let pid = fs::read_to_string(root.join("command.pid")).unwrap();
                #[cfg(windows)]
                let status = Command::new("taskkill")
                    .args(["/F", "/PID", pid.trim()])
                    .status()
                    .unwrap();
                #[cfg(unix)]
                let status = Command::new("kill")
                    .args(["-KILL", pid.trim()])
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            drop(input);
            assert!(wrapper.0.wait().unwrap().success());
            assert!(
                try_lock(open(&root.join("locks/slot-1")).unwrap())
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn ci_receipts_require_exact_command_and_clean_tree_including_untracked_files() {
        let directory = Directory::new();
        let root = &directory.0;
        crate::checked(root, "git", &["init", "-q"]).unwrap();
        crate::checked(
            root,
            "git",
            &[
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "empty",
                "--allow-empty",
            ],
        )
        .unwrap();
        let args: Vec<OsString> = ["cargo", "xtask", "ci"]
            .into_iter()
            .map(Into::into)
            .collect();
        let plain = receipt(root, root, &args).unwrap().unwrap();
        let mut required = args.clone();
        required.push("--require-oracles".into());
        assert_ne!(plain, receipt(root, root, &required).unwrap().unwrap());
        fs::write(root.join("untracked.rs"), b"pending source").unwrap();
        assert!(receipt(root, root, &args).unwrap().is_none());
        fs::remove_file(root.join("untracked.rs")).unwrap();
        let mut extra = args;
        extra.push("--bad-option".into());
        assert!(receipt(root, root, &extra).unwrap().is_none());
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn worktree_switch_prunes_old_binaries_and_disk_guard_refuses_unrecovered_headroom() {
        let root = std::env::temp_dir().join(format!("vibecut-switch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("crates/example")).unwrap();
        crate::checked(&root, "git", &["init", "-q"]).unwrap();
        fs::write(
            root.join("crates/example/Cargo.toml"),
            "[package]\nname = 'example'\n",
        )
        .unwrap();
        crate::checked(&root, "git", &["add", "."]).unwrap();
        let base = root.join("cache");
        fs::create_dir_all(base.join("locks")).unwrap();
        let deps = base.join("target/slot-1/debug/deps");
        fs::create_dir_all(&deps).unwrap();
        for filename in [
            "libexample-old.rlib",
            "test-old.exe",
            "old.o",
            "libserde-retained.rlib",
        ] {
            fs::write(deps.join(filename), b"old artifact").unwrap();
        }
        let source = root.join("crates/example/Cargo.toml");
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(UNIX_EPOCH)
            .unwrap();
        prepare(&base, &root, 1, |_| Ok(31 * GIB)).unwrap();
        assert!(!deps.join("libexample-old.rlib").exists());
        assert!(!deps.join("test-old.exe").exists());
        assert!(!deps.join("old.o").exists());
        assert!(deps.join("libserde-retained.rlib").exists());
        assert!(fs::metadata(&source).unwrap().modified().unwrap() > UNIX_EPOCH);
        assert_eq!(owner(&base.join("locks"), 1), root);
        assert!(
            prepare(&base, &root, 1, |_| Ok(29 * GIB))
                .unwrap_err()
                .contains("still below")
        );
        assert!(!base.join("target/slot-1").exists());
        let mut headroom = [29 * GIB, 31 * GIB].into_iter();
        prepare(&base, &root, 1, |_| Ok(headroom.next().unwrap())).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    use std::time::UNIX_EPOCH;

    fn age_tree(path: &Path, when: SystemTime) {
        let metadata = fs::symlink_metadata(path).unwrap();
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                age_tree(&entry.unwrap().path(), when);
            }
        }
        let mut options = File::options();
        #[cfg(unix)]
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_WRITE_ATTRIBUTES and FILE_FLAG_BACKUP_SEMANTICS permit
            // setting directory timestamps without opening directory data.
            options.access_mode(0x100).custom_flags(0x02000000);
        }
        options
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }

    fn cache_base() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Parallel tests may read the clock within its resolution; the counter
        // keeps each fixture owned by exactly one test.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "vibecut-prune-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("locks")).unwrap();
        base
    }

    #[test]
    fn cache_prune_drops_stale_sessions_and_slots_but_keeps_fresh_or_held_work() {
        let base = cache_base();
        let old = SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);

        let busy = base.join("target/slot-1/debug/incremental");
        fs::create_dir_all(busy.join("stale")).unwrap();
        fs::write(busy.join("stale/artifact"), b"old").unwrap();
        age_tree(&busy.join("stale"), old);
        let held = open(&base.join("locks/slot-1")).unwrap();
        held.lock().unwrap();

        let mixed = base.join("target/slot-2/debug/incremental");
        fs::create_dir_all(mixed.join("stale")).unwrap();
        fs::write(mixed.join("stale/artifact"), b"old").unwrap();
        fs::create_dir_all(mixed.join("fresh")).unwrap();
        fs::write(mixed.join("fresh/artifact"), b"new").unwrap();
        age_tree(&mixed.join("stale"), old);

        let stale = base.join("target/slot-3");
        fs::create_dir_all(stale.join("debug/incremental/session")).unwrap();
        fs::write(stale.join("debug/incremental/session/artifact"), b"old").unwrap();
        age_tree(&stale, old);

        prune_slots(&base, SystemTime::now(), 1 << 40, false).unwrap();
        assert!(busy.join("stale").exists(), "a held slot was pruned");
        assert!(!mixed.join("stale").exists());
        assert!(mixed.join("fresh").exists());
        assert!(!stale.exists(), "an untouched whole slot was kept");
        drop(held);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn cache_prune_drops_only_stale_dependencies_when_that_restores_the_budget() {
        let base = cache_base();
        let old = SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
        let deps = base.join("target/slot-1/debug/deps");
        fs::create_dir_all(&deps).unwrap();
        let stale = deps.join("libstale-abc.rlib");
        let fresh = deps.join("libfresh-def.rlib");
        fs::write(&stale, vec![0u8; 8192]).unwrap();
        fs::write(&fresh, vec![0u8; 4096]).unwrap();
        age_tree(&stale, old);

        // Dry-run budgeting must account for regular dependency files too.
        let used = used_bytes(&base.join("target/slot-1")).unwrap();
        let freed =
            prune_stale_deps(&base.join("target/slot-1"), 4, SystemTime::now(), true).unwrap();
        assert!(freed >= 8192, "dry run did not count the stale file");
        assert!(used.saturating_sub(freed) <= 5000);
        assert!(stale.exists(), "dry run removed the stale file");
        assert!(fresh.exists(), "dry run removed the fresh file");

        // Total exceeds the budget; the fresh artifact alone does not.
        prune_slots(&base, SystemTime::now(), 5000, false).unwrap();
        assert!(!stale.exists(), "a stale dependency was kept");
        assert!(fresh.exists(), "a fresh artifact was pruned");
        assert!(
            base.join("target/slot-1").exists(),
            "the slot was cleared even though stale entries restored the budget"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn cache_prune_clears_an_idle_slot_over_budget_and_dry_run_changes_nothing() {
        let base = cache_base();
        let slot = base.join("target/slot-1/debug");
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join("artifact"), vec![0u8; 8192]).unwrap();

        prune_slots(&base, SystemTime::now(), 0, true).unwrap();
        assert!(slot.exists(), "dry run removed the slot");

        prune_slots(&base, SystemTime::now(), 0, false).unwrap();
        assert!(!base.join("target/slot-1").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
