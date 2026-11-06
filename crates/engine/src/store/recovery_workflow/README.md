# Session recovery and save acknowledgement

## Session recovery and save acknowledgement

`recovery::Autosave` composes one Session with persistence. Supply a private root
under the app data directory, the aggregate ByteBudget, and a monotonic jobs Clock.
It uses the existing JobRunner for off-thread encoding/IO, with one in-flight job
and one latest pending snapshot. `session_mut()` dispatches commands/gestures;
`switch()` flushes before replacing it with never-saved work; to replace it with a
saved file, flush first and construct `Autosave::open`. Each observed committed snapshot
advances a checked session-local revision; gesture previews are never journaled.

Call `tick()` from the app event loop. `Schedule::default()` uses 800 ms debounce
and 10 s maximum dirty age, measured from the first unpublished edit, including
continuous editing. These are scheduling deadlines: event-loop pump latency,
budget pressure and filesystem latency add to completion time. A write reserves
40 MiB from the shared budget before encoding bounded project/metadata bytes;
budget starvation can delay admission. There is no UI toolkit dependency.

One atomically created directory and exclusive std OS file lock identify each
session. Live sessions (including other windows in one process) are never offered
by `discover()`. After process death the OS releases the lock; discovery holds it
until the returned Candidate is dropped/discarded, preventing two instances from
claiming the same abandoned session. Unsupported locking is a typed error, never
assumed stale. Dropping an Autosave (after worker shutdown) or Candidate explicitly
unlocks its session, even while a concurrently spawned child retains an inherited
descriptor before exec. Scanning is bounded to 10,000 entries; corrupt candidates yield
retained typed diagnostics, without deleting their files. Names/paths have no
Debug representation and are never logged.

The journal descriptor has its own version, project id/name, original saved path,
monotonic revision, BLAKE3 digest, timestamp and canonical snapshot filename.
Project content uses only project::encode/decode and project::autosave::save,
with two retained snapshots. The descriptor is committed through the shared atomic
writer after its snapshot. Any write failure is sticky and prevents further
rotation; an interrupted/failed descriptor update leaves the previous confirmed
pair readable. The last published revision survives forced process termination;
physical power loss and Windows runtime behavior remain unverified.

`Autosave::new(root, session, budget, clock, schedule)` accepts never-saved or
recovered work. `Autosave::open(root, path, history_limit, budget, clock, schedule)`
acquires the project-owned WriterLease before loading or exposing an editable
Session. It retains that same lease through editing and background saves; opening
is blocking IO and belongs off the UI thread. Busy/unsupported acquisition exposes
no editable session. Read-only project ingestion remains a separate project API.

`Candidate::recover()` returns never-saved work for `Autosave::new`; `save()` then
returns SaveAsRequired. `save_as(path)` acquires the destination before queuing
journal publication and project saving on the worker. Ordinary save and aliases
reuse the before-load lease. A Save As retains both source and destination leases
until acknowledgement. Project owns all locking and publication; engine transfers
those capabilities to/from its existing worker without holding a state mutex
across IO. Worker error/unwind retains the capabilities until the failed Autosave
is dropped; Drop joins/cancels workers before releasing their leases.

`tick()` acknowledges only the captured Session target and its project SaveReceipt.
`saved_receipt()` exposes the last acknowledged canonical path and exact published
content hash; it is absent after open and invalidated by a stale completion.
`saving()` includes a request queued behind an earlier journal/cleanup. A stale completion returns
StaleResult and invalidates the clean baseline: the worker has already written
its snapshot, so even undo to the previously saved Arc must remain dirty and
recoverable. This conservative invalidation also covers aliased save destinations;
a fresh acknowledged save restores the clean baseline. The existing saved path is
retained with its source lease, while a stale save-as releases the destination
lease and never adopts its path or receipt. Acknowledged Save As transfers path,
receipt, fresh ProjectId and destination lease together. The worker journals the
original snapshot before attempting publication. After acknowledgement, `saving()`
stays true through the worker's journal retirement, so a completed Save As leaves
no recovery entry under the original identity. Later dirty snapshots use the new
identity/path. A failed publication retains the original identity, path and lease.
The same Session keeps its undo/redo history; restoring a historical snapshot
retains the current ProjectId, so undo cannot return to the original project.
Subsequent recovery continues. Successful
save-as establishes the destination for later save(). Paths stored in journals and
save receipts are canonical native paths. Save As requires an absolute native
path; saves inside recovery storage are refused. Native path resolution and lease
acquisition can block the caller; serialization/publication run on the worker.
Non-UTF-8 descriptor paths return a typed unsupported encoding error without
changing the original file.

`flush()` is an explicit blocking barrier for project-switch/exit; IO still runs
on the worker. Use `switch()` to flush/acknowledge the pending revision before
replacing the project; failure retains the old Session. Active gestures/groups
refuse these boundaries. Drop joins/cancels existing work and does **not** promise
a flush. Clean saves/undo-to-saved retire obsolete journals on the worker.
Candidate::discard removes only its descriptor/canonical snapshots, preserving
original projects and unrelated files. Recovery is never silently discarded.
Empty session directories/lock files are retained; applications should archive or
remove empty private session directories during maintenance before the scan cap.

Observed tests cover abrupt subprocess termination, two live processes, exclusive
candidate claims, stale OS locks, continuous-edit/debounce deadlines, corrupted or
truncated descriptors/snapshots, digest/path/schema refusal, original-byte
preservation, save-as promotion, stale save completion, switch barriers, undo to
saved state, injected StorageFull and worker unwind, and refusal to rotate after
failure. These are synthetic lifecycle checks, not a storage throughput benchmark.
The debounce/deadline test advances an injected monotonic clock. A Unix-only
retained-descriptor case deterministically checks session/candidate release and
continued exclusion while a replacement candidate is held, without timing margins.
The app owns recovery presentation and user choices; project writer leases remain
project-owned and are composed here without a second lock protocol. Additional
integration tests cover before-load Busy refusal, retained leases across edits and
receipt acknowledgement, external edit conflict before first save, busy/stale and
acknowledged Save As, aliases, queued cancellation and worker transfer unwind.
