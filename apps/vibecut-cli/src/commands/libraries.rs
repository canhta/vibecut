//! Groups native libraries command adapters.
mod sounds;
pub(super) use sounds::run as sounds;
mod fonts;
pub(super) use fonts::run as fonts;
