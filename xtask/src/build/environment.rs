//! Host capacity, memory, load and `VIBECUT_BUILD_SLOTS` facts for admission.
#[cfg(any(target_os = "macos", test))]
use std::process::Command;
use std::{ffi::OsStr, time::Duration};
#[cfg(any(target_os = "linux", test))]
use std::{fs, path::Path};

/// Hard ceiling for the per-host slot count and for `VIBECUT_BUILD_SLOTS`.
pub(super) const MAX_SLOTS: usize = 64;
pub(super) const MIN_FREE_PERCENT: u32 = 30;
pub(super) const START_RAMP: Duration = Duration::from_secs(20);
pub(super) const GIB: u64 = 1 << 30;
/// Peak memory one running slot can reach after its compile: the worst measured
/// build (1.5 GiB) plus one 1 GiB link and one 0.5 GiB test binary. Admission
/// holds this reserve for every live lease because link and test fan-out happen
/// after compile, so a running neighbour has not provably passed its peak.
pub(super) const SLOT_PEAK_RESERVE_BYTES: u64 = 3 * GIB;
/// Unix 1-minute load average must stay below this multiple of logical CPUs.
#[cfg(not(windows))]
pub(super) const MAX_LOAD_PER_CPU: f64 = 2.0;
/// Windows exposes aggregate processor time rather than a run-queue length.
/// Admission holds at or above this average busy percentage per logical CPU.
#[cfg(any(windows, test))]
pub(super) const MAX_PROCESSOR_LOAD_PERCENT: f64 = 90.0;
/// An invalid `VIBECUT_BUILD_SLOTS` override, rejected before queueing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SlotsError {
    NotAnInteger(String),
    OutOfRange { value: String, max: usize },
}

impl std::fmt::Display for SlotsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnInteger(value) => write!(
                formatter,
                "VIBECUT_BUILD_SLOTS must be an integer, got {value:?}"
            ),
            Self::OutOfRange { value, max } => write!(
                formatter,
                "VIBECUT_BUILD_SLOTS must be between 1 and {max} on this host, got {value:?}"
            ),
        }
    }
}

impl std::error::Error for SlotsError {}
/// One physical-memory snapshot. Unreadable, zero-total or available-over-total
/// facts are rejected so admission fails closed instead of reserving nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Memory {
    pub(super) available: u64,
    pub(super) total: u64,
}

impl Memory {
    pub(super) fn new(available: u64, total: u64) -> Option<Self> {
        (total > 0 && available <= total).then_some(Self { available, total })
    }
}

/// Bytes reserved by `running` live slots. Saturates so a hostile running count
/// can never wrap the reserve into spare memory.
pub(super) fn reserved_bytes(running: usize) -> u64 {
    u64::try_from(running)
        .unwrap_or(u64::MAX)
        .saturating_mul(SLOT_PEAK_RESERVE_BYTES)
}

/// Does free memory minus every running slot's peak reserve stay at or above
/// `MIN_FREE_PERCENT`? A neighbour admitted during compile still has its link
/// and test peak ahead, so its whole reserve is outstanding until it exits.
pub(super) fn memory_fits(memory: Memory, running: usize) -> bool {
    let spare = u128::from(memory.available).saturating_sub(u128::from(reserved_bytes(running)));
    spare * 100 >= u128::from(memory.total) * u128::from(MIN_FREE_PERCENT)
}

/// Does the instant snapshot meet `MIN_FREE_PERCENT` before reserves?
pub(super) fn memory_available(memory: Memory) -> bool {
    memory_fits(memory, 0)
}

/// Upper bound from host facts alone. CPU bounds the number of slots. Memory
/// keeps the 30% admission reservation and divides the rest by the peak one
/// running slot reserves; the first build is never memory-gated, so one slot
/// is always available on top of that division. An unreadable total memory
/// returns `None`, which holds extra admission.
pub(super) fn derived_slots(cpus: usize, total_memory: Option<u64>) -> Option<usize> {
    let total = total_memory?;
    let cpu_bound = cpus.max(1);
    let budget = total.saturating_mul(u64::from(100 - MIN_FREE_PERCENT)) / 100;
    let memory_bound = usize::try_from(budget / SLOT_PEAK_RESERVE_BYTES)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    Some(cpu_bound.min(memory_bound).clamp(1, MAX_SLOTS))
}

/// Resolve `VIBECUT_BUILD_SLOTS` or derive the ceiling. A non-gated Unix host
/// keeps its CPU-derived capacity and admits by available slots alone.
pub(super) fn configured_slots(
    memory_gated: bool,
    override_value: Option<&OsStr>,
    cpus: usize,
    total_memory: Option<u64>,
) -> Result<usize, SlotsError> {
    if let Some(value) = override_value {
        let text = value
            .to_str()
            .ok_or_else(|| SlotsError::NotAnInteger(value.to_string_lossy().into_owned()))?;
        let slots = text
            .parse::<usize>()
            .map_err(|_| SlotsError::NotAnInteger(text.to_owned()))?;
        // An override may not exceed the CPU bound: every command still runs at
        // least two jobs, so aggregate jobs stay within twice the logical CPUs.
        let max = cpus.clamp(1, MAX_SLOTS);
        if !(1..=max).contains(&slots) {
            return Err(SlotsError::OutOfRange {
                value: text.to_owned(),
                max,
            });
        }
        return Ok(slots);
    }
    if memory_gated {
        Ok(derived_slots(cpus, total_memory).unwrap_or(1))
    } else {
        Ok(cpus.clamp(1, MAX_SLOTS))
    }
}

/// Unix load average bound: a finite, non-negative 1-minute load below
/// twice the logical CPUs.
#[cfg(not(windows))]
pub(super) fn loadavg_within_bound(load: Option<f64>, cpus: usize) -> bool {
    load.is_some_and(|load| {
        load.is_finite() && load >= 0.0 && load < MAX_LOAD_PER_CPU * cpus.max(1) as f64
    })
}

/// Windows processor-load bound: a finite, non-negative aggregate busy
/// percentage below the documented per-CPU ceiling.
#[cfg(any(windows, test))]
pub(super) fn processor_load_within_bound(load: Option<f64>) -> bool {
    load.is_some_and(|load| load.is_finite() && (0.0..MAX_PROCESSOR_LOAD_PERCENT).contains(&load))
}

pub(super) fn load_within_bound(load: Option<f64>, cpus: usize) -> bool {
    #[cfg(windows)]
    {
        let _ = cpus;
        processor_load_within_bound(load)
    }
    #[cfg(not(windows))]
    {
        loadavg_within_bound(load, cpus)
    }
}
// Each platform reads physical memory from exactly one source. The `memory`
// dispatcher below is the only admission path: every gated platform feeds the
// same `MIN_FREE_PERCENT` threshold, slot peak reserve and `START_RAMP` in
// `admit_extra`. macOS reads the `sysctl` level and `hw.memsize`, Linux parses
// `/proc/meminfo`, Windows calls `GlobalMemoryStatusEx`. A source that cannot
// be read or parsed returns `None` (fail closed), which holds the extra slot.

#[cfg(any(target_os = "macos", test))]
pub(super) fn sysctl_memory(program: impl AsRef<OsStr>) -> Option<Memory> {
    let program = program.as_ref();
    let level = Command::new(program)
        .args(["-n", "kern.memorystatus_level"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|level| *level <= 100)?;
    let total = sysctl_total_memory(program)?;
    let available = u64::try_from(u128::from(total) * u128::from(level) / 100).ok()?;
    Memory::new(available, total)
}

/// Build bytes facts from one `/proc/meminfo` text. Overflow fails closed.
#[cfg(any(target_os = "linux", test))]
pub(super) fn meminfo_memory(text: &str) -> Option<Memory> {
    let meminfo = parse_meminfo(text)?;
    Memory::new(
        meminfo.available_kb.checked_mul(1024)?,
        meminfo.total_kb.checked_mul(1024)?,
    )
}

/// Read Linux memory from `/proc/meminfo`. A missing or unreadable file keeps
/// `None` (holds the extra slot), as a failed macOS/Windows probe does.
#[cfg(any(target_os = "linux", test))]
pub(super) fn proc_meminfo_memory(path: impl AsRef<Path>) -> Option<Memory> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| meminfo_memory(&text))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn proc_meminfo_available(path: impl AsRef<Path>) -> bool {
    proc_meminfo_memory(path).is_some_and(memory_available)
}

/// Pure `/proc/meminfo` parser. A trustworthy line is exactly
/// `"Key: <u64> kB"`. Missing keys, a zero total, or any hostile value
/// (overflow, non-numeric, missing/wrong `kB` suffix, trailing tokens) fails
/// the whole probe rather than being ignored. `u128` keeps the comparison from
/// overflowing on adversarial `u64` values.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Meminfo {
    pub(super) total_kb: u64,
    pub(super) available_kb: u64,
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn parse_meminfo(text: &str) -> Option<Meminfo> {
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
        *slot = Some(parsed?);
    }
    match (total_kb, available_kb) {
        (Some(total_kb), Some(available_kb)) if total_kb > 0 && available_kb <= total_kb => {
            Some(Meminfo {
                total_kb,
                available_kb,
            })
        }
        _ => None,
    }
}

/// Decide whether `MemAvailable` meets `MIN_FREE_PERCENT` of `MemTotal`.
#[cfg(any(target_os = "linux", test))]
pub(super) fn meminfo_memory_available(text: &str) -> bool {
    meminfo_memory(text).is_some_and(memory_available)
}

/// Read total physical memory from `/proc/meminfo`; an unreadable or
/// unparsable source holds extra admission instead of guessing capacity.
#[cfg(any(target_os = "linux", test))]
pub(super) fn proc_meminfo_total(path: impl AsRef<Path>) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    parse_meminfo(&text).map(|meminfo| meminfo.total_kb.saturating_mul(1024))
}

/// Read total physical memory from `sysctl hw.memsize`.
#[cfg(any(target_os = "macos", test))]
pub(super) fn sysctl_total_memory(program: impl AsRef<OsStr>) -> Option<u64> {
    Command::new(program)
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
}

/// Pure first field of `/proc/loadavg`: the 1-minute run-queue length.
/// Truncated, non-numeric, negative, infinite or NaN input fails closed.
#[cfg(any(target_os = "linux", test))]
pub(super) fn parse_loadavg(text: &str) -> Option<f64> {
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() < 3 {
        return None;
    }
    for field in fields.iter().take(3) {
        let value = field.parse::<f64>().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
    }
    fields.first()?.parse::<f64>().ok()
}

/// Read the Linux 1-minute load average. A missing or malformed source
/// returns `None`, which holds extra admission.
#[cfg(any(target_os = "linux", test))]
pub(super) fn proc_loadavg(path: impl AsRef<Path>) -> Option<f64> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_loadavg(&text))
}

/// Parse `sysctl vm.loadavg` output `{ <1m> <5m> <15m> }`.
#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_sysctl_load(text: &str) -> Option<f64> {
    let mut fields = text.trim().trim_start_matches('{').split_whitespace();
    let load = fields.next()?.parse::<f64>().ok()?;
    for _ in 0..2 {
        let field = fields.next()?;
        let value = field.trim_end_matches('}').parse::<f64>().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
    }
    (load.is_finite() && load >= 0.0).then_some(load)
}

/// Read the macOS 1-minute load average from `sysctl vm.loadavg`, whose output
/// is `{ <1m> <5m> <15m> }`.
#[cfg(any(target_os = "macos", test))]
pub(super) fn sysctl_load(program: impl AsRef<OsStr>) -> Option<f64> {
    let text = Command::new(program)
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())?;
    parse_sysctl_load(&text)
}

/// Windows processor load from two `GetSystemTimes` samples. On multiprocessor
/// systems each value is summed over all processors, and kernel time includes
/// idle time (Microsoft, `GetSystemTimes`), so busy fraction is
/// `1 - idle_delta / (kernel_delta + user_delta)`.
#[cfg(any(windows, test))]
pub(super) fn processor_load(
    idle: (u64, u64),
    kernel: (u64, u64),
    user: (u64, u64),
) -> Option<f64> {
    let idle = u128::from(idle.1.checked_sub(idle.0)?);
    let kernel = u128::from(kernel.1.checked_sub(kernel.0)?);
    let user = u128::from(user.1.checked_sub(user.0)?);
    let total = kernel.checked_add(user)?;
    if total == 0 || idle > total {
        return None;
    }
    Some((total - idle) as f64 / total as f64)
}

/// One Windows processor-load sample in percent, separated by the short
/// interval the counters need to advance.
#[cfg(windows)]
pub(super) fn windows_processor_load() -> Option<f64> {
    let sample = || {
        winsafe::GetSystemTimes()
            .ok()
            .map(|(idle, kernel, user)| (u64::from(idle), u64::from(kernel), u64::from(user)))
    };
    let first = sample()?;
    std::thread::sleep(Duration::from_millis(250));
    let second = sample()?;
    processor_load(
        (first.0, second.0),
        (first.1, second.1),
        (first.2, second.2),
    )
    .map(|fraction| fraction * 100.0)
}

pub(super) fn total_memory() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        sysctl_total_memory("sysctl")
    }
    #[cfg(target_os = "linux")]
    {
        proc_meminfo_total("/proc/meminfo")
    }
    #[cfg(windows)]
    {
        winsafe::GlobalMemoryStatusEx()
            .ok()
            .map(|memory| memory.ullTotalPhys)
            .filter(|bytes| *bytes > 0)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

pub(super) fn load() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        sysctl_load("sysctl")
    }
    #[cfg(target_os = "linux")]
    {
        proc_loadavg("/proc/loadavg")
    }
    #[cfg(windows)]
    {
        windows_processor_load()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

/// One Windows memory snapshot in bytes.
#[cfg(windows)]
pub(super) fn windows_memory() -> Option<Memory> {
    winsafe::GlobalMemoryStatusEx()
        .ok()
        .and_then(|memory| Memory::new(memory.ullAvailPhys, memory.ullTotalPhys))
}

pub(super) fn memory() -> Option<Memory> {
    #[cfg(target_os = "macos")]
    {
        sysctl_memory("sysctl")
    }
    #[cfg(target_os = "linux")]
    {
        proc_meminfo_memory("/proc/meminfo")
    }
    #[cfg(windows)]
    {
        windows_memory()
    }
    // Unix platforms without a memory probe keep the historical "admit by
    // available slots alone" behaviour: `run_in` passes `memory_gated = false`
    // there, so this dispatcher is only consulted for job pressure, where an
    // unknown source leaves the default fan-out unchanged.
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}
