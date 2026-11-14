//! Sizes the default byte budget from the host's memory.
use sysinfo::System;

/// Smallest default budget: a 4K single-layer export declares about 1 GiB.
const MIN_HOST_BUDGET: u64 = 1 << 30;
/// Largest default budget; more adds no export or cache speed measured so far.
const MAX_HOST_BUDGET: u64 = 8 << 30;

/// Host memory in bytes; `0` marks a value the probe could not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMemory {
    pub total: u64,
    pub available: u64,
}

/// The OS memory reading; tests inject a small and a large host.
pub trait MemoryProbe {
    fn memory(&self) -> HostMemory;
}

/// Reads the running host (macOS, Windows and Linux) through `sysinfo`.
pub struct SystemMemory;
impl MemoryProbe for SystemMemory {
    fn memory(&self) -> HostMemory {
        let Ok(mut system) = System::new() else {
            return HostMemory {
                total: 0,
                available: 0,
            };
        };
        system.refresh_memory();
        HostMemory {
            total: system.total_memory(),
            available: system.available_memory(),
        }
    }
}

/// A quarter of physical memory, halved again when less than half is available at start,
/// clamped to [`MIN_HOST_BUDGET`]..=[`MAX_HOST_BUDGET`]. An unreadable probe gives the floor.
pub fn host_budget_limit(probe: &dyn MemoryProbe) -> u64 {
    let HostMemory { total, available } = probe.memory();
    (total / 4)
        .min(available / 2)
        .clamp(MIN_HOST_BUDGET, MAX_HOST_BUDGET)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host(u64, u64);
    impl MemoryProbe for Host {
        fn memory(&self) -> HostMemory {
            HostMemory {
                total: self.0,
                available: self.1,
            }
        }
    }
    const GIB: u64 = 1 << 30;
    #[test]
    fn rows_for_small_medium_large_and_unreadable_hosts() {
        for ((total, available), want) in [
            ((4, 1), 1),
            ((8, 4), 2),
            ((8, 2), 1),
            ((16, 10), 4),
            ((16, 2), 1),
            ((64, 50), 8),
            ((256, 200), 8),
            ((0, 0), 1),
        ] {
            let host = Host(total * GIB, available * GIB);
            assert_eq!(host_budget_limit(&host), want * GIB, "{total}/{available}");
        }
    }
    #[test]
    fn system_probe_reads_this_host() {
        let memory = SystemMemory.memory();
        assert!(memory.total > 0 && memory.available <= memory.total);
    }
}
