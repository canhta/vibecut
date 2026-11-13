//! Carries the user's cancel request into a running turn.
use ai::{ExecutionHooks, Stage};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A shared flag. Cloning shares it, so the UI keeps one handle and the turn reads another.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);
impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
impl ExecutionHooks for CancelFlag {
    fn cancelled(&self) -> bool {
        self.is_cancelled()
    }
    fn stage(&self, _stage: Stage) {}
}
