# text

Unicode shaping, line layout and CPU rasterization for overlay titles and burn-ins.

## Contract

`layout(text, &LayoutOptions) -> Result<Layout, TextError>` normalizes the input to NFC, shapes
it with the bundled faces, breaks lines, and returns positioned glyphs and line boxes.
`LayoutOptions` carries the family, style, pixel size, maximum wrap width, alignment, baseline
distance, line limit, an ordered fallback-family chain and a line-breaking mode. All layout
coordinates are pixels with y down; the origin is the top-left of the text box.

`render(&Layout, Canvas, origin, &Paint) -> Result<Image, RenderError>` rasterizes the glyphs
into a premultiplied RGBA8 image with a fill colour, an optional outer/centre/inner outline and
an optional rounded background box. Rendering uses no GPU, clock, network or filesystem input.

Public entry points: `layout`, `render`, `resolve`, `fallback_chain`, `fallback_for`, `face`,
`families`, `Layout`, `Line`, `Glyph`, `LayoutOptions`, `Align`, `LineBreak`, `TextError`,
`Paint`, `Outline`, `StrokeKind`, `Rectangle`, `Canvas`, `Image`, `RenderError`, `Color`,
`Mask`, `Path` and `Xform`.

## Language data

Language profiles are owned by `captions`; this crate does not match language tags. A profile
supplies `LayoutOptions::fallbacks` (family names in order) and `LayoutOptions::line_break`
(`Unicode` for UAX #14, `Grapheme` to break at every grapheme boundary). Adding a language adds
profile data and font assets, not layout code. An empty chain uses the generic bundled order.

## Fonts

| Family | Faces | Source |
|---|---|---|
| Inter | Regular, Medium, SemiBold, Bold, Italic | OFL-1.1 |
| Noto Serif | Regular | OFL-1.1 |
| VibeCut Sans CJK | Regular | OFL-1.1 subset of Noto Sans SC |

The CJK face is a 2.1 MB subset covering GB2312, Big5 level 1, ASCII, CJK punctuation and
fullwidth forms (9,804 characters). The full Noto Sans SC Regular face is 8.3 MB; the subset
keeps the repository asset small while covering common Simplified and Traditional Chinese.
Characters outside the subset resolve deterministically to the preferred face and render as a
missing glyph. A full-font or downloadable-font follow-up can extend coverage without changing
layout logic. Every asset has a licence and attribution sidecar under `assets/fonts/`.

Fallback is deterministic: the preferred face wins when it covers the character; otherwise the
profile chain is searched in order, then the bundled table order. Whitespace and control
characters stay on the preferred face.

## Limits

- Left-to-right text only; bidirectional reordering and vertical layout are excluded.
- No hyphenation, OpenType feature toggles, variable-font axes, colour emoji or hinting.
- Input text is capped at 16 MiB; canvases at 64 M pixels (256 MiB of RGBA8).
- A single grapheme wider than the wrap width occupies its own line.

## Tests and goldens

`cargo test -p text` covers raster coverage, strokes, metrics, fallback order, wrapping, long
unbreakable words, alignment, truncation and input validation. Integration tests cover:

- the Vietnamese stress string in NFC and NFD: identical glyph runs, no missing glyphs and
  identical rendered pixels;
- Simplified and Traditional Chinese: CJK fallback, no-space wrapping and grapheme-safe breaks;
- deterministic fallback for a character the requested face lacks;
- a 1080×1920 two-line Vietnamese overlay with a 4 px outline, written to `target/text-vi.png`;
- a Chinese overlay written to `target/text-zh.png`.

The tests write PNGs under the repository `target/` and compare repeated renders byte for byte
and NFC/NFD pixels exactly. They are CPU-only and deterministic on a given build.

## Performance

No benchmark is recorded yet; performance is unverified beyond the test suite completing on the
development machine.
