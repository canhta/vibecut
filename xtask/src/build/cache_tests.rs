//! Build-cache pruning and worktree-switch preparation tests.
use super::cache::*;
use super::environment::*;
use super::queue::*;
use super::slot::*;
use super::*;
use std::fs::{self, File};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn worktree_switch_prunes_old_binaries_and_disk_guard_refuses_unrecovered_headroom() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("crates/example")).unwrap();
    crate::process::checked(root, "git", &["init", "-q"]).unwrap();
    fs::write(
        root.join("crates/example/Cargo.toml"),
        "[package]\nname = 'example'\n",
    )
    .unwrap();
    crate::process::checked(root, "git", &["add", "."]).unwrap();
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
    prepare(&base, root, 1, |_| Ok(31 * GIB)).unwrap();
    assert!(!deps.join("libexample-old.rlib").exists());
    assert!(!deps.join("test-old.exe").exists());
    assert!(!deps.join("old.o").exists());
    assert!(deps.join("libserde-retained.rlib").exists());
    assert!(fs::metadata(&source).unwrap().modified().unwrap() > UNIX_EPOCH);
    assert_eq!(owner(&base.join("locks"), 1), root);
    assert!(
        prepare(&base, root, 1, |_| Ok(29 * GIB))
            .unwrap_err()
            .contains("still below")
    );
    assert!(!base.join("target/slot-1").exists());
    let mut headroom = [29 * GIB, 31 * GIB].into_iter();
    prepare(&base, root, 1, |_| Ok(headroom.next().unwrap())).unwrap();
}

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

fn cache_base() -> tempfile::TempDir {
    let base = tempfile::tempdir().unwrap();
    fs::create_dir_all(base.path().join("locks")).unwrap();
    base
}

#[test]
fn cache_prune_drops_stale_sessions_and_slots_but_keeps_fresh_or_held_work() {
    let directory = cache_base();
    let base = directory.path();
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

    prune_slots(base, SystemTime::now(), 1 << 40, false).unwrap();
    assert!(busy.join("stale").exists(), "a held slot was pruned");
    assert!(!mixed.join("stale").exists());
    assert!(mixed.join("fresh").exists());
    assert!(!stale.exists(), "an untouched whole slot was kept");
    drop(held);
}

#[test]
fn cache_prune_drops_only_stale_dependencies_when_that_restores_the_budget() {
    let directory = cache_base();
    let base = directory.path();
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
    let slot = base.join("target/slot-1");
    let freed = prune_deps(&slot, SystemTime::now(), used, 5000, true).unwrap();
    assert!(freed >= 8192, "dry run did not count the stale file");
    assert!(used.saturating_sub(freed) <= 5000);
    assert!(stale.exists(), "dry run removed the stale file");
    assert!(fresh.exists(), "dry run removed the fresh file");

    // Total exceeds the budget; the fresh artifact alone does not.
    prune_slots(base, SystemTime::now(), 5000, false).unwrap();
    assert!(!stale.exists(), "a stale dependency was kept");
    assert!(fresh.exists(), "a fresh artifact was pruned");
    assert!(
        base.join("target/slot-1").exists(),
        "the slot was cleared even though stale entries restored the budget"
    );

    // Still over the budget with only fresh dependencies: the oldest go first
    // and the slot stays warm; with only non-dependency bytes over the budget it is cleared.
    let older = deps.join("libolder-123.rlib");
    fs::write(&older, vec![0u8; 8192]).unwrap();
    age_tree(&older, SystemTime::now() - Duration::from_secs(60));
    prune_slots(base, SystemTime::now(), 5000, false).unwrap();
    assert!(!older.exists(), "the oldest fresh dependency was kept");
    assert!(
        fresh.exists(),
        "a newer dependency was pruned before an older one"
    );
    fs::write(slot.join("debug/artifact"), vec![0u8; 8192]).unwrap();
    prune_slots(base, SystemTime::now(), 4096, false).unwrap();
    assert!(!slot.exists(), "an unprunable slot over budget was kept");
}

#[test]
fn cache_prune_ages_out_slots_beyond_the_current_count() {
    let directory = cache_base();
    let base = directory.path();
    let old = SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
    let beyond = base.join("target/slot-9/debug");
    fs::create_dir_all(&beyond).unwrap();
    fs::write(beyond.join("artifact"), b"old").unwrap();
    age_tree(&base.join("target/slot-9"), old);
    let fresh = base.join("target/slot-12/debug");
    fs::create_dir_all(&fresh).unwrap();
    fs::write(fresh.join("artifact"), b"new").unwrap();

    prune_slots(base, SystemTime::now(), 1 << 40, false).unwrap();
    assert!(
        !base.join("target/slot-9").exists(),
        "a stale slot beyond the current capacity was kept"
    );
    assert!(
        base.join("target/slot-12").exists(),
        "a fresh slot beyond the current capacity was pruned"
    );
}

#[test]
fn cache_prune_clears_an_idle_slot_over_budget_and_dry_run_changes_nothing() {
    let directory = cache_base();
    let base = directory.path();
    let slot = base.join("target/slot-1/debug");
    fs::create_dir_all(&slot).unwrap();
    fs::write(slot.join("artifact"), vec![0u8; 8192]).unwrap();

    prune_slots(base, SystemTime::now(), 0, true).unwrap();
    assert!(slot.exists(), "dry run removed the slot");

    prune_slots(base, SystemTime::now(), 0, false).unwrap();
    assert!(!base.join("target/slot-1").exists());
}

// Must 4: a worktree switch keeps a warm slot unless that slot alone is over
// the measured per-slot budget or free disk cannot fund a build. The total
// size of every warm slot must never turn a switch into a cold rebuild.
#[test]
fn worktree_switch_clears_only_a_slot_over_its_own_budget_or_low_free_space() {
    for (used, free, expected) in [
        (0, 31 * GIB, false),
        (CACHE_SLOT_LIMIT, 31 * GIB, false),
        (CACHE_SLOT_LIMIT + 1, 31 * GIB, true),
        (CACHE_SLOT_LIMIT + 1, 100 * GIB, true),
        (0, 30 * GIB - 1, true),
        (0, 30 * GIB, false),
    ] {
        assert_eq!(
            switch_overflow(used, free),
            expected,
            "used={used} free={free}"
        );
    }
}
