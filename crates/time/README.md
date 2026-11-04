# time

Exact integer media time for the editor core: ticks, rational frame rates and
nonempty source ranges.

## Purpose and exclusions

Owns `Tick` at 254 016 000 000 ticks per second, `FrameRate`, `TimeRange` and the
`SourceRange` alias, plus conversions between ticks, floating seconds, audio
samples, rational container timebases and frame indices.

Excludes media probing, variable-frame-rate policy, drop-frame timecode display,
project persistence and every platform dependency.

## Public entry points

- `TICKS_PER_SECOND`, `Tick`, `Tick::from_seconds_f64`, `Tick::from_units`,
  `Tick::from_rational` and their inverse conversions.
- `FrameRate` constants (`FPS_23_976` through `FPS_120`), `FrameRate::new`,
  `frame_at`, `tick_of`, `snap` and `snap_nearest`.
- `TimeRange::new`, `from_start_duration`, `contains`, `overlaps` and `intersect`;
  `SourceRange` names the same nonempty half-open range in source coordinates.
- `Rounding` selects floor, ceiling or nearest (ties away from zero).
- `TimeError` reports non-finite seconds, invalid rates, empty ranges, overflow and
  out-of-range results. No public operation panics on hostile input.

## Supported inputs and limits

Admitted frame rates are positive rationals with `i64` parts; the standard
broadcast and NTSC rates divide the tick rate exactly. Audio sample rates are
positive `i64` values; 44 100, 48 000 and 96 000 Hz are exact. Floating seconds are
accepted only at display/provider boundaries and must be finite.

Ranges are nonempty and half-open: `end > start` and the duration must fit `i64`.
Empty selections are represented by `Option::None`. Frame boundaries use ceiling
division so `frame_at(tick_of(frame))` returns `frame` even when a frame duration
is not an integer number of ticks. Operator arithmetic on `Tick` saturates; use
`checked_add`, `checked_sub` and `checked_neg` when overflow must be reported.

## Tests

`cargo test -p time` runs deterministic round-trip samples over ten hours for
23.976, 24, 25, 29.97, 30, 50, 59.94, 60 and 120 fps, exact sample conversions,
rounding ties, range boundaries, hostile rates and overflow paths, plus property
tests for random frames, extreme rational rates and rounding bounds. There is no
external oracle: expected values derive from the exact rational arithmetic.
