#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The agent's contracts through its public interface: the refusal policy table, the turn
//! loop over a scripted model and the evaluation set.
mod eval;
mod policy;
mod support;
mod turn;
