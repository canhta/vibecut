//! Build-cache pruning and worktree-switch preparation tests.
use super::cache::*;
use super::environment::*;
use super::queue::*;
use super::*;
use std::fs::{self, File};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn checkout_targets_survive_other_checkouts_without_touching_sources() {
    let directory = cache_base();
    let base = directory.path();
    let first = base.join("first");
    let second = base.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let source = first.join("source.rs");
    fs::write(&source, b"source").unwrap();
    open(&source).unwrap().set_modified(UNIX_EPOCH).unwrap();
    let target = prepare(base, &first, &[], |_| Ok(200 * GIB)).unwrap();
    fs::write(target.join("workspace.rlib"), b"compiled").unwrap();
    let other = prepare(base, &second, &[], |_| Ok(200 * GIB)).unwrap();
    assert_ne!(target, other);
    assert_eq!(
        prepare(base, &first, &[], |_| Ok(200 * GIB)).unwrap(),
        target
    );
    assert_eq!(
        fs::read(target.join("workspace.rlib")).unwrap(),
        b"compiled"
    );
    assert_eq!(
        fs::metadata(source).unwrap().modified().unwrap(),
        UNIX_EPOCH
    );
    assert_eq!(
        target_name(&first).unwrap(),
        target_name(&first.join(".")).unwrap()
    );
    for free in [0, 30 * GIB - 1] {
        assert!(
            prepare(base, &first, &[], |_| Ok(free))
                .unwrap_err()
                .contains("30 GiB")
        );
        assert!(
            target.join("workspace.rlib").exists(),
            "refusal destroyed warm output"
        );
    }
    assert!(prepare(base, &first, &[], |_| Ok(30 * GIB)).is_ok());
    let owner = base
        .join("locks")
        .join(format!("{}.owner", target_name(&first).unwrap()));
    fs::write(&owner, second.to_str().unwrap()).unwrap();
    assert!(
        prepare(base, &first, &[], |_| Ok(200 * GIB))
            .unwrap_err()
            .contains("collision")
    );
    assert!(target.join("workspace.rlib").exists());
    fs::remove_file(owner).unwrap();
    assert!(
        prepare(base, &first, &[], |_| Ok(200 * GIB))
            .unwrap_err()
            .contains("owner")
    );
}

#[test]
fn directory_capacity_refuses_new_targets_and_reports_prune_candidates() {
    for (free, used, expected) in [
        (0, 0, 0),
        (30 * GIB - 1, 0, 0),
        (30 * GIB, 0, 0),
        (30 * GIB + CACHE_SLOT_LIMIT - 1, 0, 0),
        (30 * GIB + CACHE_SLOT_LIMIT, 0, 1),
        (30 * GIB, CACHE_SLOT_LIMIT, 1),
        (u64::MAX, u64::MAX, (u64::MAX - 30 * GIB) / CACHE_SLOT_LIMIT),
    ] {
        assert_eq!(directory_cap(free, used, CACHE_SLOT_LIMIT), expected);
    }
    let directory = cache_base();
    let base = directory.path();
    let first = base.join("first");
    let second = base.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    prepare(base, &first, &[], |_| Ok(30 * GIB + CACHE_SLOT_LIMIT)).unwrap();
    let error = prepare(base, &second, &[], |_| Ok(30 * GIB + CACHE_SLOT_LIMIT)).unwrap_err();
    assert!(error.contains(&target_name(&first).unwrap()));
    assert!(error.contains("capacity 1"));
    assert!(error.contains("prune --make-room"));
    assert!(
        !base
            .join("target")
            .join(target_name(&second).unwrap())
            .exists()
    );
}

#[test]
fn checkout_prune_removes_gone_then_oldest_without_touching_held_or_dry_run_targets() {
    let directory = cache_base();
    let base = directory.path();
    let old = SystemTime::now() - Duration::from_secs(60);
    let mut paths = Vec::new();
    for name in ["gone", "oldest", "held", "newest"] {
        let root = base.join(name);
        fs::create_dir_all(&root).unwrap();
        let target = prepare(base, &root, &[], |_| Ok(200 * GIB)).unwrap();
        fs::write(target.join("artifact"), b"warm").unwrap();
        let key = target_name(&root).unwrap();
        if name == "oldest" {
            open(&base.join("locks").join(format!("{key}.used")))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        paths.push((root, target, key));
    }
    fs::remove_dir(&paths[0].0).unwrap();
    let _held = try_lock(open(&base.join("locks").join(&paths[2].2)).unwrap())
        .unwrap()
        .unwrap();
    let free = 30 * GIB + 2 * CACHE_SLOT_LIMIT;
    prune_targets(base, SystemTime::now(), free, true, false, "fixture").unwrap();
    assert!(paths.iter().all(|(_, target, _)| target.exists()));
    prune_targets(base, SystemTime::now(), free, false, false, "fixture").unwrap();
    assert!(!paths[0].1.exists());
    assert!(!paths[1].1.exists());
    assert!(paths[2].1.join("artifact").exists());
    assert!(paths[3].1.join("artifact").exists());
    fs::write(
        base.join("locks").join(format!("{}.owner", paths[3].2)),
        "damaged owner",
    )
    .unwrap();
    prune_targets(base, SystemTime::now(), free, false, false, "fixture").unwrap();
    assert!(paths[3].1.exists(), "unverified owner was pruned");
    assert!(paths[2].1.join("artifact").exists());
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

#[test]
fn warm_reserve_bootstraps_qualifies_matching_full_runs_and_tracks_failed_growth() {
    let directory = cache_base();
    let base = directory.path();
    let target = base.join("sample");
    fs::create_dir_all(&target).unwrap();
    let args = ["cargo", "test", "--workspace"].map(std::ffi::OsString::from);
    let key = configuration(base, &args).unwrap();
    assert_eq!(warm_reserve(base, &[], &key).unwrap(), CACHE_SLOT_LIMIT);
    fs::write(target.join("artifact"), vec![0u8; 4096]).unwrap();
    let crate_args = ["cargo", "test", "-p", "example"].map(std::ffi::OsString::from);
    record_size(base, base, &target, &crate_args, true).unwrap();
    assert_eq!(
        warm_reserve(base, &[], &key).unwrap(),
        CACHE_SLOT_LIMIT,
        "a small crate run lowered the bootstrap"
    );
    record_size(base, base, &target, &args, false).unwrap();
    assert_eq!(
        warm_reserve(base, &[], &key).unwrap(),
        CACHE_SLOT_LIMIT,
        "a failed full run qualified"
    );
    record_size(base, base, &target, &args, true).unwrap();
    let measured = used_bytes(&target).unwrap();
    assert_eq!(
        warm_reserve(base, &[], &key).unwrap(),
        measured,
        "a complete compact target did not qualify"
    );
    fs::write(target.join("artifact"), vec![0u8; 16384]).unwrap();
    record_size(base, base, &target, &crate_args, false).unwrap();
    assert_eq!(
        warm_reserve(base, &[], &key).unwrap(),
        used_bytes(&target).unwrap(),
        "failed growth was lost"
    );
    let release_args = ["cargo", "test", "--workspace", "--release"].map(std::ffi::OsString::from);
    let other_key = configuration(base, &release_args).unwrap();
    assert_ne!(key, other_key);
    assert_eq!(
        warm_reserve(base, &[], &other_key).unwrap(),
        CACHE_SLOT_LIMIT
    );
    fs::write(base.join("Cargo.lock"), b"changed dependency configuration").unwrap();
    let changed = configuration(base, &args).unwrap();
    assert_ne!(key, changed);
    assert_eq!(warm_reserve(base, &[], &changed).unwrap(), CACHE_SLOT_LIMIT);
    let growing = Target {
        name: String::new(),
        root: base.to_path_buf(),
        bytes: CACHE_SLOT_LIMIT + 1,
        modified: SystemTime::now(),
        busy: true,
        valid_owner: true,
    };
    assert_eq!(
        warm_reserve(base, &[growing], &key).unwrap(),
        CACHE_SLOT_LIMIT + 1,
        "incomplete live growth was not reserved"
    );
    assert_eq!(
        warm_reserve(base, &[], &key).unwrap(),
        CACHE_SLOT_LIMIT + 1,
        "high-water was not persisted"
    );
}

#[test]
fn malformed_reserve_state_keeps_bootstrap_and_failed_publication_preserves_the_prior_state() {
    let directory = cache_base();
    let base = directory.path();
    for bytes in [
        b"{".to_vec(),
        b"{\"version\":2,\"configuration\":\"x\",\"measured_bytes\":1,\"peak_bytes\":1}".to_vec(),
        b"{\"version\":1,\"configuration\":null,\"measured_bytes\":1,\"peak_bytes\":1}".to_vec(),
        vec![b' '; 513],
    ] {
        fs::write(base.join("build-reserve.json"), bytes).unwrap();
        assert_eq!(
            warm_reserve(base, &[], "fixture").unwrap(),
            CACHE_SLOT_LIMIT
        );
    }
    let prior = fs::read(base.join("build-reserve.json")).unwrap();
    fs::create_dir(base.join("build-reserve.pending")).unwrap();
    assert!(
        warm_reserve(base, &[], "fixture")
            .unwrap_err()
            .contains("publish")
    );
    assert_eq!(fs::read(base.join("build-reserve.json")).unwrap(), prior);
}

#[test]
fn ordinary_prune_keeps_exact_capacity_and_make_room_evicts_at_most_one_verified_idle_target() {
    for unknown in ["missing", "damaged", "mismatched"] {
        let directory = cache_base();
        let base = directory.path();
        let old = SystemTime::now() - Duration::from_secs(120);
        let mut rows = Vec::new();
        for (index, role) in ["gone", "unknown", "held", "oldest", "newest"]
            .into_iter()
            .enumerate()
        {
            let root = base.join(role);
            fs::create_dir_all(&root).unwrap();
            let target = prepare(base, &root, &[], |_| Ok(400 * GIB)).unwrap();
            fs::write(target.join("artifact"), b"warm").unwrap();
            let name = target_name(&root).unwrap();
            open(&base.join("locks").join(format!("{name}.used")))
                .unwrap()
                .set_modified(old + Duration::from_secs(index as u64))
                .unwrap();
            rows.push((root, target, name));
        }
        fs::remove_dir(&rows[0].0).unwrap();
        let owner = base.join("locks").join(format!("{}.owner", rows[1].2));
        match unknown {
            "missing" => fs::remove_file(&owner).unwrap(),
            "damaged" => fs::write(&owner, [0xff, 0xfe]).unwrap(),
            "mismatched" => fs::write(
                &owner,
                fs::canonicalize(&rows[4].0).unwrap().to_str().unwrap(),
            )
            .unwrap(),
            _ => panic!("{unknown}: unknown owner fixture"),
        }
        let _held = try_lock(open(&base.join("locks").join(&rows[2].2)).unwrap())
            .unwrap()
            .unwrap();
        let free = 30 * GIB + 4 * CACHE_SLOT_LIMIT;
        prune_targets(base, SystemTime::now(), free, false, false, "fixture").unwrap();
        assert!(!rows[0].1.exists(), "verified removed checkout was kept");
        assert!(
            rows[1..]
                .iter()
                .all(|(_, path, _)| path.join("artifact").exists()),
            "ordinary cleanup evicted at exact capacity"
        );
        prune_targets(base, SystemTime::now(), free, true, true, "fixture").unwrap();
        assert!(
            rows[1..].iter().all(|(_, path, _)| path.exists()),
            "dry run evicted a target"
        );
        prune_targets(base, SystemTime::now(), free, false, true, "fixture").unwrap();
        assert!(
            !rows[3].1.exists(),
            "oldest verified idle target was not selected"
        );
        for index in [1, 2, 4] {
            assert!(rows[index].1.join("artifact").exists());
        }
        prune_targets(base, SystemTime::now(), free, false, true, "fixture").unwrap();
        assert!(rows[4].1.exists(), "make-room evicted while below capacity");
        let extra = base.join("extra");
        fs::create_dir(&extra).unwrap();
        let extra_target = prepare(base, &extra, &[], |_| Ok(400 * GIB)).unwrap();
        let one = 30 * GIB + CACHE_SLOT_LIMIT;
        prune_targets(base, SystemTime::now(), one, false, true, "fixture").unwrap();
        assert!(!rows[4].1.exists());
        assert!(
            extra_target.exists(),
            "make-room evicted more than one live-checkout target"
        );
        let _extra =
            try_lock(open(&base.join("locks").join(target_name(&extra).unwrap())).unwrap())
                .unwrap()
                .unwrap();
        prune_targets(base, SystemTime::now(), one, false, true, "fixture").unwrap();
        assert!(extra_target.exists());
        assert!(rows[1].1.exists(), "unverified owner was evicted");
        assert!(rows[2].1.exists(), "held target was evicted");
        assert_eq!(
            targets(base).unwrap().len(),
            3,
            "unverified/held targets were counted as reclaimed"
        );
    }
}
