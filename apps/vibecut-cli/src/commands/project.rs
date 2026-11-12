//! Groups native project command adapters.
mod lifecycle;
pub(super) use lifecycle::run as project;
mod exec;
pub(super) use exec::run as exec;
mod control;
pub(super) use control::run as control;
mod captions;
pub(super) use captions::run as captions;
mod cut;
pub(super) use cut::run as cut;
mod reframe;
pub(super) use reframe::run as reframe;
mod transcript;
pub(super) use transcript::run as transcript;
