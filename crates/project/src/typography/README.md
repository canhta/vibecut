# Text typography

Text items also carry a 1..256-byte font-family name, line `align` (left, centre,
right; default centre), `decoration` (none, underline, line-through), an optional RGBA
background box and x/y canvas percentages in 0..=100 (default 50/84.5 reproduces
the original lower-third box). The box has `padding_x`/`padding_y` and `radius` in 0..=128
and `offset_x`/`offset_y` in -1024..=1024 canvas pixels; it follows the text bounds, and
its `background.*` animation channels (colour, padding, offset, corner radius) override
the stored values while keyed. Weight, slant, spacing and line height are the
typography fields above; sizes are project canvas pixels, scaled with the output. Typed invalid/unsupported/limit errors refuse a whole candidate.
