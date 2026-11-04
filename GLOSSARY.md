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

**SourceAsset**: A source-media identity and its location, content fingerprint and
probed stream information; timeline items refer to it without copying its media.

**Transcript**: Source-language recognition result with stable segments and a
provider/model/settings fingerprint, owned by `captions`.

**Cue**: Identified source range with source language and per-language text,
independent edit flags and translation references.

**EditMapping**: The correspondence between selected source ranges and their
positions in an output timeline; one source span can appear more than once.

**LanguageProfile**: Segmentation, reading-speed, punctuation and font-fallback
rules for a content language, with a generic fallback for unknown tags.

**Job**: A tracked editing operation with progress, cancellation, failure and
checkpoint information.

**Session**: The active project together with its command execution and undo/redo
history.

**Segment**: Stable identified span of transcript text in source-media time.

**Word**: Recognized source text with optional source range, speaker and confidence.

**Track**: A `CueTrack` grouping cues for validation, repair and subtitle export.
