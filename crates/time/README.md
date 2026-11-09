# time

Exact integer media time for the editor core: ticks, rational frame rates and nonempty source ranges.

Owns `Tick` at 254 016 000 000 ticks per second, `FrameRate`, `TimeRange` and the `SourceRange` alias, plus conversions between ticks, floating seconds, audio samples, rational container timebases, frame indices and DF/NDF editor timecode. Media probing, variable-frame-rate policy, binary LTC/VITC encoding, project persistence and platform dependencies are excluded.

| Module | Responsibility | Contract |
|---|---|---|
| `tick` | Counts media time in exact integer ticks. | [Ticks](src/tick/README.md) |
| `rate` | Compares rational frame rates and converts them to and from ticks. | [Frame rates](src/rate/README.md) |
| `range` | Holds nonempty half-open ranges in output and source ticks. | [Ranges](src/range/README.md) |
| `rounding` | Applies the explicit rational rounding policy. | [Rounding](src/rounding/README.md) |
| `timecode` | Formats and parses signed DF/NDF editor labels. | [Timecode](src/timecode/README.md) |
| `speed` | Maps constant playback speed to exact source and output durations. | [Speed](src/speed/README.md) |

The crate root owns `TimeError` and re-exports the same public names. `TimeError` reports non-finite seconds, invalid rates, empty ranges, overflow and out-of-range results. No public operation panics on hostile input. Tests have no external oracle; expected values derive from exact rational arithmetic.
