# VibeCut

Language for the editor's privacy and execution boundaries.

## Language

**Local-first**: Editing and project data stay on the user's computer by default;
remote AI processing requires explicit consent.

**Headless core**: The editing capabilities that run without a graphical interface.

**AI provider**: A service or local model offering AI processing through an adapter.

**Explicit consent**: The user's affirmative approval to send specified media or
transcripts to a remote provider.

**Tick**: An integer time unit at 254_016_000_000 ticks per second, exact for
admitted rational frame rates and audio sample rates.

**FrameRate**: An exact rational frames-per-second rate, stored as a positive
numerator and denominator rather than a floating value.

**SourceRange**: A nonempty half-open `[start, end)` range in source-media ticks.
