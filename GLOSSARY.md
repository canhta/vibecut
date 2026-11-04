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
provider/model/settings fingerprint.

**Cue**: Identified source range with source language and per-language text,
independent edit flags and translation references.

**EditMapping**: The correspondence between selected source ranges and their
positions in an output timeline; one source span can appear more than once.

**LanguageProfile**: Segmentation, reading-speed, punctuation and font-fallback
rules for a content language, with a generic fallback for unknown tags.

**Subtitle workflow**: The engine composition from source speech through explicit
provider choice, checkpointed recognition, optional alignment/translation, cue
segmentation and subtitle export/burn.

**ASR checkpoint**: One independently complete recognition window stored under its
exact source content, sample range, language, provider/model/settings and aligner
identity; a restarted workflow reuses valid checkpoints and recomputes missing ones.

**Provider catalog**: Offline capability data (operation, models, languages, local
or remote, native timing) used to list and filter explicit provider choices without
constructing an adapter or making a network call.

**Upload disclosure**: The exact remote request scope shown before consent:
provider/model, operation, languages, audio duration, payload bytes, per-invocation
cost ceiling and the provider's official policy links.

**Job**: A tracked editing operation with progress, cancellation, failure and
checkpoint information.

**Session**: The active project together with its command execution and undo/redo
history.

**Segment**: Stable identified span of transcript text in source-media time.

**Timing unit**: Unit of recognized speech timing: segment, word, syllable,
character, grapheme or token. A token can be part of a character or word.

**Display variant**: Same-language spelling or script rendering of recognized
text, retaining its source speech timing.

**Word**: Recognized source text with optional source range, speaker and confidence.

**CueTrack**: A collection of timed cues for subtitle validation and export.
