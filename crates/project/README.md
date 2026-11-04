# project

Headless project/timeline snapshots, invariant validation, `.vibecut` persistence,
atomic replacement and recovery snapshots. Editing commands/history belong to
engine; media owns `SourceAsset`; captions owns cue content and language tags.
Project also owns item-relative parameter/keyframe data, validation, evaluation and
time propagation. No codecs, widgets, device state, scheduling or effect execution.

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

## Animated parameters

`Item.animations` is the single `Animations` map for parameter overrides. A missing
path leaves the consumer's existing static property in use. A present `Param`
owns its explicit base value and keys, including when no keys remain; editing
static item content does not rewrite this independent override. `Param::new(value)`
creates a static override. Validate edited candidate data through `Project::new`.
`Animations::value_at(path, local_tick)` is the shared evaluation entry point for
engine, rendering, audio automation and graph consumers. None means absent,
not zero. No render/CLI execution is introduced by this data contract.

`AnimationPath::new` admits `transform.positionX/Y`, `transform.scaleX/Y`,
`transform.rotate`, `opacity`, `volume`, `color`, `background.color`,
`background.paddingX/Y`, `background.offsetX/Y`, `background.cornerRadius`,
`params.<name>` and `effects.<id>.params.<name>`. Extension ids/names are 1..=64 ASCII
letters/digits/underscore/hyphen; complete paths are at most 160 bytes. These
extension namespaces reserve animation data, not an effect/graphic implementation
or catalog entry. Future owners resolve ids against their admitted implementations.
Built-in numeric paths require scalars; color paths accept linear RGBA composites
or discrete text values. Extension paths admit all parameter kinds.

`ParamValue` holds scalar, bool, NFC text or linear-light unpremultiplied RGBA.
RGBA uses four independently keyed scalar `Channel`s, selected by `ColorComponent`;
there is no shared-key-time restriction. Stored RGBA values are in 0..=1 and
composite evaluation clamps overshoot/extrapolation to that range. Input/output is
already linear RGBA; existing byte-color fields are not silently reinterpreted or
converted. Discrete color text stays a held string. `AnimationPath::scale_group()`
identifies the scaleX/Y editor group; both scalar curves remain independent.

`Channel::new(base, keys, before, after)` validates strictly increasing unique
key times and homogeneous values. Each scalar key's `Segment` governs the segment
leaving it: Step, Linear or Bezier. Exact endpoints return their stored values.
Bezier handles contain relative signed `dt` ticks and `dv` in scalar value units;
incoming dt <= 0, outgoing dt >= 0. At evaluation dt is clamped to the adjacent
span. Missing handles use (span/3, delta/3) secants. The cubic time curve is inverted
before evaluating its absolute value curve, including nonzero handles between
equal endpoint values. Scalar extrapolation before/after is independently Hold
or the edge/neighbor Linear secant; a single key holds.

`TangentMode::{Auto, Aligned, Broken, Flat}` are persisted source labels only.
They do not calculate, flatten or mirror handles and never change evaluation.
This is an explicit source gap: no UI may advertise them as easing choices until
a separate ticket defines their mathematics. Explicit dt/dv handles are authoritative.
Bool/text keys require Step and Hold extrapolation; before their first key the
explicit base applies, then each key holds until the next. A scalar channel before
its first key follows its configured edge extrapolation.

Key times are signed i64 ticks relative to item start. Negative and out-of-window
keys are retained: trims/splits translate anchors rather than discarding or
resampling curves. `Animations::shift` checks overflow, preserving exact tick
differences and handle offsets; engine's range changes call this once. Trim-end
and item moves retain local times. `Animations::retime(num, den)` stretches key and
handle times by a positive rational through `Tick::checked_scale` with nearest-tick,
ties-away-from-zero rounding. It refuses collisions/overflow atomically. Integer
stretches of frame-aligned 29.97-fps keys remain exact. General rational stretches
may round each timestamp by at most half a tick; they do not promise an invertible
round trip. Media playback rate/output duration remain the retime consumer's job.
Moving one key leaves explicit handles unchanged and clamps them when evaluated.

Limits: 64 paths per item, 4096 keys per channel, 100,000 keys per project; JSON
budgets below also apply. Stored scalars/handle dv must be finite with absolute
value <= 10^12. Unknown fields/paths, wrong component kinds, unordered/duplicate
keys, invalid handles and over-budget input refuse. Raw channel deserialization
bounds its key sequence; map deserialization refuses duplicate paths. Candidate
mutations remain subject to whole-project validation. Numeric evaluation uses
allocation-free scalar/rgba arithmetic and binary channel lookup; no throughput
or renderer output claim is made.

## Persistence and limits

`encode`/`decode` use one JSON envelope: `format`, `schema_version`, `generator`,
`project`. Current schema 4 stores ProjectId as a canonical decimal string, preserving
all u64 identity bits for control clients; other IDs remain numeric. Schemas 1–3
are refused rather than migrated or silently defaulted. Older schemas return `TooOld`, newer schemas `TooNew`; there are no
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
editing and saving. Dropping the lease explicitly unlocks before closing its
handle, so descriptors inherited during concurrent subprocess creation cannot
extend its lock lifetime. `WriterLease::load` refuses a changed baseline; use standalone
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
then replaces the held lease only on success. `save_to(&self, &mut destination,
project)` publishes to an already acquired destination while retaining the source;
engine uses it to defer lease adoption until revision/receipt acknowledgement. Both
paths refuse an invalidated source lease. Busy/failure preserves source path
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

## Streaming output

`atomic::StreamingStage::new(destination, overwrite, max_bytes)` creates a private
same-directory `.vibecut-export-*` temporary file. `writer()` yields the sole
bounded seekable `StreamingWriter`, including media's mux-sink interface. Writes
and seeks reject positions beyond the authorized maximum before touching bytes.
`finalize()` syncs and restores prior permissions, yielding `FinalizedStage`;
only that type exposes `publish()`. `Overwrite::Refuse` uses atomic no-clobber
publication, including a competing destination created after admission.
`Replace` is explicit authorization to replace the target at publication.

Dropping or failing a stage removes its temporary file and leaves existing bytes
untouched. `AtomicFs` injects sync/publication failure for boundary tests. A crash
may leave an orphan temporary file; it is never promoted, reused or automatically
swept. The caller may present cleanup after restart. Destination symlinks and
nonregular files are refused. Callers serialize writes and close their mux writer
before finalizing; cancellation arbitration belongs to their execution owner.
Parent-directory sync remains best effort after commit: there is no fallible
post-publication result. Hardware power loss, racing parent replacement and
Windows runtime behavior remain unverified.

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

## Optional project metadata

`WriterLease::metadata` reads `<canonical project filename>.meta.json`, bounded to
4 KiB, with metadata schema version 1. Missing, unreadable, corrupt, oversized or
unsupported metadata defaults without blocking the project. `Metadata` contains
created/modified Unix milliseconds and `ViewState` (positive zoom at most 1000, horizontal
scroll 0–1e9 logical pixels, nonnegative integer-tick playhead). Nonfinite values
are refused. This sidecar is outside the editable project envelope and does not
change its schema. No project index or thumbnail bytes are stored here.

`WriterLease::save_metadata` validates then atomically replaces the sidecar under
the existing canonical project lease and content precondition. Sidecar symlinks
are refused. Project writer destinations reserve the `.meta.json` suffix, including
case variants and canonical aliases, so saving a project cannot replace another
project's metadata. A sidecar write error is returned; it does not roll back a
previously successful project publication. Callers acknowledge a save only after
both writes succeed. `tests/metadata.rs` covers bounds, fallback, native paths,
symlink refusal, external project replacement and round-trip independence.

## Item effects

`effect_definitions()` exposes the admitted effect names and scalar parameter bounds,
defaults and animation capability for UI consumers. `Item.effects` is an ordered stack of at most 64 visual `EffectInstance`s. Instance
IDs are item-local ASCII animation identifiers (1–64 bytes); duplicates refuse.
`EffectParams::Blur` stores finite intensity 0–100 (default 15); `enabled` bypasses
an instance. Audio items refuse visual effects. Evaluation/device work belongs to
render/the desktop app. Animated overrides use the existing `Animations` map and
`Param` storage at `effects.<instance-id>.params.intensity`, with scalar channels
only. No second keyframe store is introduced. Removing an effect removes its
animation paths; undo restores both. Schema 4 adds the required stack and refuses
older envelopes through the existing typed version errors.
