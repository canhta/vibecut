//! Integration tests share one binary and one copy of their helpers.

mod cli;
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
mod mcp;
