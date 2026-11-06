//! Groups native ai command adapters.
mod subtitle;
pub(super) use subtitle::run as subtitle;
mod clips;
pub(super) use clips::run as clips;
mod providers;
pub(super) use providers::run as providers;
mod model;
pub(super) use model::run as model;
