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

The crate root re-exports the same public names. Mask and path internals are private; callers paint through `render`.

Pinned FNV-1a checksums of full rendered buffers are regression goldens. Changing one requires visual inspection and an explained re-bless, never automatic acceptance.
