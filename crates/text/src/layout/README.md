# Layout

`layout/breaks` computes line ranges from shaped advances and Unicode break opportunities. `layout/paragraph` turns shaped text runs into positioned lines.

`layout(text, &LayoutOptions) -> Result<Layout, TextError>` normalizes the input to NFC, shapes it with the bundled faces, breaks lines and returns positioned glyphs and line boxes. `Layout::caret(source, char_index)` maps a character index in the source string, through its NFC prefix, to a cluster boundary and returns the line, leading x and line-box top/bottom; indices past the end clamp and an index inside a grapheme moves to the next boundary. `LayoutOptions` carries the family, style, pixel size, maximum wrap width, alignment, baseline distance, line limit, an ordered fallback-family chain, a line-breaking mode and letter spacing. Letter spacing adds its pixel value after every grapheme cluster, including a line's last (negative values tighten), counts toward wrapping and line width, and is finite with magnitude at most 4096 px. `style_name(weight, italic)` turns a CSS weight 100..=900 and an italic flag into the style vocabulary `resolve` understands. Coordinates are pixels with y down and the origin at the top-left of the text box.

## Language data

Language profiles are owned by `captions`; this crate does not match language tags. A profile supplies `LayoutOptions::fallbacks` (family names in order) and `LayoutOptions::line_break` (`Unicode` for UAX #14, `Grapheme` to break at every grapheme boundary). An empty chain uses the generic bundled order.

## Limits

- Left-to-right text only; no bidirectional reordering or vertical layout.
- Registered faces are never unloaded within a process.
- No hyphenation, OpenType feature toggles, variable-font axes, colour emoji or hinting.
- Input text is capped at 16 MiB, canvases at 64 M pixels (256 MiB of RGBA8), font size at 4096 px and explicit line height at 16,384 px.
- CR, CRLF, NEL, line separator, paragraph separator, vertical tab and form feed become LF before shaping.
- Every line's final span is shaped independently whenever a boundary ligature or cross-boundary kerning changes its context; unaffected lines reuse the paragraph shape. A multi-grapheme line that still exceeds the width is split further without dropping its prefix, and a single grapheme wider than the wrap width occupies its own line.
- Line ranges tile the complete source, apart from intentional max-lines overflow.
