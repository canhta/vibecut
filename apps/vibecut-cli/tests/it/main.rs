//! Integration tests share one binary and one copy of their helpers.

mod cli;
mod mcp;
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod media_fixture;
