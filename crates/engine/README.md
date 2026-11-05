# engine

Headless command execution, Session ownership, project snapshots and transient item-selection history,
bounded background jobs and content-addressed artifact storage, plus batch export
orchestration for accepted ordered-segment shorts. Processing
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

`registry()` derives JSON Schema 2020-12 input schemas, output schemas, effects,
labels and executable examples alongside `Command` from one declaration. Leaf
shapes come from the owning types; fixed objects are closed recursively, including
empty params and nested batches. Domain constructors still validate invariants.
`CommandSpec::enabled(session)` describes coarse availability. Required nullable
keyframe fields remain required; `NewItem.source_asset` may be omitted or null.
JSON integer admission is limited to ±(2^53−1), including nested values. ProjectId
has one decimal-string encoding at its owner, including project persistence;
other IDs and Tick remain numbers. No adapter-specific encoding exists.

For example, `execute("timeline.add_track", {"name":"Video", "kind":"video"})`
returns `Outcome { changed, created }`. `created` lists engine-allocated typed
track/item/bookmark IDs in execution order (including batch children). IDs share
a checked increasing counter initialized above all loaded track/item/bookmark
IDs. They are not reused after delete, undo, branching, cancelled previews or
rolled-back groups within a Session.
Media supplies asset identity; `add_asset` registers a validated SourceAsset and
does not perform probe/hash IO. A failed command/batch consumes no IDs; successful
preview/group children retire their IDs even when their outer operation rolls back. ID exhaustion returns
a typed error. Reloading a project starts a new Session/identity lifetime.

## Timeline placement

`timeline.insert { placement, item }` performs track choice, optional allocation
and insertion as one atomic undo step. `placement` is a tagged object with
`mode: auto` (the default), `track` with a track ID, or `above`/`below` with a
reference track ID. `Outcome.placements` reports the final track and item IDs
for allocated items still present after the command/batch; `created` continues
to report only new IDs in allocation order.

Auto video insertion uses the unlocked main track when magnet is enabled.
It picks the nearest clip edge at the requested start (ties before), inserts
without replacing content, then packs the main track from zero. This is an
engine placement policy, separate from same-track ripple deletion/trim.
With magnet disabled, or for other kinds, auto chooses the first unlocked
compatible track whose requested half-open range is free: main, overlays in
screen order, then audio in screen order. If none fits it creates a secondary
track at the top of its group. Locked main falls back to secondary tracks.
The first insertion into empty main clamps to zero even with magnet disabled;
subsequent explicit placements retain their requested time. Explicit track
placement refuses overlaps and incompatible kinds. Negative starts and checked
arithmetic overflow refuse atomically, including track/ID allocation.

`timeline.add_track { name, kind, placement }` uses `mode: auto` by default,
or `above`/`below` with a reference ID. Relative placement respects the project
owner's groups: non-audio above main, audio below main; crossing these boundaries
refuses. Creating a track does not edit its reference, even when it is locked.
Compatibility is determined by `ItemParams::kind()` in project; empty reserved
track kinds remain available but cannot receive unsupported item kinds.

Command integration and fixed-seed random insertion tests cover compatibility,
ordering/non-overlap, first-main clamping, undo/redo and persistence round trips.

## Transient selection and shared fields

`set_selection(BTreeSet<ItemId>)` validates existing IDs, breaks merge continuity
and changes no Project content, revision or history length. A change during an
active gesture/group refuses. `selection()` exposes the current item IDs.
Each bounded undo snapshot retains the selection that produced the edit; undo
and redo restore both. Deleted IDs are pruned from current selection. Scope
selections (mask points/keyframes) remain app-owned. Selection is absent from
project JSON and optional persisted metadata.

`timeline.set_item_param_many` accepts a nonempty, unique `items` list (at most
100,000 IDs) and one typed `patch`: `audio_gain` in −120..=24 dB, `text_size` in
1..=4096 px, or `text_color` as four RGBA bytes. All numbers must be finite.
The command preserves unrelated fields and skips incompatible item kinds;
missing IDs, duplicates, invalid values and locked compatible items refuse the
entire edit. An all-incompatible selection is a no-op. One successful changed
command is one undo step, including every item's previous field value.

## Control admission, revisions and receipts

`control::{Request, Caller, Policy, Response}` is the transport-neutral boundary.
`Session::control_json` parses at most 8 MiB of UTF-8 with duplicate-key refusal,
32 nested levels, 200,000 nodes, 100,000 members per collection and 4 MiB aggregate
text. `control_request` also bounds already-allocated requests. Unknown commands,
wrong types/fields and nested batch publications refuse before effects. Validators
compile only generated local schemas with network/file resolution disabled.
API version 1 is independent of the project file schema. The BLAKE3 catalog digest
covers schemas, static effects/examples and limits, excluding enabled/session state.
Effectful calls require the exact digest and version; catalog reads need no digest.
[Generated commands](../../docs/engineering/commands.md) and `catalog.digest` regenerate
with `cargo run -p engine --example control_catalog`; the conformance test checks
both against current registry output in CI.

Every Session starts with an OS-random epoch. `revision()` is an opaque string;
its generation and `JobTarget` share the same Arc invalidation boundary. UI edits,
undo/redo, preview lifetimes, transaction rollback/net-zero end, view changes and
successful save advance it; ordinary no-ops/refusals do not. Effectful control
requests require `base_rev`. A stale revision refuses before any child of a batch.
Before publishing an admitted JSON/control edit, Session validates the candidate project's actual serialized
integers and created IDs, including values produced by allocation or tick/keyframe
arithmetic. Undo/redo validate their destination snapshot before moving history.
An invalid candidate returns InvalidParams without consuming IDs, advancing
revision or emitting a project change; its refusal receipt can be replayed. This
also applies to legacy JSON execution; typed in-process editing keeps the owner
types' full integer domain.
The recovery journal descriptor is version 2 for the same ProjectId encoding;
older descriptors are refused before model decoding, without deleting their files.
Session/project lifecycle creation and disk saves retain their existing
`Session::create`/`Autosave` ownership; direct Session edit/control execution returns
a typed refusal for those commands. Their registry examples declare those routes.

The composition root supplies `Caller` and a mandatory trusted `Policy`, outside
client JSON. Engine invokes it on the command and every batch child before effects
and before receipt replay. It must enforce source/destination/overwrite and provider
consent through their owners; it is not an authenticated transport implementation.
External callers cannot begin/commit/cancel gestures, including through generic
run or batches. External mutations refuse while the UI holds a gesture. Agents use
atomic batches and undo. Reads do not expose uncommitted gesture snapshots.

Effectful requests require a 1..=128-byte idempotency key. Receipts are scoped by
trusted principal, Session and key; the fingerprint covers command, params,
base revision and contract identity, excluding transport/request correlation.
Replay returns the original revision/result even after later edits; conflicting
semantics refuse. Mutable Session ownership serializes concurrent callers. The
ledger retains at most 4096 receipts and 32 MiB without eviction until Session end;
new effects reserve 128 KiB of available ledger capacity before execution. Mutation
results have at most 1024 created IDs; returned publication paths are capped at
4096 encoded bytes. Request IDs and principals are 1..=128 bytes. Refusals before
reservation do not occupy a key; accepted execution errors do. A new Session refuses
the old epoch rather than rerunning an uncertain mutation.

Inspection/catalog responses are capped at 8 MiB; oversize reads refuse instead of
truncating or mixing revisions. Safe wire error codes omit paths, user text,
provider bodies and Rust Debug. `bounded_json` is the adapter's bounded response
serializer. No transport, filesystem grant store, CLI persistent Session or MCP
runtime is introduced here; their composition belongs to the consumers.

`inspect`, `catalog`, `provider_catalog`, `preview`, general `export`, `job_status`
and `job_cancel` share the existing declaration. Export starts the production
render job against its admitted immutable snapshot and returns job ID/revision and
paths. Preview starts a CPU job through the same runner; job_status returns its
retained inline JPEG result without consuming it. Up to four exact output frames
are rendered at a maximum 640-pixel edge, JPEG quality 80, with frame/active clip IDs
burned in through the production text compositor. At most 16 active clips are
admitted per preview frame; larger label sets refuse. Labels that cannot fit at 14 output pixels within half
the image also refuse rather than clipping. Preview results retain their
captured revision; missing sources fail rather than invent picture evidence.
Images never enter receipts or event queues. Cancel requests obey the runner's
publication fence. Absent/evicted jobs return JobNotRetained; replaying their start
receipt never submits another job. Applications keep ticking app-owned jobs after
a listener disconnects; process shutdown retains JobRunner's cancel/join behavior.

## Control events and resynchronization

One typed `Payload` set covers command result/error, committed project change,
revision change and job snapshots. Envelopes include API/catalog/session identity,
revision and string sequence number. Command replies/receipts remain authoritative;
large read/image results stay on the response path. Project changes require a full
snapshot rather than pretending created IDs form a complete diff. Gesture previews
and cancellation emit revision changes, never committed project changes.

`snapshot_and_subscribe(&runner)` captures committed project, revision, current jobs
and a cursor while Session ownership is held. The owning loop calls `observe_jobs`
after runner ticks/accepted responses; it uses retained snapshots without draining
other consumers. Acceptance is visible before job lifecycle events. At most 128
small live events are retained for every listener together; cursors hold no queue.
A slow or foreign cursor gets ResyncRequired and must install a fresh snapshot.
No durable replay or exhaustive transition log is promised; progress can coalesce.
Terminal snapshots follow the existing runner/publication fence, and repeated polls
do not duplicate unchanged terminal events. Dropping a cursor never cancels work.

The control tests enumerate registry-owned examples and fixture requirements,
validate closed schemas, exercise real export/preview jobs, stale requests,
receipt replay/conflict/exhaustion, hostile parser/integer inputs, policy refusal,
gesture ownership and snapshot pressure. Measurements and acceptance limitations
are recorded in the local /07 done report; a build alone proves none of the
transport or installed-client behavior.

## Transactions, gestures and changes

`begin_gesture`, `commit_gesture` and `cancel_gesture` are registered commands and
have matching typed methods. During a gesture `project()` exposes validated
preview snapshots, while original undo/redo and pending committed changes remain
untouched. Commit adds one undo step, evicting at most one oldest step; cancel
restores the entry snapshot and pending event without adding undo/redo. Starting
or ending a gesture breaks merge continuity. A no-op gesture preserves redo and
emits no committed-project event; its closed preview lifetime advances the control revision. A refused preview command leaves the prior preview/gesture intact.

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
always break merging. App presentation/scopes/preferences are not Session state.

## Edit semantics and limits

Commands build a private candidate, then validate through `Project::new` before
publishing one snapshot. Failure leaves project, history and allocation untouched.
Each batch child must also yield a valid project; a failed child rolls back the
whole batch. Typed commands admit at most 1024 commands and nesting depth 16.
JSON dispatch additionally caps depth at 32, nodes at 200,000, each collection at
100,000 entries and aggregate string/key bytes at 4 MiB before deserialization.
The caller already owns the Value; these caps bound added processing, not its
initial allocation. Project and source constructors retain their own limits.

- `timeline.add_track` inserts a secondary track at the highest position for its group;
  `track.remove` refuses the mandatory main track and removes secondary contents
  in the same undo step. `track.set_muted` accepts Video/Audio;
  `track.set_hidden` accepts all kinds except Audio. Hidden filters visual output;
  muted filters audio, independently, for preview and export.
  `track.set_magnet` persists main-track magnet state, undoable with the project;
  gap-closing execution belongs to the ripple edit policy, not this preference.
- `timeline.insert` places an item without shifting or overwriting its neighbors.
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
  Selected `retranslate` target edits update that retained content and remap the
  visible view, preserving hidden/unselected cues and unclipped source ranges.
  An explicit `set_item_param` replaces the full retained content; splitting
  establishes separate retained content for each piece. Refusal rolls back retained content
  with the candidate. Retention ends at commit/cancel or a merge boundary.
  Undo restores the original snapshot and all its cue content.
- `delete` lifts identified items leaving gaps. `ripple_delete` removes one item
  and shifts later items on that track left by its duration, preserving earlier
  gaps; other tracks and bookmarks stay fixed. No cross-track sync lock exists.
- `set_item_param` replaces the complete typed ItemParams. `track.set_flags`
  replaces all three flags and is permitted on locked tracks so they can unlock.
  All item edits, insertion, moving into/out of, and track removal refuse locks.
- `track.remove` removes its items together. `add_bookmark` accepts a nonnegative
  output position/duration and normalized note. Asset, track and bookmark
  additions are validated with the rest of the candidate.

Errors are typed `EditError` categories with safe IDs/static diagnostics and
retained project/time/caption sources. No raw JSON diagnostics or parameter text is
returned. Commands perform no IO or retries; processing lifecycle is described below.

## Keyframe commands

Project owns the [parameter model and evaluator](../project/README.md#animated-parameters).
Engine adds the registered `keyframes.set_param`, `keyframes.add`,
`keyframes.remove`, `keyframes.retime`, `keyframes.set_handle` and
`keyframes.scale_time` commands. The registry owns their exact required fields and
wire types. `set_param` installs/replaces a validated static/animated override;
add/remove/move/handle edits select its path and an optional RGBA component.
For scalar/discrete params the component is null; RGBA requires r/g/b/a.
`retime` moves one key between exact local ticks and refuses an occupied time;
`scale_time` stretches all item channels/handle times by a positive rational,
without changing media playback or item duration. Missing paths/keys, invalid
handles, locks and failed budgets preserve the entire candidate/history.

Commands share existing typed/JSON dispatch, batches, undo/redo and gestures.
Begin/commit one gesture for a drag; previews commit one history step and cancel
restores the original curves. Range edits call project's checked translation,
retaining out-of-window keys; no second interpolation or tangent algorithm.
`NewItem.animations` carries explicit animation data on insertion. Rendering and
CLI property/export wiring are separate consumers of the project evaluator.

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

## Render kernel execution

`JobContext` supplies render's scheduling hook: up to eight workers, capped by
available parallelism. Each row batch borrows disjoint buffers, runs one task on
the job thread and uses scoped engine workers for the rest. It reserves 512 KiB
per extra worker stack before spawning, uses sequential execution when the
extra stacks do not fit, catches worker panics as typed render
failures, and joins all tasks on success, cancellation, refusal and unwind.
There is no detached work or retained pixel pool. Default hooks outside engine
remain sequential. The existing job-class limits bound simultaneous jobs; each
job can have at most seven additional row workers during a batch. Native FFmpeg
conversion receives the same concurrency bound and holds its separately charged
worker allowance for the converter/encoder lifetime; render/media retain their
pixel and codec ownership. `tests/render_kernels.rs` covers failure joins, stack
admission, released charges and parallel/sequential pixel equality.

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
in-flight export, persistent artifacts, cache readers/staging, job results and native
model disk usage. `used` includes every category; `limited_used()` excludes
`model_bytes()`, which are admitted by free-disk preflight rather than the memory
ceiling. Model files remain pinned and cannot starve ordinary work admission.
`reserve(owner, bytes)` returns a lifetime-bound lease; owners reserve **before**
allocating, including both overlapping batches and actual pixel storage. The
engine defines policy; frame/glyph inner loops stay in their data-owning crates.
On byte pressure, every reservation reclaims unpinned persistent artifacts in
one aggregate LRU order, including artifacts owned by other open caches. Reopened
entries register with the same policy. Cache storage alone cannot permanently
starve an admitted job. Oversized reservations fail before eviction; pressure from
active work/readers can still defer queued jobs until their leases release.
Reclamation performs synchronous file unlinking outside accounting/registry locks;
reserve before execution, never in an audio callback. Unlink failures return typed
IO errors and leave charges intact for a later retry. Atomic reader pins exclude
in-use files and a read's requested entry from reclamation; competing reclaimers
can yield temporary BudgetFull. Removed cache metadata is pruned on the next owner
operation and weak registry entries are compacted on registration/pressure.
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
late partial success is discarded. `JobContext::commit_publication` atomically
seals cancellation before publishing an irreversible complete artifact and sets the
Publish stage. A cancellation that wins the seal prevents publication; once sealed,
late cancellation is ignored and even a transient publication error is not retried.
The closure must perform its final cancellation-aware validation before sealing.
Failure never yields a JobOutput.

Transcribe/Translate/DownloadModel jobs in the Network class retry NetworkTransient or an
`ai::Error` whose `is_transient` is true (network, provider server, rate limit or structured schema failure); an
`ai` rate limit carrying `Retry-After` raises that attempt's backoff to at least the
provider delay, capped at five minutes. Auth, quota, payload, invalid, cancelled and all other errors, plus
local/preview/export/import jobs, run once. Retry attempts are bounded to 1..=10, with
configured exponential backoff through an injected monotonic Clock. Workers/bytes release during backoff. If the pending queue filled while the
attempt ran, retry is refused by retaining the typed transient failure. The closure
must revalidate request consent and aggregate monetary authorization on every
attempt; the runner never grants consent or performs automatic provider fallback.

`submit_for_session(session, spec, task)` binds its JobOutput to the original
Session/revision; `output.apply(session, command)` uses that stored identity.
Jobs submitted without a Session cannot apply edits through this method.
`Session::job_target()` also captures the identity for externally adapted work. `apply_job_result(target, command)` refuses stale identity and active
previews/groups, then uses the normal atomic command boundary. Every changed edit,
undo/redo and changed preview invalidates older targets, even if undo restores
equal content. Every group/gesture squash, including a net-zero group, and
transaction rollback or unwind invalidates targets captured inside its preview
lifetime. Group exit
conservatively invalidates targets even when final content is equal.
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
| ModelBundle | complete ai catalog record including named-file SHA-256, size, source and licence |

`ArtifactCache::open` owns one exclusive cache directory and rebuilds bounded
metadata after restart. Use only one live writer per directory; application
placement chooses the directory and an OS file lock enforces exclusivity (`CacheBusy`
on contention). It removes atomic staging debris and malformed
canonical entries with a diagnostic count. Unrecognized files are skipped and
never deleted; they are outside cache accounting. Directory scanning streams metadata;
excess entries evict without reading their payloads. Headers bind the canonical
key; `get` checks admitted size, header and payload digest before returning a hit.
Corruption/missing files return `CacheCorrupt`; corrupt-file removal waits for any
older reader pins to release, so a caller can record the typed diagnostic and
recompute. Other IO errors retain their source.

`put` commits a complete artifact through project's synced atomic-file writer.
The 72-byte envelope is charged along with payload, so even empty entries consume
budget. The shared budget covers persistent bytes and the temporary envelope/read
allocation; put refuses an artifact whose envelope exceeds half the aggregate
limit (`ArtifactTooLarge`), because staging and disk must fit together. Pressure
evicts least-recently-used unpinned entries by bytes; max_entries also bounds
metadata. Reopen resets recency. A read first pins its requested entry, then uses
the shared reservation/reclamation path for read memory, evicting unrelated
unpinned files as needed. A read returns an explicit ArtifactLease pin, with
separately charged read memory. Eviction and `clear` skip pinned files;
clear returns the count still in use and a later clear removes them after release.
Put cannot replace a pinned key. Dropping the cache keeps outstanding reader bytes
and disk charges alive through their leases; closed, unleased directories are
reaccounted on next open. OS/allocator overhead and unopened cache directories are
outside this active-engine accounting.

## Local model transfers

`models::ModelTransfers` composes the existing JobRunner, ArtifactCache and ai's
shared streaming transport. `open_default` reuses `ai::models::cache_dir`; `new`
accepts an already owned cache and injected `ModelIo`. The runner must share that
cache's ByteBudget. There is one active transfer per cache, including queue/backoff;
no additional runner or worker loop is created. State mutexes cover ownership
changes only, never network, file IO or verification.

`submit` requires ai's exact-model `DownloadConsent`, reauthorizes every attempt,
and reports Download/Verify/Publish stages through existing job events. The app's
model manager owns showing source, licence and size and obtaining the affirmative
click before granting consent; provider selection alone cannot authorize download.
Offline `installed` lookup requires no new download consent and verifies all named
files before returning a `ModelLease`. Retain that lease for the entire native
adapter lifetime, including lazy reads by the Gipformer child. Its root lock and
reader pin prevent another writer or explicit deletion while paths are in use.

ModelBundle is a native entry kind inside the same cache: a content-key directory
contains immutable catalog-named raw files and `.bundle` metadata in the existing
72-byte VCACHE01 key/digest envelope. Ordinary artifact files retain their layout.
Native metadata version 1 rejects other versions and malformed identities. Only an
atomically published verified-complete marker permits a ModelLease. Partial
checkpoints remain unavailable to loaders. Models and checkpoints are permanently
pinned against LRU/generic clear; only explicit `remove` deletes them, and live
leases refuse removal. Native bundles are exempt from the half-budget entry cap;
actual logical file and metadata bytes are reported as ModelWeights, with free-disk
preflight for remaining bytes plus atomic metadata headroom. Uncheckpointed tails
are reclaimed before append/restart and credited during preflight. Disk-full writes
still return typed StorageFull if availability changes after the advisory check.
Unknown directories and symlink targets are never adopted or overwritten.

Transfers stream through a fixed 64 KiB buffer within a 256 KiB job reservation.
Checkpoints sync raw files before atomically updating offsets every 4 MiB, at EOF,
on cooperative cancellation or read failure. A killed process resumes only the
last durable checkpoint and truncates its later tail. Range resumption requires a
strong matching ETag and exact Content-Range/length; a full 200 response replaces
the prefix. Weak/missing validators restart from zero. Changed 206 entities reset
before retry; malformed, encoded, excess, truncated and hash-invalid responses
cannot publish. HTTPS redirects are bounded to five, forbid credentials/fragments
and keep signed target URLs ephemeral. Hash/size checks reuse ai's streaming owner.
The shared client has finite timeouts; cancellation during a blocking request/read
can wait for the response or timeout. No live remote throughput or Windows runtime
claim follows from offline integration tests. Weights are never Git or installer
content; actual model-manager UI and CLI composition remain downstream work.

## ASR checkpoints

ASR resume stores **only independently complete invocation chunks**, each under
its exact content/range/model/settings key. A restarted job probes those keys,
reuses valid chunks, computes missing ones and assembles a final output only after
all chunks succeed. The public-job crash test commits chunk one, unwinds before
whole-result completion, reopens the cache and proves only chunk two executes.
This establishes checkpoint mechanics with synthetic bytes, not speech-boundary
quality for long recordings. Encoded-export resume returns a typed refusal in render; independently finalized
segments and manifest/codec/source identity would be required before admitting it. No partial encoded file is treated as a reusable artifact.

Observed verification: `tests/jobs.rs` exercises actual submission with controlled
cancellation, failure, unwind, oversized results, deterministic retry/backoff,
resource/queue pressure, 200 completions with stable entry/charged-byte bounds,
semantic key invalidation, restart checkpoints, corruption, byte eviction, pinning,
clear during use and stale results after edit/undo/save/Session switch. Retained
review regressions cover discarded transaction targets, cache-starved processing and read pressure.
Additional boundary tests cover failed/unwinding/net-zero groups and submitted
outputs, reopened/multiple-cache reclamation, global recency, active pins,
concurrent reclaimers and repair after unlink failure.
No new media/render algorithm is introduced; throughput, RSS, provider quality and
Windows runtime behavior remain unverified by these synthetic lifecycle tests.

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
receipt and destination lease together. Subsequent recovery continues. Successful
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

## Translation selection and application

`translation::window` selects cue IDs from one caption item, includes two source
context cues on each side, and skips target texts with edited=true unless force
is explicit. Unknown/duplicate IDs, no eligible source cues, missing source text
and mixed source languages are typed refusals. Selection and context are bound
in ai's canonical TranslationWindow; source timing remains untouched.

`TranslationRun::new` takes a TranslationTarget (Session and caption item), validates
that every selected ID/text still matches its source, and captures that revision
before retaining the selected provider, complete window, Scope and Consent. It preflights the per-attempt estimate against an explicit aggregate Money
budget for 1..=10 attempts. `submit(runner, session)` uses the existing shared job
runner and refuses a changed Session/revision before submission, including edits
during consent preparation. Each invocation atomically consumes
one operation-wide attempt, including schema and transport retries; retrying or
resubmitting the same run never resets that allowance. Every attempt revalidates
consent. The runner alone owns backoff and retry admission; Schema is retryable,
while refusal, auth/quota, payload/consent/cost and incomplete responses stop.
Schema repair retries the complete window; no partial output is applied or silently
switched to another provider. Four MiB work plus one MiB result is reserved from
the aggregate budget before execution. The content key includes the whole window,
source/target scripts and exact provider/model/settings identity.

Deserialize completed output bytes as ai::Translation and apply through
`JobOutput::apply(session, Command::Retranslate { item, target_language, texts,
translation_ref, force })`. The command's registered JSON ID is `retranslate`;
its required fields are those same typed fields, with force explicitly boolean.
Only returned/selected IDs participate, and unknown IDs or blank text refuse the
whole command. Existing edited targets are preserved unless force=true, including
when called directly through JSON. Source-language text cannot be replaced by a
translation. A successful command is one undo step with translation provenance;
locked items and stale Session outputs are refused through existing owners.
During retained trim gestures/groups/batches/merged edits, selected translations
update the authoritative cue track before remapping the visible range. Hidden
unselected content, source timings and all other language metadata survive inward
and outward reversal; protected no-ops preserve retention as well. Cancel/refusal
rolls these changes back with the candidate. Digit warnings travel in the AI result
for presentation; no provider edits Project.

Offline integration tests exercise actual OpenAI/fake-transport job submission,
schema repair on the selected provider, shared attempt limits, exact selection and
context, protected and force-retranslated targets, unselected cue preservation,
atomic hostile-ID refusal, undo and stale-result rejection. These establish the
execution contract, not human translation quality or actual cloud billing.

## Render execution

`submit_export` submits an immutable Project snapshot to the real runner's export
resource class. JobContext implements render's ExecutionHooks, leasing actual
buffers from the shared ByteBudget under decoded/compositor/export owners rather
than declaring a second whole-job charge. Render owns pixels/audio/output policy;
the job never applies editing commands. Completion returns an empty JobOutput
after the complete file has been atomically published.

Stages expose admission, rendering, mixing, encoding, finalization and publication.
`begin_publish` atomically changes running to publication, or refuses an earlier
cancel. Publishing is visible without progress throttling. A late cancel cannot
turn a committed output into Cancelled; shutdown joins that publication. Before
the fence, cancellation/unwind drops staging and preserves the existing file.
`tests/export_jobs.rs` verifies both sides of the fence with real production
render hooks, worker unwind, persistent-cache reclamation and oversized-frame
refusal before allocation. This does not establish crash/power-loss durability.

## Subtitle export commands

The existing `registry()` also declares `export_subtitles` and `burn_subtitles`.
They publish files without editing the Session or undo history. Dispatch their
strict JSON parameters with `Session::execute_export(runner, input, id, params)`
or their typed `Command` with `submit_export_command`; ordinary edit execution,
batches and active transactions refuse file publication. Both execute against a
captured immutable project snapshot in the existing JobRunner.

`export_subtitles` takes `track` (a Text TrackId containing captions), `format` (`srt` or `web_vtt`),
`languages` (one tag or two distinct tags in display order), native `path`, and
`overwrite` (bool). Ordinary title items on the shared Text track are skipped by
byte admission and cue serialization; caption items retain their source windows
and output gaps. `subtitle_file` exposes the same serialization plus captions'
typed repair warnings for preflight presentation. Each caption item's source
window maps to its output start through captions' EditMapping, preserving cuts,
repeated occurrences and timeline gaps. Captions owns the only SRT/VTT serializer.
This path opens no media and works with an offline source. Jobs reserve bounded
mapping/serialization scratch, write through project's StreamingStage, finalize,
then fence cancellation before atomic publication. Existing files survive refusal
or cancellation; source path aliases are refused through render's shared output
admission. Job output contains no transcript text.

`burn_subtitles` takes `range` (integer ticks), `languages`, `style` (render's
CaptionStyle), `portrait` (bool), native `path` and `overwrite` (bool). Portrait
selects 1080×1920; otherwise the project canvas is retained. It requires a visible
caption item intersecting the range and forces encoding through the production
render export path. Render owns pixel styling, media admission, encoding and
atomic finalization; missing/changed required video or audio returns its typed
asset/range diagnostics and preserves the previous destination. Advanced codec,
canvas and placement choices remain available through `submit_export` with
render's ExportSettings. UI/CLI adapters consume these engine APIs; they do not
implement their own overlay or serializer.

## Clip suggestion workflow

`clip_suggest` is the composition used by CLI/03 and later UI/04: the shared
`subtitles::transcribe` ASR-to-cue stage, an explicit clip provider, ordered-segment
ranking and 02 boundary snapping. The caller owns the runner, cache, credentials,
consent prompt, progress and cancellation token; the module writes no file during
suggestion.

`choose_suggestion` accepts only the offline `suggestion_options` catalog rows:
local `heuristic`, OpenAI `gpt-6-luna`/`gpt-6.1-sol`, or Anthropic
`claude-sonnet-5-5`. There is no fallback list: one provider attempt runs, and a
provider failure is returned unchanged. Remote adapters receive only transcript
segments (IDs, text, ranges and settings), never audio. Before the first upload,
`UploadRequest` names the provider, endpoint, model, exact payload bytes, cost
ceiling and the provider's data-retention/training policy link; refusal stops
before any adapter receives data. `DenyUploads` is the test default.

`ClipRequest` resolves the source language (or asks local whisper), runs ASR,
builds the AI `ClipInput` from the resulting cues, and executes the same
`providers::suggest_clips` path as other consumers. Candidates then pass through
`snap_clip_boundaries` with the source frame rate, caller tolerance/RMS and
duration/segment limits. A snap refusal drops that candidate with a typed
`RejectedClip` reason instead of publishing an unsnapped range; AI drops keep
their `ClipDropReason`. Candidate segments keep play order, roles, source-order
ranks and signed edge deltas.

`prepare_export` validates 1..=1000 distinct candidate indices, translates the
source cue track for each requested non-source language through the subtitle
translation path, and calls 05's `ClipBatch::prepare`. `export` keeps up to the
runner's export concurrency in flight (capped by terminal-result retention),
refilling slots as shorts complete and pumping the runner when its queue is full.
A failed admission or job cancels and drains admitted siblings before returning
its typed error; already published
complete files remain. The CLI only selects candidates and prints paths.

`cargo test -p engine --test clip_suggest` covers the ASR-to-snapped workflow,
source and translated caption preparation, portrait geometry, hostile/empty/
duplicate selections and the explicit provider catalog/credential refusal. Provider
attempt/consent tests remain in `tests/providers.rs`; media encoding remains in
`tests/clip_export.rs`.

## Clip batch export

`ClipBatch::prepare(source_project, ClipBatchRequest)` owns one export job per
accepted short; UI/04 and CLI/03 submit this same batch and add no renderer, cue
mapper or retry loop. A short is an ordered `ai::clips::ClipSourceSegment` list in
play order (`ClipShort::from_suggestion` keeps it) plus a title and optional
normalized crop. One request names the source asset, the source-timed `CueTrack`,
the existing output directory, none or one/two distinct caption languages, sidecar
formats (none, SRT, WebVTT or both), overwrite/degraded consent and a per-artifact
byte ceiling.

Preparation builds one derived immutable Project per short without touching media.
Video items keep each piece's exact source range at cumulative output starts, so
source gaps contribute no output duration and one piece is the single-range case.
Audio comes from each piece's source audio. The output is 1080x1920 (9:16) with
render's Cover fit: center crop by default, or the supplied normalized crop rect
for every piece. Joining forces `ExportMode::Encode`.

Cues map through captions' `EditMapping` over the ordered `(piece range, cumulative
output start)` pairs: fully cut cues drop, partial cues clip at piece edges, gaps
remain gaps, and play-order gaps never shift text. Each piece's caption item stores
the same clipped source-coordinate cues, so render's burn-in and `subtitle_file`'s
SRT/VTT output read one mapping. A requested language missing from any selected cue
refuses that short at preparation instead of failing during render.

Output names derive from the NFC title: characters illegal on Windows (`< > : " /
\ | ? *`), controls and bidi overrides become `-`; trailing spaces/dots are trimmed;
reserved device names are prefixed; the base is limited to 200 UTF-8 bytes on
grapheme boundaries; duplicates get deterministic `-2`, `-3` suffixes. Outputs are
`<base>.mp4` plus `<base>.srt`/`<base>.vtt`. Empty titles fall back to `clip-NN`. No
user text appears in job errors.

`prepare` accepts 1..=1000 shorts, 1..=32 pieces per short, nonnegative
nonoverlapping source ranges inside a known asset duration, one or two distinct
languages, at most one of each sidecar format, a 1 byte..=1 TiB artifact ceiling
and at most 256 MiB of prepared cue content across the batch. Request-level failures are `Invalid`; short-level failures are
`EditError::ClipExport { short, reason }`; a source asset without video is refused.

`submit_one(runner, index)` enqueues one prepared short and refuses an
out-of-range or already submitted index before touching the runner;
`submit(&mut runner)` enqueues every remaining short in order. The runner owns
concurrency and queue bounds. `job_id`/`snapshot` expose per-short progress, `cancel`/`cancel_all`
stop queued or running work and `retry` resubmits a failed or cancelled short with
its prepared inputs (a completed, running or evicted job is refused). A repeated
`submit` adds nothing. Sidecars are staged through project's StreamingStage before
the MP4 and published only after render's MP4 publication; a prepublish failure or
cancellation therefore leaves no new artifact, and every published file is
complete. A publication failure after the MP4 is reported but cannot roll back the
already complete MP4 or an existing MP4, which is preserved.

Each short retains publication progress across retries. `retry` skips artifacts
this short already published (checking that they still exist), republishes only
missing ones under the original overwrite policy, and re-renders the MP4 when it
is gone. It never treats an artifact this batch did not publish as skippable, so a
sidecar failure after the MP4 retries without deleting the MP4 or granting blanket
overwrite permission. Sidecar-only retries seal cancellation through the same
job publication fence: a late cancel cannot relabel committed output as Cancelled.

Missing or changed sources return render's typed `Unavailable` entries per job; the
batch does not silently degrade without `allow_degraded`. Storage exhaustion and
the byte ceiling surface as typed IO/project failures from the bounded stages,
leaving previous destinations and no staging orphan. Every artifact path is
validated through render's shared output admission against the retained immutable
original project, not only the selected source, so exporting into any original
project source path or hard link is refused even with overwrite permission.
Retry reuses the prepared snapshot, so relinking a source requires preparing a new
batch.

`cargo test -p engine --test clip_export` covers play-order joining, gap exclusion,
one concatenated mapping against `subtitle_file`, bilingual cue ordering, portrait
crop placement, Unicode/bounded/unique names, hostile ranges/languages/formats,
missing sources per short, cancel mid-batch with neighbour completion and retry,
storage ceilings for sidecars and the MP4, refusal of an unselected video or sidecar
source alias under overwrite, and retry after a sidecar publication failure without
manual deletion. Oracle tests generate a synthetic FFmpeg fixture and probe the
exported 1080x1920 MP4 and both sidecars.

## Provider selection and fallback

`providers::{transcribe, translate, suggest_clips}` executes an AI-owned
`ProviderSelection` on the existing job worker with its `ExecutionHooks`. A caller
that configures a local fallback uses a CPU resource reservation for the whole
run, so switching from network to inference cannot bypass CPU concurrency limits.
Every attempt must exactly match the persisted feature/provider order; no adapters are
inferred, and each adapter is invoked at most once. A successful attempt ends the
run. Auth, quota, network, server, rate-limit, schema and incomplete-response
failures advance only to the next configured provider; cancellation, consent,
cost, input and capability errors stop immediately. Exhaustion returns the last
typed failure. No retry budget is reset or second scheduler introduced.

Composition implements `providers::Approval` to show provider identity, its
policy link, exact payload and cost at setup and again immediately before a
fallback executes. The callback receives `fallback=true` for the renewed prompt;
a remote attempt without its own exact consent token cannot upload. Approval
errors stop the run. Clip windows are all approved before a provider's first IO;
partial results from a failed provider are discarded. Local heuristics are a
normal explicitly configured `ClipSuggester`, never an implicit recovery path.

`ProviderOutput` retains the exact successful provider configuration beside its
output; serializable outputs preserve this provenance in job result bytes. Job
callers retain the complete envelope when storing results, and include the entire
selection and request settings in cache keys. Recognition remains its existing
sample-coordinate model, including text-only results when timing is unavailable.
The functions do not mutate projects; translation results still go through the
registered edit command. There is no desktop settings UI in this crate.

`cargo test -p engine --test providers` observes no-fallback defaults, ordered
provider failures, renewed approval before any fallback upload, rejection of
primary-token reuse, refusal/cost/cancellation and exact provenance. Offline
injected adapters establish execution policy, not third-party model quality.

Clip provider outputs preserve AI-owned ordered source segments, resolved source-order
ranks and summed duration; consumers use the [AI clip contract](../ai/README.md#transcript-clip-suggestions).

## Clip boundary adjustment

`clip_boundaries::snap_clip_boundaries` takes explicit shorts as vectors of
`ai::clips::ClipSourceSegment` in play order, a source Transcript, its resolved
LanguageProfile and LayoutHooks, media SpeechEnergyWindows and
ClipBoundarySettings. Suggestion generation is optional. The caller can submit
media energy decoding through the existing JobRunner with its cancellation token;
snapping itself performs no IO or project edits.

Every source edge is adjusted independently to an output-fps frame within the
configured tick tolerance. Selection prefers a sentence gap with low RMS, then a
sentence gap, low RMS in another safe gap, then any word/segment gap; ties prefer
nearest time, then earlier time. RMS threshold is an explicit amplitude ratio.
A quiet sample never permits a cut inside a captions-owned indivisible speech
unit. Quantization is constrained to the safe gap, rather than rounded into a
word afterward. Missing/incomplete native timing protects the whole segment;
proportional subtitle timing is never used for cuts. A gap with no frame in the
tolerance is refused, rather than cutting speech.

SnappedShort keeps play order, roles, source-order ranks and repair metadata,
with adjusted ranges, summed duration and signed original/adjusted tick deltas
for both edges of every segment. Inputs remain unchanged. Invalid candidates,
source-order ranks, multiple/misplaced hooks, overlapping source ranges,
post-adjustment overlap and duration violations are typed refusals. A whole call
is atomic. Positive total/piece duration limits and a segment limit in 1..=32 are
caller-selected; at most 32 shorts and 360,001 energy windows are accepted.
`BoundaryFailure` identifies candidate, safe-frame, overlap or duration refusal
with zero-based short/segment indices (whole-short failures use segment zero).

Tests inject reordered multi-segment candidates, known-pause PCM generated by
external FFmpeg, silence conflicting with words, incomplete timing, tolerance and
frame conflicts, duration/overlap failures, and 128 fixed-seed rational-rate
samples. `examples/clip_boundary_benchmark.rs` measures a bounded local media and
Transcript pair, including signed deltas and mid-speech counts against nearest
frame snapping. This proves source-timing constraints, not ASR accuracy.

## Subtitle workflow

`subtitles` owns the shared end-to-end composition used by the CLI and later UI:
speech-window extraction, explicit transcription/translation provider choice,
checkpointed ASR chunks, optional local alignment, cue segmentation, translation
application and the existing SRT/burn export jobs. It never reimplements captions,
render, provider consent/cost or cache/checkpoint primitives.

`provider_options()` is offline capability data (operation, model ids, languages,
local/remote and native timing) for pickers. `choose_transcription`/`choose_translation`
accept only a listed model and never fall back. `Runtime` selects local threads,
the explicit GPU request and the shipped Gipformer runner path. `Credentials::new`
accepts composition-root secrets and validates them; errors name the environment
entry, never the value.

`Providers::open` loads only the selected adapters. Local Whisper/Gipformer models
come from verified `ArtifactCache` bundles and keep their `ModelLease` alive for the
adapter lifetime; text-only ASR additionally loads the pinned `whisper-small-dtw`
Candle aligner. Missing weights return `ModelMissing`, a missing runner
`RunnerMissing`, and missing credentials `CredentialMissing`. Remote adapters use
the official endpoints, shared `ureq` transport and their own disclosures.
`with_translation` attaches the explicitly selected translator only after an
auto-detected source language is known, so a same-language local run never requires
unrelated cloud credentials.

`run(request, providers, runner, cache, consent, progress, cancellation)` imports
the input, extracts mono 16 kHz windows at speech-safe boundaries, reuses complete
raw `AsrChunk` cache entries keyed by source content, exact sample range, language
and provider/model/settings, executes missing windows as Transcribe jobs, then
applies the selected local aligner as a separate CPU job when the recognition is
text-only. Raw recognition is committed to the checkpoint cache before alignment,
so an alignment failure preserves completed ASR work; alignment is recomputed on
the next run and its request is bound to the exact recognized text by
`ai::align`. The workflow then assembles with `ai::assemble`, converts and
segments through captions, applies translation windows through the existing
`TranslationRun`, and dispatches `export_subtitles`/`burn_subtitles` through the
real job runner. Silent windows are skipped; no speech, unknown duration and burn
without video return typed refusals. No project file is written.

`Cancellation` is the operation-wide cooperative signal. `run` passes its media
flag to extraction, checks it at every stage/window boundary, and cancels the
active submitted job; `wait_for_job` joins that job before returning. Cancellation
after a completed ASR window keeps the checkpoint. Export keeps render's existing
publication fence: once publication is sealed a late cancel cannot turn the
completed file into `Cancelled`, and cancellation before the fence preserves the
existing destination. The CLI currently exposes no signal-driven cancel gesture,
so process interruption remains crash semantics; UI/CLI callers that do have a
gesture pass the shared token.

Remote upload scopes are prompted through `ConsentPrompt` before every request;
`DenyUploads` is the default test refusal and `AllowAll` is the explicit test
grant. `UploadRequest` carries provider/model, languages, audio seconds, payload
bytes, cost ceiling and the adapter's official policy links. The
workflow never uploads without a prompt approval and never downloads weights.
`ModelDownloader` wraps one explicitly consented catalog download and
`default_cache_root` is the single shared model/checkpoint location.

Tests in `tests/subtitles.rs` cover all six vi/en/zh translation directions and
every bilingual pair, SRT plus burned MP4 output, ASR checkpoint reuse on a second
run, remote consent refusal with zero calls, credential refusal, catalog filtering
and a real generated fixture. They establish composition and failure contracts,
not model accuracy, live provider quality or Windows behavior.

## Project lifecycle and view state

`Session::create(Command::CreateProject { id, name }, history_limit)` consumes the
registered `project.create` command and uses project's default canvas/timeline.
`project.rename` is an ordinary edit with undo/redo. `project.set_view_state`
requires `zoom`, `scroll` and `playhead`; it validates through project and changes
only `Session::metadata`, outside project undo and committed-project notifications.
View changes are refused inside gestures/groups/batches. `restore_metadata`
loads the optional sidecar at open and is also refused during transactions.

`Autosave::open` reads metadata after acquiring its writer lease. Its background
save writes the project and then the sidecar before publishing a receipt. Desktop
and CLI use that same lease metadata API. `Autosave::execute("project.save", {})`
queues the registered save; callers must poll/flush and check acknowledgement.
`Session::execute` refuses create/save because they require lifecycle ownership;
commands stay in the one generated registry. Destination admission refusals before
a persistence job starts (including Busy, unsupported names and lease IO errors)
return their typed error without poisoning the active autosave owner; choosing a
different destination can succeed. Once a job is admitted, publication and worker
failures retain the sticky error policy.

## Effect commands

`effects.add { item, effect }`, `effects.remove { item, effect: instance_id }` and
`effects.update_params { item, effect: instance_id, params }` mutate the project
stack through normal atomic history. Locked/missing items, duplicate IDs, unknown
instances, wrong types and out-of-range intensity refuse. Add stores enabled state
and static parameters; scalar animation overrides use `keyframes.set_param` at
`effects.<id>.params.intensity`. Remove deletes those animation paths in the same
undo step. CLI `exec` consumes these same registered commands before export.

## Mask edits

`masks.add`, `masks.remove`, `masks.update` and `masks.reorder` operate on the
project-owned ordered mask vector through the same atomic candidate/history path.
Update selects by the supplied mask's stable ID; reorder requires an exact
permutation. Missing/duplicate identities, invalid parameters and locked tracks
refuse the entire command or batch. Removal also removes that mask's animation
channels; undo restores both. Registry schemas and CLI execution share these
commands and their examples. No app-only mask state or evaluator is introduced.
