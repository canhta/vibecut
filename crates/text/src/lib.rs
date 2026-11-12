//! Unicode text shaping, line layout and CPU rasterization for overlay titles.
//!
//! [`layout`] normalizes text to NFC, shapes it with bundled fonts (Inter and Noto Serif),
//! breaks lines at Unicode opportunities without splitting a grapheme cluster, and positions
//! glyphs; [`Layout::caret`] resolves a source character index to its cluster and line box,
//! [`Layout::index`] maps a layout point back to a source index, and [`word_range`] finds the
//! word boundaries a double-click selects.
//! [`render`] turns a layout into a premultiplied RGBA8 image with fill, an outer,
//! centre or inner outline, an optional shadow or glow and an optional background box.
//!
//! The crate is headless and deterministic: it reads no system fonts, uses no GPU and has no
//! clock, network or filesystem input. Callers that own a font catalog hand it font bytes
//! through [`register`]; the registry only grows, so earlier results never change.

mod color;
mod fonts;
mod layout;
mod mask;
mod raster;
mod render;

pub use color::Color;
pub use fonts::{
    DEFAULT_FAMILY, Face, FaceId, FontData, FontError, MAX_FONT_BYTES, MAX_REGISTERED_BYTES,
    MAX_REGISTERED_FACES, Resolved, face, fallback_chain, fallback_for, families, generation,
    is_available, register, resolve, style_name,
};
pub use layout::{
    Align, Bounds, Caret, Glyph, Layout, LayoutOptions, Line, LineBreak, TextError,
    canonical_boundaries, canonical_text, layout, word_range,
};
pub use mask::{StrokeKind, coverage_signed_distance, coverage_signed_distance_bytes};
pub use render::{
    Canvas, Decoration, EmphasisFill, Glow, Image, MAX_EMPHASIS_SPANS, Outline, Paint, Rectangle,
    RenderError, Shadow, SpanFill, render,
};
