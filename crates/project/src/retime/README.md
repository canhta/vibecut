# Retime

Video and audio items carry `retime { speed, maintain_pitch, window? }` (default
1x, pitch preserved); every other kind must keep the default. An unsplit item's
output duration is `floor(source / speed)`; a rate change keeps its source range.
Items shorter than one output tick refuse. `Item::source_time_at` (floor) and
`output_time_of` (ceiling) own the mapping used by render and edits.

`Retime.window`, when present, retains the original source processing window,
output offset and slice duration. `source_time_at` evaluates that original mapping
at offset plus local output time, retaining fractional-tick phase. Split pieces
partition output time exactly, including the original final tick, while
`source_range` records their source cut. The original window also lets audio
continue the same interpolation/WSOLA processing across cuts. Validation checks
positive contained slices, source mapping consistency and the original source
window against the asset duration. A new speed establishes a new mapping; a
pitch-only change retains it. Shortening a retained slice preserves its mapping.

`split_at` keeps both original output edges and places the new cut exactly at the
requested tick. A piece with an empty source range refuses. `trim_start` and
`trim_end` keep the opposite edge fixed; unsplit trims may displace the requested
edge inward by less than one source tick's output duration (up to 100 ticks at
0.01x), while shortening a retained slice keeps the exact output tick. Pitch
policy is data only; render applies it. Frame interpolation uses the nearest
earlier source frame; the project holds no blend policy.
