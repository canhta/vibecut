//! Cross-process FIFO build admission, warm slots and clean-tree CI receipts.
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime};

const SLOTS: usize = 3;
const GIB: u64 = 1 << 30;

// Explicit unlock also covers retained/duplicated descriptors at a process boundary.
struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
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
        Ok(()) => Ok(Some(Lock(file))),
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
            held: Some(Lock(file)),
        })
    }

    fn admission(locks: &Path) -> io::Result<Lock> {
        let file = open(&locks.join("admission"))?;
        file.lock()?;
        Ok(Lock(file))
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
            // A held lock is the liveness signal on all platforms. PIDs are
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
        // Close before unlink, including on Windows. Caller holds admission.
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
    _lock: Lock,
    number: usize,
}

fn owner(locks: &Path, number: usize) -> PathBuf {
    fs::read_to_string(locks.join(format!("slot-{number}.owner")))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn memory_available(program: impl AsRef<OsStr>) -> bool {
    Command::new(program)
        .args(["-n", "kern.memorystatus_level"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u32>().ok())
        .is_some_and(|level| level >= 30)
}

fn admit_extra(
    macos: bool,
    busy: bool,
    last_start: Option<SystemTime>,
    memory: impl FnOnce() -> bool,
) -> bool {
    if !macos || !busy {
        return true;
    }
    last_start
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age >= Duration::from_secs(20))
        && memory()
}

fn claim(
    queue: &mut Queue,
    root: &Path,
    macos: bool,
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
    if free.is_empty() || !admit_extra(macos, busy, last, memory) {
        return Ok(None);
    }
    free.sort_by_key(|(number, _)| (owner(&queue.locks, *number) != root, *number));
    let (number, mut lock) = free
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("no build slot"))?;
    fs::write(queue.locks.join("last-start"), b"")?;
    lock.0.set_len(0)?;
    writeln!(lock.0, "{}", root.display())?;
    queue.leave()?;
    Ok(Some(Slot {
        _lock: lock,
        number,
    }))
}

fn cache() -> Result<PathBuf, String> {
    // Same per-user cache owner as native provisioning (including native
    // USERPROFILE on Windows, independently of MSYS HOME).
    crate::ffmpeg::cache()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cache has no parent".into())
}

fn used_bytes(path: &Path) -> io::Result<u64> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut total = 0u64;
    for entry in entries {
        let path = entry?.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let size = if metadata.is_dir() {
            used_bytes(&path)?
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                metadata.blocks().saturating_mul(512)
            }
            #[cfg(not(unix))]
            {
                metadata.len()
            }
        };
        total = total.saturating_add(size);
    }
    Ok(total)
}

fn disk_free(path: &Path) -> Result<u64, String> {
    #[cfg(windows)]
    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Item -LiteralPath $env:VIBECUT_BUILD_PATH).PSDrive.Free",
        ])
        .env("VIBECUT_BUILD_PATH", path)
        .output();
    #[cfg(not(windows))]
    let result = Command::new("df").args(["-Pk"]).arg(path).output();
    let result = result.map_err(|e| format!("disk free-space query: {e}"))?;
    if !result.status.success() {
        return Err("disk free-space query failed".into());
    }
    let text = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let value = text.trim().parse::<u64>().map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
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
            // Touching it would make a nested `cargo xtask ci` rebuild the
            // executing launcher, which Windows refuses to replace.
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

fn paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|bytes| {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
            }
            #[cfg(not(unix))]
            {
                String::from_utf8(bytes.to_vec())
                    .map(PathBuf::from)
                    .map_err(|e| e.to_string())
            }
        })
        .collect()
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
    let base = cache()?;
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
    let slot = loop {
        if let Some(slot) = claim(&mut queue, root, cfg!(target_os = "macos"), || {
            memory_available("sysctl")
        })
        .map_err(|e| e.to_string())?
        {
            break slot;
        }
        if !said {
            eprintln!(
                "[build] waiting (FIFO; extra macOS builds need 30% free memory and 20s ramp)"
            );
            said = true;
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let target = prepare(&base, root, slot.number, disk_free)?;
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
        command.env("CARGO_BUILD_JOBS", "4");
    }
    let status = command
        .status()
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
        let mut queue = Queue::enter(&root.join("locks")).unwrap();
        fs::write(root.join(format!("{label}.ready")), b"").unwrap();
        let slot = loop {
            if let Some(slot) = claim(&mut queue, &root, false, || false).unwrap() {
                break slot;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        fs::write(
            root.join(format!("{label}.acquired")),
            slot.number.to_string(),
        )
        .unwrap();
        wait(&root.join(format!("{label}.release")));
        drop(slot);
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
    }

    #[test]
    fn memory_probe_failure_blocks_only_extra_macos_slots_and_observes_ramp() {
        let directory = Directory::new();
        assert!(!memory_available(directory.0.join("missing-sysctl")));
        let old = SystemTime::now() - Duration::from_secs(30);
        for (macos, busy, start, memory, expected) in [
            (true, false, None, false, true),
            (true, true, Some(old), false, false),
            (true, true, Some(old), true, true),
            (true, true, Some(SystemTime::now()), true, false),
            (true, true, None, true, false),
            (false, true, None, false, true),
        ] {
            assert_eq!(admit_extra(macos, busy, start, || memory), expected);
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
}
