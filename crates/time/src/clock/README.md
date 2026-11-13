# Clock

`format_clock`, `format_short_clock` and `format_duration` render a tick as fixed display text. Negative ticks read as zero, and every formatter floors from exact integer ticks, never floating seconds.

| Function | Output | Use |
|---|---|---|
| `format_clock` | `HH:MM:SS.mmm`, milliseconds floored, hours at least two digits | Asset media info and exact fallbacks |
| `format_short_clock` | `M:SS` with total minutes, so one hour is `60:00` | Asset grid badges |
| `format_duration` | `M:SS` below one hour, `H:MM:SS` from one hour | Project cards and info dialogs |

These are display text only. Editor frame labels are [timecode](../timecode/README.md). Locale and digit glue belong to the app.
