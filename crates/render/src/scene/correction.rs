//! Footage corrections render applies inside an item's own picture: stabilization and the
//! person matte. Each reads derived data the caller resolved; none opens a cache or media.
pub mod brush;
pub mod matte;
pub mod stabilize;
