# Writer lease and save acknowledgement

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
