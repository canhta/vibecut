# text

Unicode shaping, line layout and CPU rasterization for overlay titles and burn-ins.


| Module | Responsibility | Contract |
|---|---|---|
| `layout/` | Lays out a paragraph from normalized text into positioned lines. | [Layout](src/layout/README.md) |
| `fonts` | Owns the bundled font database and deterministic family/style resolution. | [Fonts](src/fonts/README.md) |
| `color` | Owns the small sRGB colour type text drawing needs. | [source](src/color.rs) |
| `mask` | Computes outlines from an exact Euclidean distance transform. | [Mask coverage](src/mask/README.md) |
| `raster` | Rasterizes glyph outlines and boxes with anti-aliasing. | [source](src/raster.rs) |
| `render` | Extracts glyph outlines and composites premultiplied RGBA8 images. | [Rendering](src/render/README.md) |

The crate root re-exports the same public names at unchanged crate-root paths.

## Contract

Public entry points: `layout`, `render`, `resolve`, `style_name`, `fallback_chain`,
`fallback_for`, `face`, `families`, `register`, `generation`, `is_available`, `FontData`,
`FontError`, `Layout`, `Line`, `Glyph`, `LayoutOptions`, `Align`, `LineBreak`, `TextError`,
`Paint`, `Outline`, `StrokeKind`, `Rectangle`, `Canvas`, `Image`, `RenderError` and `Color`.
Mask and path internals are private; callers paint through `render`.

## Tests and goldens

`cargo test -p text` covers raster coverage, strokes, metrics, fallback order, runtime face
registration limits and conflicts, letter spacing per cluster, wrapping, long
unbreakable words, complete-source line coverage, ligature- and kerning-safe per-line shaping
against isolated spans, mandatory separators, bounded extreme sizes, narrow-wrap scaling,
alignment, truncation and input validation. Integration tests cover:

- the Vietnamese stress string in NFC and NFD: identical glyph runs, no missing glyphs and
  identical rendered pixels;
- Simplified and Traditional Chinese: CJK fallback, no-space wrapping and grapheme-safe breaks;
- deterministic fallback for a character the requested face lacks;
- a 1080×1920 two-line Vietnamese overlay with a 4 px outline, written to `target/text-vi.png`;
- a Chinese overlay written to `target/text-zh.png`;
- clipping at canvas edges, outline coverage outside the fill, glyph-only coverage, and pinned
  FNV-1a checksums of the full rendered buffers.

The tests write PNGs under the repository `target/` and compare repeated renders byte for byte
and NFC/NFD pixels exactly. The pinned checksums are regression goldens: changing them requires
visual inspection and an explained re-bless, never automatic acceptance.
