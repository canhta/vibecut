//! Cross-process admission, capacity, memory, load and receipt tests.
use super::admission::*;
use super::environment::*;
use super::queue::*;
use super::receipt::*;
use super::*;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

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

const TEST_CPUS: usize = 8;

fn overloaded_load() -> f64 {
    #[cfg(windows)]
    {
        MAX_PROCESSOR_LOAD_PERCENT
    }
    #[cfg(not(windows))]
    {
        MAX_LOAD_PER_CPU * TEST_CPUS as f64
    }
}

fn test_admission(max_slots: usize, memory_gated: bool) -> Admission {
    Admission {
        max_slots,
        cpus: TEST_CPUS,
        memory_gated,
        caller_jobs: false,
    }
}

/// A snapshot far above the 30% floor with no reserve, used where a test only
/// cares about slots, jobs or load rather than memory pressure.
fn ample_memory() -> Memory {
    Memory {
        available: 64 * GIB,
        total: 64 * GIB,
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

// Claims temporarily lock free slots before publishing their reservations.
// Observe under the same admission lock so probes cannot look like holders.
fn snapshot(locks: &Path, budget: usize) -> std::io::Result<(usize, usize)> {
    let _admission = Queue::admission(locks)?;
    active_slots(locks, budget)
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
                test_admission(3, false),
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
                test_admission(3, false),
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
                test_admission(3, false),
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
    let admission = if label.starts_with("budget-") {
        wait(&root.join("budget.go"));
        test_admission(8, false)
    } else {
        test_admission(3, false)
    };
    let slot = loop {
        if let Some(slot) = claim(&mut queue, &root, admission, || None, || None).unwrap() {
            break slot;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let acquired = root.join(format!("{label}.acquired"));
    let pending = acquired.with_extension("pending");
    fs::write(root.join(format!("{label}.jobs")), slot.jobs.to_string()).unwrap();
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
        // Return only once the descendant is live, so its fate is the point.
        wait(&root.join("wrapped.ready"));
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
    let expected = if std::env::var("CARGO_BUILD_JOBS").as_deref() == Ok("7") {
        2 * TEST_CPUS
    } else {
        (TEST_CPUS / 3).max(2)
    };
    assert_eq!(
        snapshot(&root.join("locks"), 2 * TEST_CPUS).unwrap(),
        (1, expected)
    );
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
    for _ in 0..3 {
        let mut queue = Queue::enter(&locks).unwrap();
        slots.push(
            claim(&mut queue, root, test_admission(3, false), || None, || None)
                .unwrap()
                .unwrap(),
        );
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
    drop(slots);

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
        let expected = jobs
            .map(str::to_owned)
            .unwrap_or_else(|| (TEST_CPUS / 3).max(2).to_string());
        assert_eq!(
            fs::read_to_string(root.join("native.jobs")).unwrap(),
            expected
        );
        assert_eq!(
            PathBuf::from(fs::read_to_string(root.join("native.target")).unwrap()),
            root.join("target/slot-1")
        );
    }
    #[cfg(unix)]
    let _held = (0..2)
        .map(|_| {
            let mut queue = Queue::enter(&locks).unwrap();
            claim(&mut queue, root, test_admission(3, false), || None, || None)
                .unwrap()
                .unwrap()
        })
        .collect::<Vec<_>>();
    #[cfg(unix)]
    for label in ["wrapped", "detached"] {
        let _ = fs::remove_file(root.join("wrapped.ready"));
        let mut wrapped = Process(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "build::tests::build_process", "--nocapture"])
                .env("VIBECUT_BUILD_TEST_ROOT", root)
                .env("VIBECUT_BUILD_TEST_LABEL", label)
                .env_remove("CARGO_BUILD_JOBS")
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
            claim(
                &mut queued,
                root,
                test_admission(3, false),
                || None,
                || None
            )
            .unwrap()
            .is_none(),
            "wrapper death made the surviving command's slot reusable"
        );
        drop(input);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if claim(
                &mut queued,
                root,
                test_admission(3, false),
                || None,
                || None,
            )
            .unwrap()
            .is_some()
            {
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
        .env_remove("CARGO_BUILD_JOBS")
        .status()
        .unwrap();
    assert!(status.success());
    let mut queued = Queue::enter(&locks).unwrap();
    assert!(
        claim(
            &mut queued,
            root,
            test_admission(3, false),
            || None,
            || None
        )
        .unwrap()
        .is_some(),
        "failed spawn retained its slot"
    );
}

#[test]
fn memory_probe_failure_blocks_only_extra_slots_and_observes_ramp() {
    let directory = Directory::new();
    assert_eq!(sysctl_memory(directory.0.join("missing-sysctl")), None);
    let old = SystemTime::now() - Duration::from_secs(30);
    // 64 GiB * 30% = 19.2 GiB floor; one running slot reserves a 3 GiB peak.
    let total = 64 * GIB;
    let plenty = Memory {
        available: 60 * GIB,
        total,
    };
    let low = Memory {
        available: 10 * GIB,
        total,
    };
    let at = |available| Memory { available, total };
    for (memory_gated, busy, start, running, memory, load, expected) in [
        // The first build is never memory-gated; unreadable facts only hold extras.
        (true, false, None, 0, None, None, true),
        (true, true, Some(old), 0, None, Some(0.0), false),
        (true, true, Some(old), 0, Some(plenty), Some(0.0), true),
        (true, true, Some(old), 0, Some(low), Some(0.0), false),
        // Must 154: free minus every running slot's reserve decides admission.
        // 22 GiB - 3 GiB = 19 GiB stays below the 19.2 GiB floor; 23 GiB - 3 GiB
        // clears it. Each extra live slot subtracts another 3 GiB peak.
        (
            true,
            true,
            Some(old),
            1,
            Some(at(22 * GIB)),
            Some(0.0),
            false,
        ),
        (
            true,
            true,
            Some(old),
            1,
            Some(at(23 * GIB)),
            Some(0.0),
            true,
        ),
        (
            true,
            true,
            Some(old),
            2,
            Some(at(25 * GIB)),
            Some(0.0),
            false,
        ),
        (
            true,
            true,
            Some(old),
            2,
            Some(at(26 * GIB)),
            Some(0.0),
            true,
        ),
        (
            true,
            true,
            Some(old),
            3,
            Some(at(28 * GIB)),
            Some(0.0),
            false,
        ),
        (true, true, Some(old), 0, Some(plenty), None, false),
        (
            true,
            true,
            Some(old),
            0,
            Some(plenty),
            Some(overloaded_load()),
            false,
        ),
        (
            true,
            true,
            Some(SystemTime::now()),
            0,
            Some(plenty),
            Some(0.0),
            false,
        ),
        (true, true, None, 0, Some(plenty), Some(0.0), false),
        (false, true, None, 0, None, None, true),
    ] {
        assert_eq!(
            admit_extra(memory_gated, busy, start, 4, running, || memory, || load),
            expected
        );
    }

    // Exact reserve boundary: the 30% floor admits, one byte below it waits,
    // each running slot moves the required free memory up by one 3 GiB peak,
    // and a reserve larger than free memory saturates to zero spare.
    let total = 32 * GIB;
    let threshold = u64::try_from((u128::from(total) * 30).div_ceil(100)).unwrap();
    let (reserve, floor) = (SLOT_PEAK_RESERVE_BYTES, threshold - 1);
    for (available, running, expected) in [
        (threshold, 0, true),
        (floor, 0, false),
        (threshold + reserve, 1, true),
        (threshold + reserve - 1, 1, false),
        (threshold + 2 * reserve, 2, true),
        (threshold + 2 * reserve - 1, 2, false),
        (threshold + 3 * reserve, 3, true),
        (threshold + 3 * reserve - 1, 3, false),
        (reserve, 1, false),
        (0, 0, false),
    ] {
        assert_eq!(
            memory_fits(Memory { available, total }, running),
            expected,
            "free={available} running={running}"
        );
    }

    let root = &directory.0;
    let locks = root.join("locks");
    let mut first = Queue::enter(&locks).unwrap();
    let held = claim(&mut first, root, test_admission(3, true), || None, || None)
        .unwrap()
        .unwrap();
    let mut second = Queue::enter(&locks).unwrap();
    let mut third = Queue::enter(&locks).unwrap();
    File::options()
        .write(true)
        .open(locks.join("last-start"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(
        claim(
            &mut second,
            root,
            test_admission(3, true),
            || None,
            || Some(0.0)
        )
        .unwrap()
        .is_none()
    );
    assert!(
        claim(
            &mut third,
            root,
            test_admission(3, true),
            || Some(ample_memory()),
            || Some(0.0)
        )
        .unwrap()
        .is_none(),
        "low memory must not let a later waiter overtake the second build"
    );
    assert!(
        claim(
            &mut second,
            root,
            test_admission(3, true),
            || Some(ample_memory()),
            || None
        )
        .unwrap()
        .is_none(),
        "an unreadable load source must hold the extra slot"
    );
    // The held lease's 3 GiB reserve is wired into `claim`, not only the pure
    // table: one byte below the reserved floor waits, exactly at it admits.
    let total = 32 * GIB;
    let threshold = u64::try_from((u128::from(total) * 30).div_ceil(100)).unwrap();
    let reserved_floor = threshold + SLOT_PEAK_RESERVE_BYTES;
    assert!(
        claim(
            &mut second,
            root,
            test_admission(3, true),
            || Some(Memory {
                available: reserved_floor - 1,
                total
            }),
            || Some(0.0)
        )
        .unwrap()
        .is_none(),
        "a running slot's peak reserve was not subtracted from free memory"
    );
    let admitted = claim(
        &mut second,
        root,
        test_admission(3, true),
        || {
            Some(Memory {
                available: reserved_floor,
                total,
            })
        },
        || Some(0.0),
    )
    .unwrap()
    .unwrap();
    assert_ne!(held.number, admitted.number);
    assert!(
        claim(
            &mut third,
            root,
            test_admission(3, true),
            || Some(ample_memory()),
            || Some(0.0)
        )
        .unwrap()
        .is_none(),
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
    assert_eq!(proc_meminfo_total(&good), Some(1000 * 1024));
    let low = directory.0.join("low-meminfo");
    fs::write(&low, "MemTotal: 1000 kB\nMemAvailable: 100 kB\n").unwrap();
    assert!(!proc_meminfo_available(&low));
    // Missing path, a directory and invalid UTF-8 all fail closed.
    assert!(!proc_meminfo_available(directory.0.join("missing-meminfo")));
    assert_eq!(
        proc_meminfo_total(directory.0.join("missing-meminfo")),
        None
    );
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
    assert!(!admit_extra(
        true,
        true,
        Some(old),
        4,
        0,
        || meminfo_memory(bad),
        || Some(0.0)
    ));
    assert!(admit_extra(
        true,
        true,
        Some(old),
        4,
        0,
        || meminfo_memory(good),
        || Some(0.0)
    ));
    assert!(
        !admit_extra(
            true,
            true,
            Some(SystemTime::now()),
            4,
            0,
            || meminfo_memory(good),
            || Some(0.0)
        ),
        "the ramp still gates extra admission on Linux"
    );
    // The first build is never gated; only extra slots consult the probe.
    assert!(admit_extra(
        true,
        false,
        None,
        4,
        0,
        || meminfo_memory(bad),
        || None
    ));
}

// Must 1: capacity is the smaller of the CPU bound and the memory budget,
// with a hard ceiling. The budget divides by the 3 GiB peak every extra slot
// reserves, plus the first slot that is never memory-gated. Unreadable total
// memory yields no derived capacity, which holds extra admission (the first
// build still runs).
#[test]
fn slot_ceiling_follows_cpu_memory_and_the_hard_bound() {
    // 64 GiB host: (64*0.70)/3 = 14 reserves plus the first slot = 15, below 16 CPUs.
    assert_eq!(derived_slots(16, Some(64 * GIB)), Some(15));
    // 32 GiB host: (32*0.70)/3 = 7 reserves plus the first slot = 8, below 20 CPUs.
    assert_eq!(derived_slots(20, Some(32 * GIB)), Some(8));
    // 4 GiB host: the 30% reservation leaves less than one 3 GiB peak reserve.
    assert_eq!(derived_slots(64, Some(4 * GIB)), Some(1));
    assert_eq!(derived_slots(200, Some(1024 * GIB)), Some(MAX_SLOTS));
    assert_eq!(derived_slots(8, None), None);
    assert_eq!(derived_slots(8, Some(SLOT_PEAK_RESERVE_BYTES)), Some(1));
    assert_eq!(derived_slots(8, Some(0)), Some(1));
    assert_eq!(derived_slots(0, Some(64 * GIB)), Some(1));
    let directory = Directory::new();
    assert_eq!(
        sysctl_total_memory(directory.0.join("missing-sysctl")),
        None
    );
}

// Must 1/3: a supported override wins; zero, out-of-range or non-numeric
// values are typed errors before any queue work; non-gated Unix keeps its
// CPU-derived capacity and admits by available slots alone.
#[test]
fn build_slot_override_is_validated_with_a_typed_error() {
    let total = Some(64 * GIB);
    assert_eq!(configured_slots(true, None, 16, total), Ok(15));
    assert_eq!(
        configured_slots(true, Some(OsStr::new("3")), 16, total),
        Ok(3)
    );
    assert_eq!(
        configured_slots(true, Some(OsStr::new("16")), 16, total),
        Ok(16)
    );
    assert_eq!(
        configured_slots(true, Some(OsStr::new("64")), 64, total),
        Ok(MAX_SLOTS)
    );
    assert_eq!(
        configured_slots(true, Some(OsStr::new("0")), 16, total),
        Err(SlotsError::OutOfRange {
            value: "0".into(),
            max: 16,
        })
    );
    assert_eq!(
        configured_slots(true, Some(OsStr::new("17")), 16, total),
        Err(SlotsError::OutOfRange {
            value: "17".into(),
            max: 16,
        })
    );
    assert_eq!(
        configured_slots(true, Some(OsStr::new("65")), 64, total),
        Err(SlotsError::OutOfRange {
            value: "65".into(),
            max: MAX_SLOTS,
        })
    );
    for value in ["-1", "not-a-number", "", " 3", "999999999999999999999"] {
        assert_eq!(
            configured_slots(true, Some(OsStr::new(value)), 16, total),
            Err(SlotsError::NotAnInteger(value.into())),
            "{value:?}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let invalid = OsStr::from_bytes(&[0xff, 0xfe]);
        assert_eq!(
            configured_slots(true, Some(invalid), 16, total),
            Err(SlotsError::NotAnInteger("\u{fffd}\u{fffd}".into()))
        );
    }
    assert_eq!(configured_slots(true, None, 8, None), Ok(1));
    assert_eq!(configured_slots(false, None, 8, None), Ok(8));
    assert_eq!(
        configured_slots(false, Some(OsStr::new("2")), 8, None),
        Ok(2)
    );
}

// Must 1/3: Linux loadavg and macOS vm.loadavg parse only finite,
// non-negative, complete samples; every malformed or unreadable source
// returns `None` so admission fails closed.
#[test]
fn load_sources_parse_complete_samples_or_fail_closed() {
    for (text, expected) in [
        ("0.42 0.57 1.86 1/563 123", Some(0.42)),
        ("0 0 0", Some(0.0)),
        ("2.50 2.00 1.50 4/100 9", Some(2.5)),
        ("", None),
        ("0.5", None),
        ("0.5 0.4", None),
        ("1.0 nope 2.0", None),
        ("-1 0 0", None),
        ("NaN 0 0", None),
        ("inf 0 0", None),
    ] {
        assert_eq!(parse_loadavg(text), expected, "{text:?}");
    }
    for (text, expected) in [
        ("{ 1.50 2.00 3.00 }", Some(1.5)),
        ("1.50 2.00 3.00", Some(1.5)),
        ("{ 1.5 2.0 }", None),
        ("1.5", None),
        ("1.5 2.0", None),
        ("{ 1.5", None),
        ("", None),
        ("{ nope 2.0 3.0 }", None),
        ("{ -1 2.0 3.0 }", None),
    ] {
        assert_eq!(parse_sysctl_load(text), expected, "{text:?}");
    }
    let directory = Directory::new();
    assert_eq!(sysctl_load(directory.0.join("missing-sysctl")), None);
    assert_eq!(proc_loadavg(directory.0.join("missing-loadavg")), None);
    let good = directory.0.join("loadavg");
    fs::write(&good, "0.25 0.50 0.75 1/100 7\n").unwrap();
    assert_eq!(proc_loadavg(&good), Some(0.25));
    let binary = directory.0.join("binary-loadavg");
    fs::write(&binary, [0xff, 0xfe]).unwrap();
    assert_eq!(proc_loadavg(&binary), None);
}

// Must 1: Windows busy fraction is `1 - idle/(kernel+user)`; backwards,
// zero-delta and idle-exceeding-total counters fail closed, and the
// documented 90% bound is strict.
#[test]
fn windows_processor_load_handles_samples_and_hostile_counters() {
    assert_eq!(
        processor_load((100, 200), (300, 450), (100, 150)),
        Some(0.5)
    );
    assert_eq!(processor_load((10, 10), (20, 20), (5, 5)), None);
    assert_eq!(processor_load((50, 10), (20, 20), (5, 10)), None);
    assert_eq!(processor_load((0, 500), (0, 100), (0, 100)), None);
    assert_eq!(
        processor_load((0, 0), (0, u64::MAX), (0, u64::MAX)),
        Some(1.0)
    );
    assert!(processor_load_within_bound(Some(89.9)));
    assert!(!processor_load_within_bound(Some(90.0)));
    assert!(!processor_load_within_bound(Some(100.0)));
    assert!(!processor_load_within_bound(Some(-1.0)));
    assert!(!processor_load_within_bound(Some(f64::NAN)));
    assert!(!processor_load_within_bound(None));
}

#[test]
fn mixed_capacities_reserve_only_remaining_host_jobs() {
    let directory = Directory::new();
    let root = &directory.0;
    let locks = root.join("locks");
    let mut held = Vec::new();
    for (ceiling, expected) in [(1, 20), (2, 10), (3, 6), (4, 4)] {
        let mut queue = Queue::enter(&locks).unwrap();
        let slot = claim(
            &mut queue,
            root,
            Admission {
                cpus: 20,
                ..test_admission(ceiling, false)
            },
            || None,
            || None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(slot.jobs, expected);
        held.push(slot);
        assert!(snapshot(&locks, 40).unwrap().1 <= 40);
    }
    let mut next = Queue::enter(&locks).unwrap();
    let admission = Admission {
        cpus: 20,
        ..test_admission(20, false)
    };
    assert!(
        claim(&mut next, root, admission, || None, || None)
            .unwrap()
            .is_none()
    );
    // A released lease frees its entire reservation; stale metadata does
    // not consume jobs, even when another slot number is selected.
    drop(held.remove(1));
    let replacement = claim(&mut next, root, admission, || None, || None)
        .unwrap()
        .unwrap();
    assert_eq!(replacement.jobs, 2);
    assert_eq!(snapshot(&locks, 40).unwrap().1, 32);

    let directory = Directory::new();
    let root = &directory.0;
    let locks = root.join("locks");
    let mut held = Vec::new();
    for ceiling in [1, 2] {
        let mut queue = Queue::enter(&locks).unwrap();
        held.push(
            claim(
                &mut queue,
                root,
                Admission {
                    cpus: 3,
                    ..test_admission(ceiling, false)
                },
                || None,
                || None,
            )
            .unwrap()
            .unwrap(),
        );
    }
    assert_eq!(snapshot(&locks, 6).unwrap().1, 5);
    let mut next = Queue::enter(&locks).unwrap();
    assert!(
        claim(
            &mut next,
            root,
            Admission {
                cpus: 3,
                ..test_admission(3, false)
            },
            || None,
            || None
        )
        .unwrap()
        .is_none(),
        "one remaining job cannot admit a two-job build"
    );

    // Must 2: a build admitted while free memory is under the floor caps its
    // own link/test fan-out; an above-floor snapshot keeps the normal formula,
    // and pressure never drops below one job.
    let directory = Directory::new();
    let root = &directory.0;
    let locks = root.join("locks");
    let mut queue = Queue::enter(&locks).unwrap();
    let compressed = claim(
        &mut queue,
        root,
        test_admission(1, true),
        || {
            Some(Memory {
                available: 8 * GIB,
                total: 32 * GIB,
            })
        },
        || None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(compressed.jobs, TEST_CPUS / 2);
    let directory = Directory::new();
    let root = &directory.0;
    let mut queue = Queue::enter(&root.join("locks")).unwrap();
    let normal = claim(
        &mut queue,
        root,
        test_admission(1, true),
        || Some(ample_memory()),
        || None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(normal.jobs, TEST_CPUS);
    let directory = Directory::new();
    let root = &directory.0;
    let mut queue = Queue::enter(&root.join("locks")).unwrap();
    let single = claim(
        &mut queue,
        root,
        Admission {
            cpus: 1,
            ..test_admission(1, true)
        },
        || {
            Some(Memory {
                available: 8 * GIB,
                total: 32 * GIB,
            })
        },
        || None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(single.jobs, 1);
}

#[test]
fn caller_job_settings_select_exclusive_admission() {
    for (args, override_value, expected) in [
        (vec!["cargo", "check"], None, false),
        (vec!["cargo", "check"], Some("999"), true),
        (vec!["cargo", "check"], Some("-1"), true),
        (vec!["cargo", "check", "--jobs", "999"], None, true),
        (vec!["cargo", "check", "--jobs=999"], None, true),
        (vec!["cargo", "check", "-j", "999"], None, true),
        (
            vec!["cargo", "xtask", "build", "--", "cargo", "check", "-j999"],
            None,
            true,
        ),
    ] {
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        assert_eq!(caller_jobs(&args, override_value.map(OsStr::new)), expected);
    }
}

#[test]
fn concurrent_admission_and_release_preserve_fifo_and_job_budget() {
    for paused_scan in [false, true] {
        let directory = Directory::new();
        let root = &directory.0;
        let locks = root.join("locks");
        let mut held = Vec::new();
        for ceiling in [1, 2, 3] {
            let mut queue = Queue::enter(&locks).unwrap();
            held.push(
                claim(
                    &mut queue,
                    root,
                    test_admission(ceiling, false),
                    || None,
                    || None,
                )
                .unwrap()
                .unwrap(),
            );
        }
        assert_eq!(snapshot(&locks, 16).unwrap().1, 14);
        if paused_scan {
            // Retained metadata from earlier builds is valid for free slots. A
            // claimant probes both free numbers before choosing one; pause in its
            // load probe, while admission and both free-slot locks are held.
            fs::write(locks.join("slot-4.jobs"), "2").unwrap();
            fs::write(locks.join("slot-5.jobs"), "16").unwrap();
            open(&locks.join("last-start"))
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(30))
                .unwrap();
            let mut queue = Queue::enter(&locks).unwrap();
            std::thread::scope(|scope| {
                let (scanned, observing) = std::sync::mpsc::sync_channel(0);
                let (resume, paused) = std::sync::mpsc::sync_channel(0);
                let locks = &locks;
                let claimant = scope.spawn(move || {
                    claim(
                        &mut queue,
                        root,
                        test_admission(5, true),
                        || Some(ample_memory()),
                        || {
                            scanned.send(()).unwrap();
                            paused.recv().unwrap();
                            Some(0.0)
                        },
                    )
                    .unwrap()
                    .unwrap()
                });
                observing.recv().unwrap();
                // This is the original unsynchronised observer's exact false
                // overcount: 14 real jobs plus two free-slot probes (2 + 16).
                assert_eq!(active_slots(locks, 16).unwrap().1, 32);
                assert!(
                    try_lock(open(&locks.join("admission")).unwrap())
                        .unwrap()
                        .is_none()
                );
                resume.send(()).unwrap();
                let extra = claimant.join().unwrap();
                assert_eq!(extra.jobs, 2);
                assert_eq!(snapshot(locks, 16).unwrap().1, 16);
                drop(extra);
            });
            assert_eq!(snapshot(&locks, 16).unwrap().1, 14);
        }
        // Independent processes contend for the final two jobs, with ample
        // free slot numbers. Their enqueue order is controlled, claims race.
        let mut first = worker(root, "budget-first");
        let mut second = worker(root, "budget-second");
        let mut third = worker(root, "budget-third");
        fs::write(root.join("budget.go"), b"").unwrap();
        wait(&root.join("budget-first.acquired"));
        assert!(!root.join("budget-second.acquired").exists());
        assert!(!root.join("budget-third.acquired").exists());
        assert_eq!(snapshot(&locks, 16).unwrap().1, 16);
        // Release races with both pending claimers. Only the FIFO head may
        // reuse the reservation; killing it must then free jobs for the next.
        fs::write(root.join("budget-first.release"), b"").unwrap();
        assert!(first.0.wait().unwrap().success());
        wait(&root.join("budget-second.acquired"));
        assert!(!root.join("budget-third.acquired").exists());
        assert_eq!(snapshot(&locks, 16).unwrap().1, 16);
        second.0.kill().unwrap();
        second.0.wait().unwrap();
        wait(&root.join("budget-third.acquired"));
        assert_eq!(snapshot(&locks, 16).unwrap().1, 16);
        for label in ["budget-first", "budget-second", "budget-third"] {
            assert_eq!(
                fs::read_to_string(root.join(format!("{label}.jobs"))).unwrap(),
                "2"
            );
        }
        fs::write(root.join("budget-third.release"), b"").unwrap();
        assert!(third.0.wait().unwrap().success());
        drop(held);
        assert_eq!(snapshot(&locks, 16).unwrap(), (0, 0));
    }
}

#[test]
fn caller_jobs_and_unreadable_reservations_hold_default_admission() {
    let directory = Directory::new();
    let root = &directory.0;
    let locks = root.join("locks");
    let mut queue = Queue::enter(&locks).unwrap();
    let explicit = Admission {
        caller_jobs: true,
        ..test_admission(8, false)
    };
    let held = claim(&mut queue, root, explicit, || None, || None)
        .unwrap()
        .unwrap();
    assert_eq!(held.jobs, 16);
    let mut next = Queue::enter(&locks).unwrap();
    let default = test_admission(8, false);
    for cpus in [8, 16, 4] {
        assert!(
            claim(
                &mut next,
                root,
                Admission {
                    cpus,
                    max_slots: cpus.min(default.max_slots),
                    ..default
                },
                || None,
                || None
            )
            .unwrap()
            .is_none(),
            "an explicit job override must stay exclusive when available CPUs change"
        );
    }
    drop(held);
    let held = claim(&mut next, root, default, || None, || None)
        .unwrap()
        .unwrap();
    let mut next = Queue::enter(&locks).unwrap();
    assert!(
        claim(&mut next, root, explicit, || None, || None)
            .unwrap()
            .is_none()
    );
    let metadata = locks.join(format!("slot-{}.jobs", held.number));
    for invalid in [None, Some(""), Some("0"), Some("broken")] {
        if let Some(text) = invalid {
            fs::write(&metadata, text).unwrap();
        } else {
            fs::remove_file(&metadata).unwrap();
        }
        assert!(
            claim(&mut next, root, default, || None, || None)
                .unwrap()
                .is_none()
        );
    }
    drop(held);
    assert!(
        claim(&mut next, root, explicit, || None, || None)
            .unwrap()
            .is_some()
    );
}

// Must 3: shrinking capacity stops new admissions but never releases a
// running build's lease, and the freed slot is usable when capacity grows.
#[test]
fn shrinking_capacity_never_kills_running_builds() {
    let directory = Directory::new();
    let root = &directory.0;
    let locks = root.join("locks");
    let mut first = Queue::enter(&locks).unwrap();
    let mut second = Queue::enter(&locks).unwrap();
    let held_a = claim(&mut first, root, test_admission(4, false), || None, || None)
        .unwrap()
        .unwrap();
    let held_b = claim(
        &mut second,
        root,
        test_admission(4, false),
        || None,
        || None,
    )
    .unwrap()
    .unwrap();
    assert_ne!(held_a.number, held_b.number);
    // A stray name is not a slot lease.
    fs::write(locks.join("slot-x"), b"").unwrap();
    assert_eq!(snapshot(&locks, 16).unwrap().0, 2);

    let mut shrunk = Queue::enter(&locks).unwrap();
    assert!(
        claim(
            &mut shrunk,
            root,
            test_admission(1, false),
            || None,
            || None
        )
        .unwrap()
        .is_none(),
        "shrunk capacity admitted another build"
    );
    assert!(
        try_lock(open(&locks.join(format!("slot-{}", held_a.number))).unwrap())
            .unwrap()
            .is_none(),
        "shrinking released a running build"
    );
    assert!(
        try_lock(open(&locks.join(format!("slot-{}", held_b.number))).unwrap())
            .unwrap()
            .is_none(),
        "shrinking released a running build"
    );

    let freed_number = held_b.number;
    drop(held_b);
    assert_eq!(snapshot(&locks, 16).unwrap().0, 1);
    let regrown = claim(
        &mut shrunk,
        root,
        test_admission(2, false),
        || None,
        || None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(regrown.number, freed_number);
}

// Must 2: a slot number beyond an earlier, smaller capacity is reusable
// when capacity grows; admission does not delete its warm directory.
#[test]
fn slots_beyond_a_shrunk_count_are_reused_not_deleted() {
    let directory = Directory::new();
    let root = &directory.0;
    let locks = root.join("locks");
    fs::create_dir_all(&locks).unwrap();
    fs::create_dir_all(root.join("target/slot-9/debug")).unwrap();
    fs::write(root.join("target/slot-9/debug/artifact"), b"warm").unwrap();
    let mut _held = Vec::new();
    for number in 1..=8 {
        let file = open(&locks.join(format!("slot-{number}"))).unwrap();
        file.lock().unwrap();
        fs::write(locks.join(format!("slot-{number}.jobs")), "2").unwrap();
        _held.push(file);
    }
    let mut queue = Queue::enter(&locks).unwrap();
    let slot = claim(
        &mut queue,
        root,
        Admission {
            cpus: 16,
            ..test_admission(9, false)
        },
        || None,
        || None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(slot.number, 9);
    assert!(root.join("target/slot-9/debug/artifact").is_file());
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

// Windows has no inherited lease: the launcher sits in a kill-on-close job, so
// launcher death (`taskkill /F` without /T) and a command that exits leaving a
// detached descendant both end the whole tree, and only then free the slot.
#[cfg(windows)]
#[test]
fn launcher_death_and_detached_descendants_do_not_outlive_the_slot() {
    for label in ["wrapped", "detached"] {
        let directory = Directory::new();
        let root = &directory.0;
        crate::checked(root, "git", &["init", "-q"]).unwrap();
        let mut wrapper = Process(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "build::tests::build_process", "--nocapture"])
                .env("VIBECUT_BUILD_TEST_ROOT", root)
                .env("VIBECUT_BUILD_TEST_LABEL", label)
                .env_remove("CARGO_BUILD_JOBS")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        // The test owns the pipe write end, so the command or descendant can
        // only end by being killed, never by reading EOF.
        let _input = wrapper.0.stdin.take().unwrap();
        wait(&root.join("wrapped.ready"));
        if label == "wrapped" {
            let launcher = wrapper.0.id().to_string();
            let status = Command::new("taskkill")
                .args(["/F", "/PID", &launcher])
                .status()
                .unwrap();
            assert!(status.success());
        }
        wrapper.0.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        for lock in ["command-live", "locks/slot-1"] {
            loop {
                let free = try_lock(open(&root.join(lock)).unwrap()).unwrap().is_some();
                if free {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{label}: {lock} still held after the launcher ended"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
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
