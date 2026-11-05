//! Unicode text shaping, line layout and CPU rasterization for overlay titles.
//!
//! [`layout`] normalizes text to NFC, shapes it with bundled fonts (Inter and Noto Serif),
//! breaks lines at Unicode opportunities without splitting a grapheme cluster, and positions
//! glyphs. [`render`] turns a layout into a premultiplied RGBA8 image with fill, an outer,
//! centre or inner outline and an optional background box.
//!
//! The crate is headless and deterministic: it reads no system fonts, uses no GPU and has no
//! clock, network or filesystem input.

mod color;
mod fonts;
mod layout;
mod mask;
mod raster;
mod render;

pub use color::Color;
pub use fonts::{Face, FaceId, Resolved, face, fallback_chain, fallback_for, families, resolve};
pub use layout::{Align, Bounds, Glyph, Layout, LayoutOptions, Line, LineBreak, TextError, layout};
pub use mask::{StrokeKind, coverage_signed_distance};
pub use render::{Canvas, Image, Outline, Paint, Rectangle, RenderError, render};
