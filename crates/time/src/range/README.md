# Ranges

- `TimeRange::new`, `from_start_duration`, `contains`, `overlaps` and `intersect`;
  `SourceRange` names the same nonempty half-open range in source coordinates.

## Supported inputs and limits

Ranges are nonempty and half-open: `end > start` and the duration must fit `i64`.
Empty selections are represented by `Option::None`. Frame boundaries use ceiling
division so `frame_at(tick_of(frame))` returns `frame` even when a frame duration
is not an integer number of ticks. Operator arithmetic on `Tick` saturates; use
`checked_add`, `checked_sub` and `checked_neg` when overflow must be reported.
