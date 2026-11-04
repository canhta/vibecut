//! Shared accounting policy; storage remains with its domain owner.
use crate::EditError;
use crate::cache::StoredArtifact;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};

/// All charged resident and persistent bytes share one ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOwner {
    DecodedFrames,
    Compositor,
    Export,
    PersistentCache,
    CacheReads,
    JobResults,
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
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub limit: u64,
    pub used: u64,
    pub owners: [u64; 6],
}
/// Clone this handle into every cache/adapter; never instantiate one per cache.
#[derive(Clone)]
pub struct ByteBudget(Arc<BudgetState>);
struct BudgetState {
    accounting: Mutex<BudgetSnapshot>,
    artifacts: Mutex<Vec<Weak<StoredArtifact>>>,
    clock: AtomicU64,
}
impl ByteBudget {
    pub fn new(limit: u64) -> Result<Self, EditError> {
        if limit == 0 {
            return Err(EditError::Limit);
        }
        Ok(Self(Arc::new(BudgetState {
            accounting: Mutex::new(BudgetSnapshot {
                limit,
                used: 0,
                owners: [0; 6],
            }),
            artifacts: Mutex::new(Vec::new()),
            clock: AtomicU64::new(0),
        })))
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
        let mut state = self
            .0
            .accounting
            .lock()
            .map_err(|_| EditError::JobPoisoned)?;
        if bytes > state.limit {
            return Err(EditError::ArtifactTooLarge);
        }
        let next = state.used.checked_add(bytes).ok_or(EditError::BudgetFull)?;
        if next > state.limit {
            return Err(EditError::BudgetFull);
        }
        let count = state
            .owners
            .get_mut(owner.index())
            .ok_or(EditError::Limit)?;
        *count = count.checked_add(bytes).ok_or(EditError::BudgetFull)?;
        state.used = next;
        Ok(ByteLease {
            budget: self.clone(),
            owner,
            bytes,
        })
    }
}
/// Keep alive for the entire allocation/file lifetime, including in-flight users.
pub struct ByteLease {
    budget: ByteBudget,
    owner: ByteOwner,
    bytes: u64,
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
