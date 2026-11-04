//! Shared accounting policy; storage remains with its domain owner.
use crate::EditError;
use std::sync::{Arc, Mutex};

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
pub struct ByteBudget(Arc<Mutex<BudgetSnapshot>>);
impl ByteBudget {
    pub fn new(limit: u64) -> Result<Self, EditError> {
        if limit == 0 {
            return Err(EditError::Limit);
        }
        Ok(Self(Arc::new(Mutex::new(BudgetSnapshot {
            limit,
            used: 0,
            owners: [0; 6],
        }))))
    }
    pub fn snapshot(&self) -> Result<BudgetSnapshot, EditError> {
        Ok(self.0.lock().map_err(|_| EditError::JobPoisoned)?.clone())
    }
    /// Reserve before allocation. A single oversized artifact is refused.
    pub fn reserve(&self, owner: ByteOwner, bytes: u64) -> Result<ByteLease, EditError> {
        let mut state = self.0.lock().map_err(|_| EditError::JobPoisoned)?;
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
        if let Ok(mut state) = self.budget.0.lock()
            && let Some(count) = state.owners.get_mut(self.owner.index())
        {
            *count = count.saturating_sub(self.bytes);
            state.used = state.used.saturating_sub(self.bytes);
        }
    }
}
