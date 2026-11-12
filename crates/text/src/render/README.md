# Rendering

`render(&Layout, Canvas, origin, &Paint) -> Result<Image, RenderError>` rasterizes glyphs into a premultiplied RGBA8 image with a fill colour, an optional outer, centre or inner outline, an optional underline or line-through bar per line (fill colour, 7% of the font size thick; underline top 10% below the baseline, line-through centred 30% above it) and an optional rounded background box with separate horizontal and vertical padding and a pixel offset from the content bounds. The canvas admits at most 64 M pixels (256 MiB of RGBA8). Rendering uses no GPU, clock, network or filesystem input.

Optional `Paint::span_fill` changes only the RGB fill of one certified canonical byte span. Its alpha is always the base fill alpha. One whole-text coverage mask and one bounded selected-glyph mask are composited once, preserving outlines, background, decorations and antialias alpha. Uncertifiable spans refuse. With no span, the original compositing path and pixels are unchanged.
