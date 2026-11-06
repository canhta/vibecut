# Rendering

`render(&Layout, Canvas, origin, &Paint) -> Result<Image, RenderError>` rasterizes the glyphs
into a premultiplied RGBA8 image with a fill colour, an optional outer/centre/inner outline,
an optional underline or line-through bar per line (fill colour, 7% of the font size thick;
underline top 10% below the baseline, line-through centred 30% above it), and an optional
rounded background box with separate horizontal and vertical padding and a pixel offset from
the content bounds. Rendering uses no GPU, clock, network or filesystem input.
