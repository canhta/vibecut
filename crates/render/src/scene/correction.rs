//! Footage corrections render applies inside an item's own picture: stabilization, the
//! person matte and its refinement brush, and flicker removal. Each reads derived data the
//! caller resolved; none opens a cache or media.
pub mod brush;
pub mod flicker;
pub mod matte;
pub mod stabilize;
