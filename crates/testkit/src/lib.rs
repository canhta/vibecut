//! Dev-only media fixtures, generated-fixture paths and FFmpeg/FFprobe oracles.
//!
//! Only tests and examples depend on this crate, and only as a dev-dependency.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

pub mod fixture;
pub mod oracle;
pub mod source;
