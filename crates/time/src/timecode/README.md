# Timecode

- `format_timecode` and `parse_timecode` convert between ticks and absolute signed
  frame labels, with explicit DF/NDF mode and typed invalid-label errors.
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
