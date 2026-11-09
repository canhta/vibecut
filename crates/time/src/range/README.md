# Ranges

`SourceRange` names the same nonempty half-open range as `TimeRange` in source coordinates.

Ranges are nonempty and half-open: `end > start` and the duration fits `i64`. Empty selections are `Option::None`. Frame boundaries use ceiling division so `frame_at(tick_of(frame))` returns `frame` even when a frame duration is not an integer number of ticks. Operator arithmetic on `Tick` saturates; use `checked_add`, `checked_sub` and `checked_neg` when overflow must be reported.
