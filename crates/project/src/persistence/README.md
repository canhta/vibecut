# Persistence and publication

Versioned project JSON, the writer lease, atomic files, recovery snapshots,
optional metadata and bounded streaming output.

| Module | Responsibility | Contract |
|---|---|---|
| `persistence` | Encodes, decodes and bounds project JSON. | [Envelope and limits](#envelope-and-limits) |
| `writer` | Holds the writer lease and save receipts. | [Writer lease and save acknowledgement](#writer-lease-and-save-acknowledgement) |
| `atomic` | Publishes files atomically and admits destinations. | [Atomic files](#atomic-files) |
| `autosave` | Writes rotating recovery snapshots. | [Autosave snapshots](#autosave-snapshots) |
| `metadata` | Owns the optional project metadata sidecar. | [Optional project metadata](#optional-project-metadata) |
| `streaming` | Streams bounded output into a staged file. | [Streaming output](#streaming-output) |

## Envelope and limits

`encode`/`decode` use one JSON envelope: `format`, `schema_version`, `generator`,
`project`. Current schema 21 adds the item `hidden` flag; schema 20 adds `canvas.preset` and `canvas.last_custom_size`; schema 19 splits the text background `padding` into `padding_x`/`padding_y` and adds its `offset_x`/`offset_y` plus text `align` and `decoration`; schema 18 added item `retime` (speed, pitch policy and the retained processing window of split pieces) and ordered transition split pieces; schema 17 added `settings.loudness` (clip/master targets and the last measured gains) to schema 16 timeline `transitions`, which followed schema 15 audio/video clip `fades`, schema 14 optional imported cue style, schema 13 text weight, italic, letter spacing and line height (also on text masks), and schema 12 audio `source` (upload or library provenance), volume/mute and master controls.
ProjectId is stored as a canonical decimal string, preserving
all u64 identity bits for control clients; other IDs remain numeric. Schemas 1-20
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

`Project::fork` copies editable content under a fresh random ProjectId, refusing
entropy failure without changing the source. Identity generation uses the existing
MIT/Apache-2.0 `getrandom` 0.4.3 system-entropy dependency. The persisted format
and schema stay unchanged.

`save_as` acquires the destination while retaining the source lease, publishes
a fork, then replaces the held lease only on success. Its receipt exposes the
published `project_id`; the supplied snapshot and source file stay untouched.
Saving to the held canonical path (including aliases) retains the identity.
`save_to(&self, &mut destination,
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
Local-filesystem writer behavior is exercised by macOS and Windows tests;
remote filesystem locking remains unverified.

Managed-directory moves opt in with `WriterLease::enable_directory_moves(root)`
before projects are published. This creates the permanent private control file
`<root>/.vibecut-directory-moves.lock`, outside every movable child. All ordinary
writer acquisitions beneath a child take shared admission on that file through
adjacent-lock acquisition, then release admission; independent open projects do
not block each other. Acquisitions begun before barrier installation recheck
admission after locking the sidecar, before reading or exposing a writer.
`WriterLease::move_directory(destination)` consumes a held
project lease, takes exclusive admission, validates source/content and an absent
sibling destination, closes the adjacent handle, and renames while still holding
exclusive admission. New writers at either path receive Busy throughout that
interval; existing writers remain excluded by the adjacent project lease. This
preserves Windows sidecar identity protection without delete sharing or an
unprotected unlock-before-rename interval. Admission files share the reserved
writer-control namespace and must never be deleted/replaced. External-file
projects without this opt-in retain the ordinary adjacent-lock protocol.

A refused move leaves its source and the caller's history unchanged. A process
interruption before/after the atomic sibling rename leaves the complete directory
at its source/destination; OS locks release on process death. The library owns
hidden-directory retention/recovery and permanent deletion. Physical power-loss
durability and noncooperating directory/control-file replacement races remain
outside this advisory protocol. Windows library integration and deterministic
close/rename interruption tests exercise this protocol; macOS review and remote
filesystem behavior require separate receipts.

## Atomic files

`atomic_write` is the shared replacement primitive for later preferences/bundles.
It stages privately in the destination directory, writes, restores the previous
file's permissions, syncs, then atomically replaces. Failed staging/publication
cleans up and preserves prior bytes. Destination symlinks are refused; parent
sync after publication is best effort. It does not preserve ownership, ACLs,
extended attributes or hard links. Hardware power-loss durability and Windows
runtime behavior remain unverified. Callers serialize writes to one target.

`admit_destination` exposes the reserved writer-control/native filename admission
to background publication consumers before they stage data. Source and
active-project identity checks remain with the execution owner. `metadata_path`
supplies that owner with the native sidecar name.

## Autosave snapshots

`autosave::save(path, project, keep)` writes into `<path>.autosave`, with increasing
20-digit snapshot numbers and no clock dependency. Retention admits 1..=1000.
Only canonical regular snapshot files participate; unrelated files, symlinks
and directories are preserved. Enumeration caps at 10,000 entries. Publication
never overwrites a snapshot and happens before pruning; pruning errors are
returned while the complete new snapshot remains. Autosave directories cannot
be symlinks. A crash can leave a temporary file, an extra complete snapshot, or
an incomplete pruning pass; no partial snapshot is promoted.

## Optional project metadata

`WriterLease::metadata` reads `<canonical project filename>.meta.json`, bounded to
512 KiB, with metadata schema version 3. Missing, unreadable, corrupt, oversized or
unsupported metadata defaults without blocking the project. `Metadata` contains
created/modified Unix milliseconds, the current `ViewState` and up to 256 scene-ID-keyed views (positive zoom at most 1000, horizontal
scroll 0–1e9 logical pixels, nonnegative integer-tick playhead). Nonfinite values
are refused. This sidecar is outside the editable project envelope and does not
change its schema. Thumbnail is Missing, Unavailable or Ready with 1..=128-pixel dimensions and
exactly width × height × 4 sRGB RGBA bytes (at most 64 KiB). Invalid dimensions,
byte counts and oversized sidecars default on read and refuse on write. No image
decoder runs on metadata. The 8-character display ID is derived from the project
ID by engine, never duplicated here. No project index is stored here.
`read_metadata(path)` supports bounded read-only catalog display; mutations remain
under WriterLease. `WriterLease::validate` checks its baseline and stable lock
identity without requiring a project file to exist.

`WriterLease::save_metadata` validates then atomically replaces the sidecar under
the existing canonical project lease and content precondition. Sidecar symlinks
are refused. Project writer destinations reserve the `.meta.json` suffix, including
case variants and canonical aliases, so saving a project cannot replace another
project's metadata. A sidecar write error is returned; it does not roll back a
previously successful project publication. Callers acknowledge a save only after
both writes succeed. `tests/it/metadata.rs` covers bounds, fallback, native paths,
symlink refusal, external project replacement and round-trip independence.

## Streaming output

`StreamingStage::new(destination, overwrite, max_bytes)` creates a private
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
