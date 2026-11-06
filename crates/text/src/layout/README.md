# Layout

`layout/breaks` computes line ranges from shaped advances and Unicode break opportunities. `layout/paragraph` turns shaped text runs into positioned lines.

`layout(text, &LayoutOptions) -> Result<Layout, TextError>` normalizes the input to NFC, shapes
it with the bundled faces, breaks lines, and returns positioned glyphs and line boxes.
`LayoutOptions` carries the family, style, pixel size, maximum wrap width, alignment, baseline
distance, line limit, an ordered fallback-family chain, a line-breaking mode and letter
spacing. Letter spacing adds its pixel value after every grapheme cluster, including a line's
last one (negative values tighten), and counts toward wrapping and line width; it is finite and
at most 4096 px in magnitude. `style_name(weight, italic)` turns a CSS weight 100..=900 and an
italic flag into the style vocabulary `resolve` understands. All layout coordinates are pixels
with y down; the origin is the top-left of the text box.
## Language data

Language profiles are owned by `captions`; this crate does not match language tags. A profile
supplies `LayoutOptions::fallbacks` (family names in order) and `LayoutOptions::line_break`
(`Unicode` for UAX #14, `Grapheme` to break at every grapheme boundary). Adding a language adds
profile data and font assets, not layout code. An empty chain uses the generic bundled order.
## Limits

- Left-to-right text only; bidirectional reordering and vertical layout are excluded.
- Registered faces are never unloaded within a process.
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
## Performance

No release-mode benchmark is recorded. A debug test lays out 8,000 single-character lines and a
two-line window over the same input under a five-second bound, which guards the former
quadratic wrapping behavior but is not a production benchmark.
