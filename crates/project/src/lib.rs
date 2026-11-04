//! Project persistence primitives.
pub mod atomic;
pub mod autosave;
pub use atomic::atomic_write;
