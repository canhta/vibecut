# Speed

- `Speed` is a constant playback speed in integer thousandths, 0.01x-5x (JSON: the
  decimal, rounded once to the nearest 0.001). `output_duration` is
  `floor(source / speed)`; `source_offset` and `output_offset` convert progress
  with an explicit `Rounding`; `source_within` gives the source span that plays for
  an output duration without exceeding it (exactly `ceil(output * speed)` from 1x
  up). A speed is an exact reduced ratio, so no duration is accumulated in floating
  point. Out-of-range or non-finite speeds are `TimeError::InvalidSpeed`.
