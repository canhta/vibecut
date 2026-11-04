# engine

Headless command execution, Session ownership and project-only snapshot history.
Jobs, scheduling, processing adapters and caches are excluded from this slice;
processing algorithms remain with their domain owners. No widgets, device state,
app preferences or platform API is required.

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
  commits retained cue content; extending it cannot recover discarded cues.
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
returned. There is no command IO, retry or cancellation in this slice.

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
