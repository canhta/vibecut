//! Groups native libraries command adapters.
mod sounds;
pub(super) use sounds::run as sounds;
mod music;
pub(super) use music::run as music;
mod stickers;
pub(super) use stickers::run as stickers;
mod fonts;
pub(super) use fonts::run as fonts;
