//! Groups native catalog command adapters.
mod commands;
pub(super) use commands::run as commands;
mod schema;
pub(super) use schema::run as schema;
