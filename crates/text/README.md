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
`Paint`, `Outline`, `StrokeKind`, `Rectangle`, `Canvas`, `Image`, `RenderError` and `Color`.
Mask and path internals are private; callers paint through `render`.
`coverage_signed_distance(width, height, coverage, cancelled)` shares the outline
Euclidean distance transform with render's text masks. It accepts exact, finite
0..=1 glyph coverage, returns negative-inside distances in output pixels, and
preserves fractional edge coverage. Dimensions admit at most 64 Mi pixels;
invalid length/values, allocation failure and cancellation return typed errors.
Cancellation is checked per input row and transform row/column. A budgeted caller
reserves 16 bytes/pixel plus 20 bytes per longer axis plus 4 bytes for temporary
and returned distance buffers; the input working image is separately owned.

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

`tools/subset-cjk-font.py` reproduces the subset from the pinned Sans2.004 archive with
fontTools 4.68.0. It checks the source OTF and output SHA-256 values recorded in the attribution
sidecar, so a regeneration mismatch is a hard failure. The source archive and OTF hashes are
also recorded there.

Fallback is deterministic: the preferred face wins when it covers the character; otherwise the
profile chain is searched in order, then the bundled table order. Whitespace and control
characters stay on the preferred face.

## Limits

- Left-to-right text only; bidirectional reordering and vertical layout are excluded.
- No hyphenation, OpenType feature toggles, variable-font axes, colour emoji or hinting.
- Input text is capped at 16 MiB; canvases at 64 M pixels (256 MiB of RGBA8); font size at
  4096 px and explicit line height at 16,384 px.
- CR, CRLF, NEL, line separator, paragraph separator, vertical tab and form feed become LF before
  shaping; mandatory breaks therefore never reach the shaper as tofu.
- Every line's final span is shaped independently whenever a boundary ligature or cross-boundary
  kerning changes its context; a nominal-shaping comparison lets unaffected lines reuse the
  paragraph shape. A multi-grapheme line that still exceeds the width is split further without
  dropping its prefix; a single grapheme wider than the wrap width occupies its own line.
- Line ranges always tile the complete source (apart from intentional max-lines overflow).

## Tests and goldens

`cargo test -p text` covers raster coverage, strokes, metrics, fallback order, wrapping, long
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

## Performance

No release-mode benchmark is recorded. A debug test lays out 8,000 single-character lines and a
two-line window over the same input under a five-second bound, which guards the former
quadratic wrapping behavior but is not a production benchmark.
