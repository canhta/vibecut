//! Groups native ai command adapters.
mod agent;
pub(super) use agent::run as agent;
mod subtitle;
pub(super) use subtitle::run as subtitle;
mod clips;
pub(super) use clips::run as clips;
mod chapters;
pub(super) use chapters::run as chapters;
mod fillers;
pub(super) use fillers::run as fillers;
mod providers;
pub(super) use providers::run as providers;
mod model;
pub(super) use model::run as model;
