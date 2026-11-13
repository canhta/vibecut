# Layout

`layout/breaks` computes line ranges from shaped advances and Unicode break opportunities. `layout/paragraph` turns shaped text runs into positioned lines.

`layout(text, &LayoutOptions) -> Result<Layout, TextError>` normalizes the input to NFC, shapes it with the bundled faces, breaks lines and returns positioned glyphs and line boxes. `Layout::caret(source, char_index)` maps a character index in the source string, through its NFC prefix, to a cluster boundary and returns the line, leading x and line-box top/bottom; indices past the end clamp and an index inside a grapheme moves to the next boundary. `Layout::index(source, point)` is its inverse: the nearest line box by y and nearest caret x by x, returned as a source character index. `word_range(source, char_index)` returns the Unicode word boundaries at an index for double-click selection. `LayoutOptions` carries the family, style, pixel size, maximum wrap width, alignment, baseline distance, line limit, an ordered fallback-family chain, a line-breaking mode, letter spacing and a `Case` (as typed, upper, lower, title). The case is applied before shaping, so `Layout::text` holds the cased text; each character maps to one character (a mapping that expands, such as `ß`, keeps its form), and title capitalises the first letter of each Unicode word and leaves the rest as typed. `Layout::caret` and `Layout::index` take the source as passed in and apply the layout's case themselves, so character indices stay on the typed text. Letter spacing adds its pixel value after every grapheme cluster, including a line's last (negative values tighten), counts toward wrapping and line width, and is finite with magnitude at most 4096 px. `style_name(weight, italic)` turns a CSS weight 100..=900 and an italic flag into the style vocabulary `resolve` understands. Coordinates are pixels with y down and the origin at the top-left of the text box. Alignment and `Layout::bounds` use each line's ink width: the advance width, or further when a glyph's ink leans past it (a synthetic italic slant), so right-aligned italic text stays inside the box.

## Language data

Language profiles are owned by `captions`; this crate does not match language tags. A profile supplies `LayoutOptions::fallbacks` (family names in order) and `LayoutOptions::line_break` (`Unicode` for UAX #14, `Grapheme` to break at every grapheme boundary). An empty chain uses the generic bundled order.

## Limits

- Left-to-right text only; no bidirectional reordering or vertical layout.
- Registered faces are never unloaded within a process.
- No hyphenation, OpenType feature toggles, variable-font axes, colour emoji or hinting.
- Input text is capped at 16 MiB, font size at 4096 px and explicit line height at 16,384 px.
- Canvas limits belong to [Rendering](../render/README.md).
- CR, CRLF, NEL, line separator, paragraph separator, vertical tab and form feed become LF before shaping.
- Every line's final span is shaped independently whenever a boundary ligature or cross-boundary kerning changes its context; unaffected lines reuse the paragraph shape. A multi-grapheme line that still exceeds the width is split further without dropping its prefix, and a single grapheme wider than the wrap width occupies its own line.
- Line ranges tile the complete source, apart from intentional max-lines overflow.

`canonical_text` is the shared line-ending-then-NFC normalization used by layout and span callers. Every positioned glyph retains the actual shaping cluster’s exclusive byte end. `Layout::certifies_span` admits only whole canonical graphemes and shaped clusters; it never assigns words by ordinal or reshapes them independently.

A span also needs actual nonmissing glyph coverage. Uncovered scripts keep ordinary base rendering and a warning; bundled font coverage and left-to-right layout limits remain unchanged.

`canonical_boundaries` pairs source grapheme boundaries with their canonical byte offsets using the same normalization. Batch `certifies_spans` checks ordered disjoint spans against all shaped clusters without a word-by-glyph Cartesian scan.
