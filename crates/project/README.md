# project

Headless project/timeline snapshots, invariant validation, `.vibecut` persistence,
atomic replacement and recovery snapshots. Editing commands/history belong to
engine; media owns `SourceAsset`; captions owns cue content and language tags.
No codecs, widgets, device state, scheduling, effects or keyframes live here.

## Entry points and invariants

`Project::empty` creates a 1920×1080, 30 fps, black-canvas project with 48 kHz
output audio. `Project::new(ProjectData)` validates a complete candidate;
`data()` borrows an immutable snapshot and `into_data()` yields an editable
candidate for an engine command. Revalidate before committing. Serde
construction also validates; use `decode`/`load` for untrusted JSON ingestion so
allocation and nesting budgets apply before typed deserialization.

Project, track, item and bookmark IDs are separate `u64` newtypes; media owns
asset IDs. IDs are unique within their type across a project. Zero is permitted;
engine owns allocation. Tracks are ordered bottom-to-top by consumers; items
within each track are sorted by output start, nonnegative and nonoverlapping.
Touching half-open intervals are valid. Item end uses checked arithmetic. There
is no retiming: output duration equals `source_range.duration()`. A project's
output duration is the maximum item end, excluding bookmarks.

Each item's typed `ItemParams` determines its track kind. Video/audio require an
asset with a corresponding stream and cannot exceed its known presentation
duration. Caption items retain source asset identity and captions' `CueTrack`,
including source ranges, languages, edited flags and translation references.
Cues must fit the item's source range. Text/solid-overlay items have no media
reference; their source range uses local content ticks. Missing referenced asset
IDs are invalid; missing files are valid. After loading, use
`SourceAsset::availability()` to show relink state; explicit relinking verifies
content before retaining the asset ID. Source paths are absolute native paths;
working directory changes never reinterpret them.

Names, notes and text use captions' NFC text owner. Canvas frames must have exact,
positive integer tick durations. Canvas area is at most 2^26 pixels; output audio
admits 8–192 kHz. Audio gain admits −120..24 dB; font sizes admit 1..4096 pixels.
Typed invalid/unsupported/limit errors refuse a whole candidate.

## Persistence and limits

`encode`/`decode` use one JSON envelope: `format`, `schema_version`, `generator`,
`project`. Older schemas return `TooOld`, newer schemas `TooNew`; there are no
migration or compatibility paths. Unknown current-schema project/envelope fields
are refused. `save` serializes completely before touching the destination;
`load` reads at most 16 MiB + 1 byte, including when a file grows while reading.

Ingestion and serialization cap JSON at 16 MiB, nesting at 64, individual raw
JSON strings at 1 MiB and combined comma/colon separators at 200,000, including
ignored fields. Those aggregate budgets can be reached before individual model
caps: 10,000 assets, 256 tracks, 100,000 total items/bookmarks, 100,000 cues per
caption item, and 1 MiB per stored text. Exceeding a budget returns an error;
no truncation or silent repair occurs. JSON diagnostics expose line/column only,
without user content. Raw serde use does not apply the ingestion budgets.

`atomic_write` is the shared replacement primitive for later preferences/bundles.
It stages privately in the destination directory, writes, restores the previous
file's permissions, syncs, then atomically replaces. Failed staging/publication
cleans up and preserves prior bytes. Destination symlinks are refused; parent
sync after publication is best effort. It does not preserve ownership, ACLs,
extended attributes or hard links. Hardware power-loss durability and Windows
runtime behavior remain unverified. Callers serialize writes to one target.

`autosave::save(path, project, keep)` writes into `<path>.autosave`, with increasing
20-digit snapshot numbers and no clock dependency. Retention admits 1..=1000.
Only canonical regular snapshot files participate; unrelated files, symlinks
and directories are preserved. Enumeration caps at 10,000 entries. Publication
never overwrites a snapshot and happens before pruning; pruning errors are
returned while the complete new snapshot remains. Autosave directories cannot
be symlinks. A crash can leave a temporary file, an extra complete snapshot, or
an incomplete pruning pass; no partial snapshot is promoted.

## Verification

`cargo xtask ci` runs model/invariant tables, serde and file round trips across
all item kinds and NTSC rates, hostile/deep/oversized JSON, NFC/Unicode paths,
missing-media state, hash-verified relink, save failure/cleanup, permissions,
symlinks and autosave retention. Property tests generate adjacent integer-tick
items and rational common rates and check exact persistence without drift.
A unit test injects failure after a partial staging write to verify original
bytes and cleanup. This establishes application-level behavior on the tested
filesystem, not survival of physical power loss. No performance claim is made.
