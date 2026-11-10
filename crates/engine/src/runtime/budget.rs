//! Shared accounting policy; storage remains with its domain owner.
use crate::EditError;
use crate::cache::StoredArtifact;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};

/// Shared accounting categories. Native model disk usage has separate free-disk admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOwner {
    DecodedFrames,
    Compositor,
    Export,
    PersistentCache,
    CacheReads,
    JobResults,
    /// Reported installed/checkpoint model disk bytes; separately preflighted, not memory admission.
    ModelWeights,
}
impl ByteOwner {
    fn index(self) -> usize {
        match self {
            Self::DecodedFrames => 0,
            Self::Compositor => 1,
            Self::Export => 2,
            Self::PersistentCache => 3,
            Self::CacheReads => 4,
            Self::JobResults => 5,
            Self::ModelWeights => 6,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub limit: u64,
    pub used: u64,
    pub owners: [u64; 7],
}
impl BudgetSnapshot {
    /// Native model disk bytes are reported, but admitted by free-disk preflight.
    pub fn model_bytes(&self) -> u64 {
        self.owners
            .get(ByteOwner::ModelWeights.index())
            .copied()
            .unwrap_or(0)
    }
    /// Bytes constrained by limit; excludes separately reported native model disk usage.
    pub fn limited_used(&self) -> u64 {
        self.used.saturating_sub(self.model_bytes())
    }
}
/// Clone this handle into every cache/adapter; never instantiate one per cache.
#[derive(Clone)]
pub struct ByteBudget(Arc<BudgetState>);
struct BudgetState {
    parent: Option<ByteBudget>,
    accounting: Mutex<BudgetSnapshot>,
    artifacts: Mutex<Vec<Weak<StoredArtifact>>>,
    clock: AtomicU64,
}
impl ByteBudget {
    pub fn new(limit: u64) -> Result<Self, EditError> {
        Self::with_parent(limit, None)
    }
    fn with_parent(limit: u64, parent: Option<Self>) -> Result<Self, EditError> {
        if limit == 0 {
            return Err(EditError::Limit);
        }
        Ok(Self(Arc::new(BudgetState {
            parent,
            accounting: Mutex::new(BudgetSnapshot {
                limit,
                used: 0,
                owners: [0; 7],
            }),
            artifacts: Mutex::new(Vec::new()),
            clock: AtomicU64::new(0),
        })))
    }
    /// Per-project admission with aggregate app accounting. Every child lease
    /// also retains a parent lease; refusal rolls back both reservations.
    pub fn child(&self, limit: u64) -> Result<Self, EditError> {
        Self::with_parent(limit, Some(self.clone()))
    }
    pub(crate) fn same_policy(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn snapshot(&self) -> Result<BudgetSnapshot, EditError> {
        Ok(self
            .0
            .accounting
            .lock()
            .map_err(|_| EditError::JobPoisoned)?
            .clone())
    }
    pub(crate) fn stamp(&self) -> u64 {
        if let Some(parent) = &self.0.parent {
            return parent.stamp();
        }
        let mut current = self.0.clock.load(Ordering::Relaxed);
        loop {
            match self.0.clock.compare_exchange_weak(
                current,
                current.saturating_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return current,
                Err(next) => current = next,
            }
        }
    }

    pub(crate) fn register(&self, artifact: &Arc<StoredArtifact>) -> Result<(), EditError> {
        let mut artifacts = self
            .0
            .artifacts
            .lock()
            .map_err(|_| EditError::JobPoisoned)?;
        artifacts.retain(|entry| entry.upgrade().is_some_and(|a| !a.removed()));
        artifacts.push(Arc::downgrade(artifact));
        drop(artifacts);
        if let Some(parent) = &self.0.parent {
            parent.register(artifact)?;
        }
        Ok(())
    }
    /// Reserve before allocation. On pressure, reclaim unpinned persistent
    /// artifacts in aggregate LRU order. No accounting/registry lock spans IO.
    /// Active pins/work can still yield BudgetFull; IO failures retain their source.
    pub fn reserve(&self, owner: ByteOwner, bytes: u64) -> Result<ByteLease, EditError> {
        match self.try_reserve(owner, bytes) {
            Err(EditError::BudgetFull) => {}
            result => return result,
        }
        let mut candidates: Vec<_> = {
            let mut artifacts = self
                .0
                .artifacts
                .lock()
                .map_err(|_| EditError::JobPoisoned)?;
            artifacts.retain(|entry| entry.upgrade().is_some_and(|a| !a.removed()));
            artifacts
                .iter()
                .filter_map(Weak::upgrade)
                .map(|artifact| (artifact.stamp(), artifact))
                .collect()
        };
        // Snapshot recency once: concurrent reads must not change sort keys.
        candidates.sort_by_key(|(stamp, _)| *stamp);
        for (_, artifact) in candidates {
            if artifact.evict()? {
                match self.try_reserve(owner, bytes) {
                    Err(EditError::BudgetFull) => {}
                    result => return result,
                }
            }
        }
        Err(EditError::BudgetFull)
    }
    fn try_reserve(&self, owner: ByteOwner, bytes: u64) -> Result<ByteLease, EditError> {
        let parent = self
            .0
            .parent
            .as_ref()
            .map(|p| p.reserve(owner, bytes).map(Box::new))
            .transpose()?;
        let mut state = self
            .0
            .accounting
            .lock()
            .map_err(|_| EditError::JobPoisoned)?;
        if owner != ByteOwner::ModelWeights && bytes > state.limit {
            return Err(EditError::ArtifactTooLarge);
        }
        let next = state.used.checked_add(bytes).ok_or(EditError::BudgetFull)?;
        let memory = state.limited_used();
        if owner != ByteOwner::ModelWeights
            && memory.checked_add(bytes).is_none_or(|v| v > state.limit)
        {
            return Err(EditError::BudgetFull);
        }
        let count = state
            .owners
            .get_mut(owner.index())
            .ok_or(EditError::Limit)?;
        *count = count.checked_add(bytes).ok_or(EditError::BudgetFull)?;
        state.used = next;
        Ok(ByteLease {
            parent,
            budget: self.clone(),
            owner,
            bytes,
        })
    }
}
/// Keep alive for the entire allocation/file lifetime, including in-flight users.
pub struct ByteLease {
    parent: Option<Box<ByteLease>>,
    budget: ByteBudget,
    owner: ByteOwner,
    bytes: u64,
}
impl ByteLease {
    pub(crate) fn resize_models(&mut self, bytes: u64) -> Result<(), EditError> {
        if self.owner != ByteOwner::ModelWeights {
            return Err(EditError::Invalid("model accounting owner"));
        }
        let mut state = self
            .budget
            .0
            .accounting
            .lock()
            .map_err(|_| EditError::JobPoisoned)?;
        let next = state
            .used
            .checked_sub(self.bytes)
            .and_then(|n| n.checked_add(bytes))
            .ok_or(EditError::BudgetFull)?;
        let count = state
            .owners
            .get_mut(self.owner.index())
            .ok_or(EditError::Limit)?;
        let next_count = count
            .checked_sub(self.bytes)
            .and_then(|n| n.checked_add(bytes))
            .ok_or(EditError::BudgetFull)?;
        if let Some(parent) = &mut self.parent {
            parent.resize_models(bytes)?;
        }
        *count = next_count;
        state.used = next;
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for ByteLease {
    fn drop(&mut self) {
        // Poison is never recovered: future admissions fail closed.
        if let Ok(mut state) = self.budget.0.accounting.lock()
            && let Some(count) = state.owners.get_mut(self.owner.index())
        {
            *count = count.saturating_sub(self.bytes);
            state.used = state.used.saturating_sub(self.bytes);
        }
    }
}

/// The one bridge from render's memory kinds to aggregate byte accounting.
/// Construct it beside the `ByteBudget` its caller owns; an underlying hooks
/// owner supplies cancellation, scheduling and progress. This adapter never
/// seals publication: an irreversible output keeps that fence with its job.
pub struct ByteBudgetHooks<'a> {
    budget: ByteBudget,
    execution: Option<&'a dyn render::ExecutionHooks>,
}

impl<'a> ByteBudgetHooks<'a> {
    pub fn new(budget: &ByteBudget, execution: Option<&'a dyn render::ExecutionHooks>) -> Self {
        Self {
            budget: budget.clone(),
            execution,
        }
    }
}

impl render::ExecutionHooks for ByteBudgetHooks<'_> {
    fn cancelled(&self) -> bool {
        self.execution
            .is_some_and(|execution| execution.cancelled())
    }
    fn parallelism(&self) -> usize {
        self.execution
            .map_or(1, |execution| execution.parallelism())
    }
    fn run_tasks(
        &self,
        tasks: &mut [&mut (dyn FnMut() -> render::Result<()> + Send)],
    ) -> render::Result<()> {
        match self.execution {
            Some(execution) => execution.run_tasks(tasks),
            None => {
                for task in tasks {
                    task()?;
                }
                Ok(())
            }
        }
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        if self.cancelled() {
            return Err(render::Error::Cancelled);
        }
        let owner = match kind {
            render::MemoryKind::Decoded => ByteOwner::DecodedFrames,
            render::MemoryKind::Compositor => ByteOwner::Compositor,
            render::MemoryKind::Export => ByteOwner::Export,
        };
        self.budget
            .reserve(owner, bytes)
            .map(|lease| Box::new(lease) as Box<dyn render::Reservation>)
            .map_err(|error| match error {
                EditError::Io(source) => render::Error::Io(source),
                _ => render::Error::Budget {
                    requested: bytes,
                    limit: self
                        .budget
                        .snapshot()
                        .map_or(0, |snapshot| snapshot.limit.saturating_sub(snapshot.used)),
                },
            })
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        match self.execution {
            Some(execution) => execution.progress(stage, fraction),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use render::ExecutionHooks;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn adapter_charges_each_kind_to_its_owner_and_releases_on_drop() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let hooks = ByteBudgetHooks::new(&budget, None);
        for (kind, owner) in [
            (render::MemoryKind::Decoded, 0),
            (render::MemoryKind::Compositor, 1),
            (render::MemoryKind::Export, 2),
        ] {
            let lease = hooks.reserve(kind, 4096).unwrap();
            let snapshot = budget.snapshot().unwrap();
            assert_eq!(snapshot.owners[owner], 4096);
            assert_eq!(snapshot.used, 4096);
            drop(lease);
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
    }

    #[test]
    fn adapter_refuses_with_budget_and_the_real_remaining_bytes() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let hooks = ByteBudgetHooks::new(&budget, None);
        let held = budget.reserve(ByteOwner::Export, 1024).unwrap();
        let Err(error) = hooks.reserve(render::MemoryKind::Compositor, 2 << 20) else {
            panic!("oversized reservation was admitted");
        };
        assert!(matches!(
            error,
            render::Error::Budget {
                requested: 2_097_152,
                limit: 1_047_552,
            }
        ));
        drop(held);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }

    struct Probe(AtomicBool);

    impl render::ExecutionHooks for Probe {
        fn cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn reserve(
            &self,
            _: render::MemoryKind,
            _: u64,
        ) -> render::Result<Box<dyn render::Reservation>> {
            panic!("the adapter owns reservations");
        }
    }

    #[test]
    fn adapter_checks_its_optional_execution_owner_before_reserving() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let probe = Probe(AtomicBool::new(false));
        let hooks = ByteBudgetHooks::new(&budget, Some(&probe));
        assert!(!hooks.cancelled());
        assert!(hooks.reserve(render::MemoryKind::Decoded, 16).is_ok());
        assert_eq!(budget.snapshot().unwrap().used, 0);
        probe.0.store(true, Ordering::SeqCst);
        assert!(hooks.cancelled());
        assert!(matches!(
            hooks.reserve(render::MemoryKind::Decoded, 16),
            Err(render::Error::Cancelled)
        ));
    }
}
