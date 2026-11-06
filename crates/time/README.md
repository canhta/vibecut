# time

Exact integer media time for the editor core: ticks, rational frame rates and
nonempty source ranges.

## Purpose and exclusions

Owns `Tick` at 254 016 000 000 ticks per second, `FrameRate`, `TimeRange` and the
`SourceRange` alias, plus conversions between ticks, floating seconds, audio
samples, rational container timebases, frame indices and DF/NDF editor timecode.

Excludes media probing, variable-frame-rate policy, binary LTC/VITC encoding,
project persistence and every platform dependency.

## Modules

| Module | Responsibility |
|---|---|
| `tick` | Counts media time in exact integer ticks. |
| `rate` | Compares rational frame rates and converts them to and from ticks. |
| `range` | Holds nonempty half-open ranges in output and source ticks. |
| `rounding` | Applies the explicit rational rounding policy. |
| `timecode` | Formats and parses signed DF/NDF editor labels. |
| `speed` | Maps constant playback speed to exact source and output durations. |

The crate root declares these modules, owns `TimeError`, and re-exports the same
public names at unchanged crate-root paths.

## Public entry points

- `TICKS_PER_SECOND`, `Tick`, `Tick::from_seconds_f64`, `Tick::from_units`,
  `Tick::from_rational` and their inverse conversions.
- `FrameRate` constants (`FPS_23_976` through `FPS_120`), `FrameRate::new`,
  `frame_at`, `tick_of`, `snap` and `snap_nearest`.
- `TimeRange::new`, `from_start_duration`, `contains`, `overlaps` and `intersect`;
  `SourceRange` names the same nonempty half-open range in source coordinates.
- `format_timecode` and `parse_timecode` convert between ticks and absolute signed
  frame labels, with explicit DF/NDF mode and typed invalid-label errors.
- `Tick::checked_scale(num, den, rounding)` scales by a positive rational with
  checked i128 intermediate arithmetic and the same explicit rounding policy.
- `Rounding` selects floor, ceiling or nearest (ties away from zero).
- `Speed` is a constant playback speed in integer thousandths, 0.01x-5x (JSON: the
  decimal, rounded once to the nearest 0.001). `output_duration` is
  `floor(source / speed)`; `source_offset` and `output_offset` convert progress
  with an explicit `Rounding`; `source_within` gives the source span that plays for
  an output duration without exceeding it (exactly `ceil(output * speed)` from 1x
  up). A speed is an exact reduced ratio, so no duration is accumulated in floating
  point. Out-of-range or non-finite speeds are `TimeError::InvalidSpeed`.
- `TimeError` reports non-finite seconds, invalid rates, empty ranges, overflow and
  out-of-range results. No public operation panics on hostile input.

## Supported inputs and limits

Admitted frame rates are positive rationals with `i64` parts; ordering uses exact
`i128` cross products, including at the maximum admitted parts; the standard
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

## Timecode

Labels use `[-]HH:MM:SS:FF` for NDF and `[-]HH:MM:SS;FF` for DF. Supported
rates are 23.976, 24, 25, 29.97, 30, 48, 50, 59.94 and 60 fps; other rates
return `Unsupported`. DF is accepted only at 29.97 and 59.94, skipping labels
00–01 or 00–03 respectively at each minute except every tenth minute. At 59.94,
FF identifies individual frames (00–59), rather than binary timecode frame pairs.

Formatting floors to the containing signed frame; parsing returns its start tick.
For example, a tick just before zero formats as frame -1. Hours extend beyond 23,
with no day wrap; parsing requires at least two hour digits, two digits per other
field, exact separators and no whitespace or leading plus. Skipped DF labels and
invalid fields are refused. Negative zero normalizes to zero. A frame boundary
outside the tick range returns `OutOfRange`, including the frame containing the
minimum tick when its boundary is earlier than that tick.

Timecode tests cover rollover tables, signed round trips, hostile inputs, tick
limits and an independent successive-label counter spanning ten minutes at both
DF rates. These editor labels do not represent wire-level LTC/VITC addresses.
