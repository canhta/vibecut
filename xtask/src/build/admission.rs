//! Admit one build command into a FIFO queue and claim a job-budgeted warm slot.
use super::{
    environment::{Memory, START_RAMP, load_within_bound, memory_available, memory_fits},
    queue::{Queue, open, try_lock},
    slot::{Slot, owner},
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    path::Path,
    time::SystemTime,
};

/// Capacity facts fixed for one admission attempt.
#[derive(Clone, Copy)]
pub(super) struct Admission {
    pub(super) max_slots: usize,
    pub(super) cpus: usize,
    pub(super) memory_gated: bool,
    pub(super) caller_jobs: bool,
}
/// Number of slots currently leased by running commands across every number,
/// including numbers above a shrunk capacity. Such builds keep running.
/// Caller holds admission: claims briefly lock free slots before publication.
pub(super) fn active_slots(locks: &Path, budget: usize) -> io::Result<(usize, usize)> {
    let mut active = 0;
    let mut jobs = 0usize;
    for entry in fs::read_dir(locks)? {
        let entry = entry?;
        let name = entry.file_name();
        let is_slot = name
            .to_str()
            .and_then(|name| name.strip_prefix("slot-"))
            .and_then(|number| number.parse::<usize>().ok())
            .is_some();
        if !is_slot {
            continue;
        }
        let file = open(&entry.path())?;
        if try_lock(file)?.is_none() {
            active += 1;
            // Only held leases count. A release during the snapshot can only
            // overcount; claims and publication serialize on admission. Missing
            // or damaged metadata holds admission until that lease exits.
            let reserved = fs::read_to_string(entry.path().with_extension("jobs"))
                .ok()
                .and_then(|text| {
                    if text.trim() == "exclusive" {
                        // Available parallelism can differ between wrappers
                        // (for example, process affinity). Exclusivity must not
                        // depend on the originating wrapper's CPU count.
                        Some(budget)
                    } else {
                        text.trim().parse::<usize>().ok()
                    }
                })
                .filter(|jobs| *jobs > 0)
                .unwrap_or(budget);
            jobs = jobs.saturating_add(reserved);
        }
    }
    Ok((active, jobs))
}
pub(super) fn admit_extra(
    memory_gated: bool,
    busy: bool,
    last_start: Option<SystemTime>,
    cpus: usize,
    running: usize,
    memory: impl FnOnce() -> Option<Memory>,
    load: impl FnOnce() -> Option<f64>,
) -> bool {
    if !memory_gated || !busy {
        return true;
    }
    last_start
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age >= START_RAMP)
        && memory().is_some_and(|memory| memory_fits(memory, running))
        && load_within_bound(load(), cpus)
}

/// Cap link/test fan-out when this build is admitted while free memory is
/// already below the floor. An unreadable source keeps the default fan-out:
/// extra slots are already denied, and non-gated hosts have no source at all.
pub(super) fn pressured_jobs(jobs: usize, memory: Option<Memory>) -> usize {
    if memory.is_some_and(|memory| !memory_available(memory)) {
        (jobs / 2).max(1)
    } else {
        jobs
    }
}

pub(super) fn claim(
    queue: &mut Queue,
    root: &Path,
    admission: Admission,
    memory: impl FnOnce() -> Option<Memory>,
    load: impl FnOnce() -> Option<f64>,
) -> io::Result<Option<Slot>> {
    let _admission = Queue::admission(&queue.locks)?;
    if !queue.first()? {
        return Ok(None);
    }
    // Count every running lease, including numbers above a capacity that has
    // since shrunk. Shrinking never releases or kills those builds.
    let budget = admission.cpus.max(1).saturating_mul(2);
    let (active, reserved) = active_slots(&queue.locks, budget)?;
    if active >= admission.max_slots {
        return Ok(None);
    }
    let remaining = budget.saturating_sub(reserved);
    let mut snapshot = None;
    let jobs = if admission.caller_jobs {
        // Explicit Cargo job settings retain their semantics, including counts
        // above the bound, but run exclusively with respect to default jobs.
        if active != 0 {
            return Ok(None);
        }
        budget
    } else {
        if remaining < 2 {
            return Ok(None);
        }
        snapshot = memory();
        // Share the CPUs that are actually free: a lone build takes the whole
        // host, and each live lease ahead of it halves the remainder. The
        // ceiling bounds leases, not jobs, so it does not price this build.
        pressured_jobs(
            (admission.cpus / (active + 1)).max(2).min(remaining),
            snapshot,
        )
    };
    let mut free = Vec::new();
    for number in 1..=admission.max_slots {
        if let Some(lock) = try_lock(open(&queue.locks.join(format!("slot-{number}")))?)? {
            free.push((number, lock));
        }
    }
    let busy = active > 0;
    let last = fs::metadata(queue.locks.join("last-start"))
        .and_then(|m| m.modified())
        .ok();
    if free.is_empty()
        || !admit_extra(
            admission.memory_gated,
            busy,
            last,
            admission.cpus,
            active,
            || snapshot,
            load,
        )
    {
        return Ok(None);
    }
    // Owner records of removed worktrees are cleared so a gone checkout never
    // keeps a slot looking claimed; the slots are locked and admission is held.
    for (number, _) in &free {
        clear_gone_owner(&queue.locks, *number)?;
    }
    free.sort_by_key(|(number, _)| preference(&queue.locks, *number, root));
    let (number, mut lock) = free
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("no build slot"))?;
    // Empty writes do not reliably advance mtime on Windows. The ramp depends
    // on the admission time, so set it explicitly while admission is locked.
    open(&queue.locks.join("last-start"))?.set_modified(SystemTime::now())?;
    lock.file.set_len(0)?;
    writeln!(lock.file, "{}", root.display())?;
    fs::write(
        queue.locks.join(format!("slot-{number}.jobs")),
        if admission.caller_jobs {
            "exclusive".to_owned()
        } else {
            jobs.to_string()
        },
    )?;
    queue.leave()?;
    Ok(Some(Slot { lock, number, jobs }))
}
/// Lower sorts first: the checkout's own slot, then warm slots (owner gone
/// before owner alive; gone owners by longest idle, live owners by number), and an empty slot only when no warm
/// one is free. An owned slot stays first even when cold: the build there is
/// paid once and grows the warm set, where taking another owner's slot only
/// moves a switch rebuild onto that owner. Idle time is the last admission, recorded in `slot-N.jobs`.
fn preference(locks: &Path, number: usize, root: &Path) -> (u8, bool, SystemTime, usize) {
    let owned = owner(locks, number);
    let warm = locks
        .parent()
        .is_some_and(|base| base.join(format!("target/slot-{number}")).is_dir());
    let idle = fs::metadata(locks.join(format!("slot-{number}.jobs")))
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let tier = if owned == root {
        0
    } else if warm {
        1
    } else {
        2
    };
    // Idle time only orders slots whose worktree is gone. Live owners keep the
    // lowest-number order, so switches concentrate on one slot and the other
    // owners stay warm (replay: longest-idle across live owners spread them).
    let alive = owned.is_dir();
    (
        tier,
        alive,
        if alive { SystemTime::UNIX_EPOCH } else { idle },
        number,
    )
}

fn clear_gone_owner(locks: &Path, number: usize) -> io::Result<()> {
    let owned = owner(locks, number);
    if !owned.as_os_str().is_empty() && !owned.is_dir() {
        fs::write(locks.join(format!("slot-{number}.owner")), b"")?;
    }
    Ok(())
}
pub(super) fn caller_jobs(args: &[OsString], override_value: Option<&OsStr>) -> bool {
    override_value.is_some()
        || args
            .iter()
            .filter_map(|arg| arg.to_str())
            .any(|arg| arg == "--jobs" || arg.starts_with("--jobs=") || arg.starts_with("-j"))
}
