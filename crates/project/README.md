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
are refused. `WriterLease::save` serializes completely before touching the destination;
read-only `load` reads at most 16 MiB + 1 byte, including when a file grows while reading.

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

## Writer lease and save acknowledgement

`WriterLease::acquire(path)` takes an exclusive OS lock on the stable adjacent
`.FILENAME.lock` before reading project bytes. Retain the lease from load through
editing and saving. `WriterLease::load` refuses a changed baseline; use standalone
`load` only for read-only ingestion. `acquire_wait(path, duration, cancelled)` polls
contention and returns `Busy` on timeout/cancellation. Native filesystem calls are
not interruptible, so cancellation is checked between them. Unsupported lock paths,
permissions or OS locking return typed errors and never authorize an unlocked save.

Project data and permanent writer control files have disjoint namespaces. All
atomic publication and writer acquire/save-as paths refuse a basename beginning
with `.` and ending with `.lock` (ASCII case-insensitive), including reusable or
not-yet-created sidecars. Admission checks both supplied and canonical names, so
symlinks and directory/case aliases cannot publish project bytes over a lease.
Windows also refuses trailing spaces/dots and alternate-stream names before
creation, preventing different spellings from acquiring separate sidecars.
Existing project files with reserved names remain readable; save them to an
admitted destination rather than rewriting the control namespace.

A lease retains its original lock object and verifies the current sidecar identity
at acquisition and before/after bounded reads. Unix compares device/inode with
metadata from the held descriptor. Windows opens without delete sharing and uses
full 128-bit file IDs; unavailable identity support refuses the lease instead of
falling back to ambiguous 64-bit IDs. A missing, replaced, symlinked or differently
identified sidecar returns `Conflict`, including when both objects are regular
files with identical bytes. Invalidated leases cannot load, save or save-as.

`save(&mut self, project)` compares the current bounded file's full BLAKE3 content
with the acquired/last-saved baseline, then stages through the existing atomic
writer. After staging/sync it rechecks both content and lease identity immediately before publication; a refused final
precondition cleans staging and returns no receipt.
A changed, removed or unexpectedly created file returns `Conflict`; failed saves
retain the baseline and issue no receipt. A successful `SaveReceipt` contains the
canonical native path and hash of the exact envelope bytes published. Engine binds
its own saved revision and still-current path to that receipt before acknowledging
clean state; revision tracking/journal scheduling remain engine-owned. Schema and
JSON envelope are unchanged; leases, hashes and receipts are not persisted in it.

`save_as` acquires the destination while retaining the source lease, publishes,
then replaces the held lease only on success. Busy/failure preserves source path
and both files. Existing symlinks resolve to the target, including symlinked parent
directories; relative paths resolve once. Case/Unicode aliases follow the native
filesystem's name rules, including for adjacent lock files. Native `OsStr` names
are retained without lossy conversion. Filesystem-rejected names return typed
errors; the lock name adds six native characters, so near-limit filenames can be
refused rather than truncated into colliding locks.

The lock file is private on Unix, is never truncated or removed, and contains no
project data. Dropping the lease or killing its process releases the OS lock; the
empty file remains safe to reuse. The shared atomic publisher enforces the control
namespace too. Locks are advisory: content checks detect external edits present
before publication, but do not provide atomic compare-and-swap against another
application racing the final check and rename. Lock-file/directory replacement
observed at a validation boundary invalidates the lease. Hard-link preservation
remains outside the atomic writer contract.
macOS local-filesystem behavior is observed; Windows and remote filesystem locking
remain unverified.

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
symlinks and autosave retention. Public writer tests cover concurrent threads and
real processes, abrupt process death, stale-content refusal, save-as acquisition,
wait/cancel, receipts, relative/symlink/case aliases and native path refusal.
Namespace regressions retain the public Save As lost-edit reproduction, all ASCII
suffix case variants, aliased publication targets, same-byte sidecar replacement,
parent replacement, stale save-as refusal and hard-link alias noninterference.
Property tests generate adjacent integer-tick items and rational common rates and check exact persistence without drift.
A unit test injects failure after a partial staging write to verify original
bytes and cleanup. This establishes application-level behavior on the tested
filesystem, not survival of physical power loss. No performance claim is made.
