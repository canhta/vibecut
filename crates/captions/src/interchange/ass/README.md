# ASS import and style mapping

`interchange/ass/style` maps ASS style records and colours onto cue styles.

`parse_ass(bytes)` imports Advanced SubStation Alpha/SSA text as typed `AssCue`
values with an optional static `CueStyle`; it is deliberately as forgiving as
`parse`. It reads `[Script Info]` PlayResX/PlayResY (defaults 384x288), both
`[V4 Styles]` (SSA alignment 1-3/5-7/9-11, `TertiaryColour` outline, decimal
colours) and `[V4+ Styles]` formats, and `[Events]` dialogue records. The
same encoding rules apply, including UTF-8/UTF-16 BOM and Windows-1252
fallback. Documents are capped at 16 MiB, 100,000 cues and 10,000 styles; a
single line over 1 MiB is skipped. Warnings retain one-based line numbers and
are capped at 1,000 with a suppressed count.

Style mapping keeps font family, font size as a ratio of PlayResY, primary,
outline and background colours as RGBA, numpad alignment plus resolved
horizontal/vertical placement, left/right/vertical margins as ratios of PlayResX
or PlayResY, border style, bold, italic, underline and strikeout. Unknown or
missing styles fall back to the built-in/default style with a warning. Cue
overrides adjust the same fields for `\b`, `\i`, `\u`, `\s` (a bare tag
restores the style value), `\fs`, `\an` and `\c`/`\1c`, `\3c`, `\4c` (alpha stays
with the style); tags inside a `\t(...)` transform never apply, and text inside
a `\p` vector drawing is dropped. Transform, karaoke, clipping, scaling and
animation tags (`\pos`, `\move`, `\fad`, `\t`, `\k`, `\clip`, ...) are
stripped and reported instead of inventing static rendering. `\N`, `\n` and
`\h` become visible text and override blocks never leak into cue text.

Malformed fields, timestamps, zero/backward durations, empty text and
unsupported event kinds (`Comment` is ignored silently) warn and skip; a font
size or margin that is negative, non-finite or over ten script heights is
replaced with the default and reported as `limit`; parsing never panics on truncated input.
Parse cost is linear in line count. Engine `captions.import` turns a parsed
document into one caption item and one undo step; the UI import dialog remains
owned by its ticket.
