//! The last action ids the app ran, kept in memory for the crash log.
use std::sync::{Mutex, TryLockError};

/// How many ids the crash log lists.
const KEPT: usize = 16;

/// Oldest first; ids only, never an argument, a path or text.
static RECENT: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Remember that the registry action `id` ran, dropping the oldest past `KEPT`.
pub(crate) fn remember(id: &'static str) {
    let mut recent = match RECENT.lock() {
        Ok(recent) => recent,
        Err(poisoned) => poisoned.into_inner(),
    };
    if recent.len() >= KEPT {
        recent.remove(0);
    }
    recent.push(id);
}

/// The remembered ids, oldest first. Never blocks: a panic raised while the list is held
/// reads it as empty rather than waiting on it.
pub(crate) fn recent() -> Vec<&'static str> {
    match RECENT.try_lock() {
        Ok(recent) => recent.clone(),
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().clone(),
        Err(TryLockError::WouldBlock) => Vec::new(),
    }
}
