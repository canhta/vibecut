# engine

Headless command execution, Session ownership, project-only snapshot history,
bounded background jobs and content-addressed artifact storage. Processing
algorithms and decoded/raster caches remain with their domain owners. No widgets,
device state, app preferences or platform API is required.

## Public entry points

`Session::new(project, history_limit)` admits 1..=1000 retained undo steps.
`project()` exposes an immutable `Arc<Project>` for downstream consumers.
`apply(Command)` and `execute(command_id, serde_json::Value)` run the same edits.
`apply_merged(command, Some(key))` combines consecutive successful changes into
one undo step for a continuous gesture; call `end_merge()` at gesture end.
Undo, redo, an explicit boundary or a different key ends merging. Refusal/unwind
restores the previous merge state. A no-op with a different key ends merging
without changing project/history. Keys are 1..=256 UTF-8 bytes. Call `end_merge()`
at save/selection changes; finish/cancel explicit gestures before those actions.
History eviction removes the oldest undo snapshot; it never splits a batch.
A changed edit clears redo; failures and no-ops preserve redo. Undo/redo move
existing snapshots, preserving exact integer timing and media/caption identity.

`registry()` owns stable IDs, labels, required parameter fields/Serde wire type
names and `CommandSpec::enabled(session)`. These are engine-owned descriptive
schemas, not JSON Schema. They are generated alongside `Command` from a single
declaration. Command fields are required; extra command/parameter/NewItem/Move fields
are refused. Model field shapes are the Serde shapes in their owning crates.
`Tick` is signed i64 ticks, IDs are u64 integers, NfcText is an NFC string,
TrackKind is a snake-case string, Color is four u8 channels, Vec is an array,
and optional values are null or their wrapped value. `NewItem.source_asset` may
also be omitted (None). Nested batch
entries use `{"command":"<id>","params":{...}}`. `NewItem` and `Move` expose
all their fields in the typed API. Enabled predicates advertise coarse state
availability; execution always validates exact targets and candidates.

For example, `execute("add_track", {"name":"Video", "kind":"video"})`
returns `Outcome { changed, created }`. `created` lists engine-allocated typed
track/item/bookmark IDs in execution order (including batch children). IDs share
a checked increasing counter initialized above all loaded track/item/bookmark
IDs. They are not reused after delete, undo, branching, cancelled previews or
rolled-back groups within a Session.
Media supplies asset identity; `add_asset` registers a validated SourceAsset and
does not perform probe/hash IO. A failed command/batch consumes no IDs; successful
preview/group children retire their IDs even when their outer operation rolls back. ID exhaustion returns
a typed error. Reloading a project starts a new Session/identity lifetime.

## Transactions, gestures and changes

`begin_gesture`, `commit_gesture` and `cancel_gesture` are registered commands and
have matching typed methods. During a gesture `project()` exposes validated
preview snapshots, while original undo/redo and pending committed changes remain
untouched. Commit adds one undo step, evicting at most one oldest step; cancel
restores the entry snapshot and pending event without adding undo/redo. Starting
or ending a gesture breaks merge continuity. A no-op gesture preserves redo and
emits no event. A refused preview command leaves the prior preview/gesture intact.

`transaction(|session| -> Result<T>)` stages several commands with the same
history/event rules. Success commits one step; error or unwind restores its entry
project, undo/redo, merge state and deferred event. The original history limit is
never raised; staged edits do not evict history. Nested gestures/groups and
undo/redo inside them return typed errors. Only engine state is transactional;
external effects performed by the caller's closure are outside that state.

`take_change()` drains a single coalesced committed-project snapshot. It returns
None during previews/groups, including for an older undrained event. Commit
publishes one change; cancellation/failure restores the previous pending event.
Consumers render explicit previews from `project()` and use committed changes for
persistence/processing. Begin/commit/cancel return no newly created IDs; individual
preview commands return their provisional IDs normally.

Commands and groups catch unwinding panics, restore their engine state and return
`EditError::Panicked` without formatting payloads. Aborts, OOM and double-panic
termination cannot be recovered. The application owns panic-hook policy. Errors
preserve project/history/merge state and undrained changes; successful undo/redo
always break merging. App presentation/selection/preferences are not Session state.

## Edit semantics and limits

Commands build a private candidate, then validate through `Project::new` before
publishing one snapshot. Failure leaves project, history and allocation untouched.
Each batch child must also yield a valid project; a failed child rolls back the
whole batch. Typed commands admit at most 1024 commands and nesting depth 16.
JSON dispatch additionally caps depth at 32, nodes at 200,000, each collection at
100,000 entries and aggregate string/key bytes at 16 MiB before deserialization.
The caller already owns the Value; these caps bound added processing, not its
initial allocation. Project and source constructors retain their own limits.

- `insert_item` places an item without shifting or overwriting its neighbors.
- `move_item` / `move_items` set absolute output positions and destination tracks.
  Multi-move removes all selected items first, permitting swaps. Duplicate IDs
  fail; wrong track kinds, overlap and missing references fail whole-candidate
  validation. Touching half-open intervals are valid.
- `split_at` uses an output tick strictly inside the identified item, retaining
  the left ID and allocating a right ID with continuous source timing.
- `trim_start` / `trim_end` set absolute output edges, adjusting source edges at
  unit speed with checked arithmetic. No implicit frame snapping/retiming occurs.
  Known media handles bound extension; generated content has no media endpoint.
  Caption mapping clips retained cues through captions' mapping owner, preserving
  source cue IDs, languages, text, translation references and edit flags. A trim
  commits retained cue content; a later independent extension cannot recover
  discarded cues. Gesture, closure-group, batch and equal-key merged trims rebuild
  each preview from retained entry cues rather than the previous clipped preview.
  An explicit `set_item_param` replaces that retained content; splitting establishes
  separate retained content for each piece. Refusal rolls back retained content
  with the candidate. Retention ends at commit/cancel or a merge boundary.
  Undo restores the original snapshot and all its cue content.
- `delete` lifts identified items leaving gaps. `ripple_delete` removes one item
  and shifts later items on that track left by its duration, preserving earlier
  gaps; other tracks and bookmarks stay fixed. No cross-track sync lock exists.
- `set_item_param` replaces the complete typed ItemParams. `set_track_flags`
  replaces all three flags and is permitted on locked tracks so they can unlock.
  All item edits, insertion, moving into/out of, and track removal refuse locks.
- `remove_track` removes its items together. `add_bookmark` accepts a nonnegative
  output position/duration and normalized note. Asset, track and bookmark
  additions are validated with the rest of the candidate.

Errors are typed `EditError` categories with safe IDs/static diagnostics and
retained project/time/caption sources. No raw JSON diagnostics or parameter text is
returned. Commands perform no IO or retries; processing lifecycle is described below.

## Verification and performance

`cargo xtask ci` exercises public JSON and typed commands, atomic failing batches,
simultaneous moves, locks, exact source/output split and trim math, ripple scope,
ID exhaustion/nonreuse, bounded history eviction, merging and redo branching,
gesture commit/cancel, failure at each batch position, group/command unwind
rollback, deferred/coalesced changes and no-op/failure redo preservation,
strict hostile input and seeded property sequences across rational frame rates.
Property checks reconstruct validated Project candidates after each operation,
undo every committed edit to the initial Arc and redo to the final Arc.
No codec/provider/native UI runtime behavior is established here. The ignored
`measure_large_project_gesture` test measures three 100-edit runs against 10,000
synthetic overlay items, asserting exact source timing, valid candidates and
undo restoration; it has no timing threshold.

Snapshots share via Arc, while each candidate clones project metadata; history
therefore has a count bound rather than a byte bound. Active transaction and
command rollback checkpoints retain bounded collections of Arc references. Batch
child validation consumes each candidate without another clone. Media bytes and
decoded frames are never copied by this crate. Byte-budgeted history and larger
real-world drag latency remain unverified; no bounded-RSS claim is made.

## Background jobs and byte policy

`jobs::JobRunner::new(config, budget, clock)` schedules blocking adapters on std
threads. `submit(spec, task)` returns a JobId and CancelToken; `tick()` joins
finished workers, admits queued work and observes progress. The application calls
it from its event loop. `JobContext` implements `ai::ExecutionHooks`; other domain
owners receive its cancellation/progress functions through their own hooks.
Engine disables ai's default native feature: composing an adapter is an app choice.
The task closure returns complete bytes or a typed `EditError` with retained AI/IO
sources. It must not apply commands or publish partial files. Independently complete
ASR chunks can be committed as checkpoints inside a task.

Source-documented rationale: existing AI adapters are blocking, Send + Sync and
hook-driven, so std threads meet the current execution contract without adding an
async runtime. `thread::Builder::spawn` returns OS creation failures; executing
closures run inside `catch_unwind`. `shutdown` and Drop cancel and join all active
workers. Adapters must check cancellation and put finite timeouts on blocking IO;
a noncooperating adapter can delay shutdown. Abort/OOM cannot be recovered.

One shared `ByteBudget` is cloned into every owner. Its snapshot publishes total
used/limit and per-owner bytes for decoded frames, compositor intermediates,
in-flight export, persistent artifacts, cache readers/staging and job results.
`reserve(owner, bytes)` returns a lifetime-bound lease; owners reserve **before**
allocating, including both overlapping batches and actual pixel storage. The
engine defines policy; frame/glyph inner loops stay in their data-owning crates.
Reservations are declared allocation ceilings, not an allocator sandbox or RSS
measurement. Captured task data is caller-owned and must be bounded; submission
cannot inspect arbitrary closures. The same reservation must not be charged a
second time by its adapter. Preferences/75 supplies the aggregate limit, rather
than allocating the whole limit independently to each cache.

RunnerConfig admits 1..=16 workers per CPU/GPU-encoder/network class, 1..=10,000
pending jobs and 1..=10,000 terminal results. Admission checks the sum of declared
work and maximum result bytes against the aggregate. Queue-full submission returns
`QueueFull`. Resource/byte-blocked tasks stay queued while other classes can run.
Result memory is reserved before execution; returned Vec capacity must fit that
reservation. Working reservations release on completion or backoff. Terminal
results keep their reservation until taken/dropped or evicted oldest-first at the
retention bound or to make room for new work. Consumers must handle an evicted ID
as absent. Outputs taken by consumers keep accounting leases until dropped.

`drain_events()` is a coalescing event channel: one latest snapshot per admitted or
retained job, bounded by pending + configured concurrency + terminal retention.
Intermediate events can be replaced; this is not an audit log. Worker progress is
finite 0..=1 and accepts at most one update per configured interval (minimum 10 ms,
including stage changes); `tick` forwards only changed snapshots. Terminal and
cancelling states bypass this progress limit. Slow consumers cannot block workers.
Cancellation is checked before execution and again before promoting success;
late partial success is discarded. Failure never yields a JobOutput.

Only Transcribe/Translate jobs in the Network class retry NetworkTransient;
all other errors and local/preview/export/import jobs run once. Retry attempts are
bounded to 1..=10, with configured exponential backoff through an injected monotonic
Clock. Workers/bytes release during backoff. If the pending queue filled while the
attempt ran, retry is refused by retaining the typed transient failure. The closure
must revalidate request consent and aggregate monetary authorization on every
attempt; the runner never grants consent or performs automatic provider fallback.

`submit_for_session(session, spec, task)` binds its JobOutput to the original
Session/revision; `output.apply(session, command)` uses that stored identity.
Jobs submitted without a Session cannot apply edits through this method.
`Session::job_target()` also captures the identity for externally adapted work. `apply_job_result(target, command)` refuses stale identity and active
previews/groups, then uses the normal atomic command boundary. Every changed edit,
undo/redo and changed preview invalidates older targets, even if undo restores
equal content. Failed transactions may conservatively invalidate them too.
Call `saved()` after a successful save; it refuses an active transaction and
invalidates pre-save targets. Opening another Session gives a new identity.
A valid old artifact may remain cached without being applied.

## Reuse and ASR checkpoints

`cache::CacheKey::new(KeyInputs)` hashes versioned, length-delimited BLAKE3 inputs.
Owners supply canonical normalized settings and semantic content hashes (not paths,
Debug strings or unstable JSON map order). Provider/model/version fields admit
1..=256 bytes each; settings admit at most 1 MiB. Every key includes source content,
provider/model/version, settings, transcript, cue, clip-range and translation
fingerprints; unused fields are zero. A changed input yields a different key,
without a generic dependency graph. Artifact kinds and their additional identity:

| Artifact | Inputs supplied by its domain owner |
|---|---|
| SpeechAudio | source content, exact source range, sample format/rate, conversion version |
| AsrChunk | speech/source content, exact invocation sample range, provider/model/version, decoding/language settings |
| Transcript | finalized chunk identities, recognition/assembly settings and version |
| Translation | transcript/text content, source/target language, provider/model/version/settings |
| Cues | transcript, translations, language profile and segmentation settings/version |
| Preview | cues, clip/source ranges, compositor/version and pixel/audio settings |

`ArtifactCache::open` owns one exclusive cache directory and rebuilds bounded
metadata after restart. Use only one live writer per directory; application
placement owns that exclusivity. It removes atomic staging debris and malformed
canonical entries with a diagnostic count. Unrecognized files are skipped and
never deleted; they are outside cache accounting. Directory scanning streams metadata;
excess entries evict without reading their payloads. Headers bind the canonical
key; `get` checks admitted size, header and payload digest before returning a hit.
Corruption/missing files return `CacheCorrupt` and remove the entry, so a caller can
record the typed diagnostic and recompute. Other IO errors retain their source.

`put` commits a complete artifact through project's synced atomic-file writer.
The 72-byte envelope is charged along with payload, so even empty entries consume
budget. The shared budget covers persistent bytes and the temporary envelope/read
allocation; put refuses an artifact whose envelope exceeds half the aggregate
limit (`ArtifactTooLarge`), because staging and disk must fit together. Pressure
evicts least-recently-used unpinned entries by bytes; max_entries also bounds
metadata. Reopen resets recency. A read returns an explicit ArtifactLease pin,
with separately charged read memory. Eviction and `clear` skip pinned files;
clear returns the count still in use and a later clear removes them after release.
Put cannot replace a pinned key. Dropping the cache keeps outstanding reader bytes
and disk charges alive through their leases; closed, unleased directories are
reaccounted on next open. OS/allocator overhead and unopened cache directories are
outside this active-engine accounting.

ASR resume stores **only independently complete invocation chunks**, each under
its exact content/range/model/settings key. A restarted job probes those keys,
reuses valid chunks, computes missing ones and assembles a final output only after
all chunks succeed. The public-job crash test commits chunk one, unwinds before
whole-result completion, reopens the cache and proves only chunk two executes.
This establishes checkpoint mechanics with synthetic bytes, not speech-boundary
quality for long recordings. Encoded-export resume is unsupported here; /09 owns
independently finalized segments, manifest/codec/source identity and timestamp
validation. No partial encoded file is treated as a reusable artifact.

Observed verification: `tests/jobs.rs` exercises actual submission with controlled
cancellation, failure, unwind, oversized results, deterministic retry/backoff,
resource/queue pressure, 200 completions with stable entry/charged-byte bounds,
semantic key invalidation, restart checkpoints, corruption, byte eviction, pinning,
clear during use and stale results after edit/undo/save/Session switch.
No new media/render algorithm is introduced; throughput, RSS, provider quality and
Windows runtime behavior remain unverified by these synthetic lifecycle tests.
