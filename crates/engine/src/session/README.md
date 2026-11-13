# Session

Interface: `Session`, `Fork`, `ChangeSummary`, `Command`, `CommandKind`, `registry`, `JobTarget`, `transcript`, `CueId`,
`WordId`, `EditError`, `AssetUsage`, `wire`.

`SessionError::StaleResult` owns stale job-target refusal: the original static diagnostic,
StaleResult code, no source or transience, and ordinary job policy. The one typed identity
guard compares original epoch and revision Arc pointers only. `apply_job_result` validates
identity before refusing an active transaction and applying a command. Translation submission,
recovery adoption and retained Runtime output validation consume the same guard, retaining
their own transaction/no-output/consent checks and ordering. Higher APIs compose the error
through `EditError::Session` inside pure editing or `engine::control::Error::Session` at a mixed
workflow boundary; this is not a new wire tag.

The Session owner fills identity entropy through one crate-visible operation returning
`SessionError::ControlIdentity` on platform failure: `control identity unavailable`, Refused,
nontransient and source-free. Session construction validates history before filling its
32-byte epoch seed; library private staging uses the same operation for its 16-byte nonce.
Both retain their BLAKE3 encoding. This operation neither allocates ProjectIds nor falls back
when entropy is unavailable.

Session lifetime and history refusals belong to the same `SessionError` owner:
active transaction/gesture, absent gesture/history and command unwind. Their diagnostics
and categories remain source-free, nontransient and ordinary job policy. Command unwind
is distinct from Runtime worker panic. Higher workflows compose these causes through
`EditError::Session` in pure editing. Catalog-local undo/redo composes its no-history
refusal through `library::Error::Edit` at its managed-library boundary.

The internal gesture start returns only `SessionError`, consumed by BeginGesture and
TimelinePreview. Public begin/commit/cancel, undo/redo and guarded transactions still run
through the mixed command boundary: generation reservation, wire/project validation and
checkpoint rollback are not bypassed to narrow their signatures.

Pure Kernel methods return `Result<T, EditError>`; the local Result names that one
owner, never the engine aggregate. EditError has no Runtime/recovery/provider/render/AI
variant. Higher workflows compose genuine edit errors with `engine::control::Error::Edit`.

`transaction` accepts a caller error `E: From<EditError>` and returns that same E.
Callback refusal is returned unchanged after rollback; only Kernel admission, squash,
revision and unwind refusals convert through EditError. This keeps both pure edits and
mixed workflow callbacks usable without introducing upper-owner knowledge here.

## Public entry points

`Session::new(project, history_limit)` admits 1..=1000 retained undo steps. `project()` exposes
an immutable `Arc<Project>`; `apply(Command)` is the typed edit entry. The session has no method taking a command id string: decoding a wire id is
[`control::run_wire`](../control/README.md#string-entry), and in-process control reads use
`Session::control_command`. Every changed `apply` is one undo step; a continuous gesture becomes one
step through `begin_gesture` and `commit_gesture`, and `transaction` groups several commands. Refusal or
unwind restores the previous state. History eviction removes the oldest
undo snapshot and never splits a batch. A changed edit clears redo; failures and no-ops preserve
it. Undo and redo move existing snapshots, preserving exact integer timing and media and caption
identity.

`registry()` derives JSON Schema 2020-12 input schemas, effects, labels and executable
examples from the same declaration as `Command`. Leaf shapes come from the owning types; fixed
objects are closed recursively, including empty params and nested batches. Each standalone
parameter schema keeps only its transitively referenced definitions so the catalog stays within
the response byte budget. Each declaration names the `ControlResult` variants its dispatch may
return (`returns = [..]`, `Edit` when omitted); the entry's `output_schema` is the union
`control::output_schema()` narrowed to those variants and the definitions they reach, the way
`params_schema` is narrowed, and the registry digest covers it. No response repeats the union.
Domain constructors still validate invariants.

Each declaration also carries per-command facts: an `Availability` rule (default `Always`),
`idle = true` for the commands refused while a gesture or transaction is open, the
`collections = [..]` and `batch = ..` parameters bounded at `MAX_COLLECTION` and
`MAX_BATCH_COMMANDS` elements (exposed per entry as `collections` and enforced by one
admission step on the control transports and in `check_budget`; a schema `maxItems` bound
replaces the declaration), `transitions = true` for the commands that keep video transitions
consistent themselves, and, outside the wire catalog and its digest, a `GrantClass` (`Agent`, `Output` for the
file-publishing commands, `Human`; required for session and job control), the `curated` MCP
flag and the `Lifecycle` route of the library and persistence commands (including
project save-as, bundle export and bundle import). `lifecycle(id)` is
the one routing answer adapters share; batch and candidate refusal of non-edit commands derives
from the declared effect. A new publishing command also needs its `Command::published_path`
arm. `CommandKind` is the parameter-free identity generated by that declaration, and
`Command::kind()` exposes it for typed commands.

The same declaration generates the private edit-only command consumed by the candidate
reducer, moving payloads without cloning them. Non-edit admission uses the declared effect;
adding a read, publishing, job or session command needs no reducer refusal arm. Every edit
variant still requires an exhaustive reducer arm, and an unclassified effect fails expansion.

Required nullable keyframe fields stay required; `NewItem.source_asset` may be omitted or null.
JSON integer admission is limited to ±(2^53−1), including nested values. ProjectId has one
decimal-string encoding at its owner, including project persistence; other IDs and Tick remain
numbers.

`apply` returns `Outcome { changed, created, skipped }`, where `skipped` lists the transitions a batch-wide edit left unchanged on locked tracks. `created` lists engine-allocated typed track,
item and bookmark IDs in execution order, including batch children. IDs share one checked
increasing counter initialized above all loaded IDs and are not reused after delete, undo,
branching, cancelled previews or rolled-back groups within a Session. A failed command or batch
consumes no IDs; successful preview and group children retire theirs even when the outer
operation rolls back. The edit allocator returns the kernel's `SessionError::IdExhausted` through ordinary
composition. Its last u64 ID is returned once, then the absent next ID refuses without
mutation. This is separate from Runtime job-ID exhaustion and generation/wire limits. Reloading a project starts a new
Session and identity lifetime.

## Forks

`Session::fork()` returns a `Fork`: an isolated session with its own identity (so receipts and
job targets never cross), the parent's project, ID counter, ripple policy, clipboards and view
state, no history, no jobs and an empty command log. It refuses `TransactionActive` while a
gesture, preview or group is open. Nothing a fork does reaches its parent.

Every command that changed the fork's project is logged once, as the caller applied it and with the
entry it ran through (typed or wire). The log holds what the kernel applied, so a job-result
command (`captions.apply`, `reframe.apply`) appears as the edit it resolved to and replays without
the job. A refused command, a no-op and the children a command applies to itself are not logged.
A `Fork::transaction` group is logged as one unit and a failed one drops its children. A fork refuses every `SessionControl` command
(undo, redo, gestures, previews, scene and view changes, ripple, clipboard copy): they belong to the
parent. `Fork::commands`, `is_empty` and `summary` read the log; commands run through
[control](../control/README.md#forks).

`Session::apply_fork(fork)` replays the log inside one `transaction`, one undo step and one change
event. It refuses `StaleRevision` unless the session is at the fork's `base_revision`, which also
rules out a successor session of the same project. That is the intended guarantee that a fork
cannot outlive `project.delete`: the revision embeds the session identity, and reopening a deleted
and restored project makes a new session. An empty log is a no-op: no undo entry, no event, no new revision. A failing
child, or a replay that does not end at the fork's project (`SessionError::ForkDiverged`),
restores project, history and pending change; the revision still advances as for any rolled-back
group, so job targets captured meanwhile go stale. Replay clears trim cue sources between
entries because the fork ran each on its own, and keeps them inside a group. Each logged command
carries the fork's ID counter at its start and replay resumes from it, so IDs a failed fork
transaction retired stay retired on the live session too. The live playhead
does not follow a fork's paste. The kernel does not cap the log: the agent's per-turn step limit
bounds it. `apply_fork` does not run the control `Policy`:
each command was authorized on the fork, at the same revision.

`ChangeSummary` (`Fork::summary`, or `ChangeSummary::between` for any two projects) lists, for the
active scene, the item ids added, removed, moved (start or track), trimmed (source range),
retimed, parameter-changed (content, effects, masks, animations, blend mode, visibility, source
asset) and caption-changed (caption content), with the scene duration before and after. An item can
be in several lists. The app and the model read this one value.

## Transcript read model

`Session::transcript()` returns the active scene's caption cues in output time, each with its
item and track, source and target texts, `protected` (the cue's source text was explicitly
edited) and `locked` (the owning track) flags, and its measured words. A `TranscriptWord`
carries a `WordId` (`item`, retained `cue`, word index), the paired source token when token and
timing counts agree, its output range and the same flags. A `CueId` addresses one caption cue by
`item` and retained `cue`. The view reads `Item::word_spans`, so a trimmed item clips its words
and repeated source spans produce one entry per occurrence. It changes no project, revision or
history. A `WordId` or `CueId` from another scene is refused with `SceneBoundary` by the command
path rather than cutting two timelines.

## Filler candidate read model

`timeline.filler_candidates {language, include_markers}` runs the captions lexicon detector over
every caption item's visible cue track, so a candidate always names output ranges the timeline
shows. Each result carries its id (`filler:<item>:<cue>:<source word start>`), item and retained
cue, output range, `filled_pause`/`discourse_marker` kind and the cue's `edited_cue` flag;
filled pauses are the default and markers require `include_markers`. An unknown language is a
typed refusal; cues without measured words contribute nothing. The query changes no project,
revision or history, and it refuses during a gesture or group because it reads committed
output. `fillers.classify` is the separate opt-in network job and returns the same ids.
`timeline.remove_fillers` resolves an id through a fresh local detection first, then as one
visible measured word, and refuses an id no visible word matches rather than cutting a
different word.

## Transactions, gestures and changes

`begin_gesture`, `commit_gesture` and `cancel_gesture` are registered commands with matching
typed methods. During a gesture `project()` exposes validated preview snapshots, while original
undo/redo and pending committed changes stay untouched. Commit adds one undo step, evicting at
most one oldest step; cancel restores the entry snapshot and pending event without adding
undo/redo. A no-op gesture preserves redo
and emits no committed-project event; its closed preview lifetime advances the control revision.
A refused preview command leaves the prior preview and gesture intact.

`transaction(|session| -> Result<T>)` stages several commands with the same history and event
rules. Success commits one step; error or unwind restores its entry project, undo/redo
and deferred event. The history limit is never raised and staged edits do not evict
history. Nested gestures or groups, and undo/redo inside them, return typed errors. Only engine
state is transactional; external effects performed by the caller's closure are outside it.

`take_change()` drains a single coalesced committed-project snapshot. It returns None during
previews and groups, including for an older undrained event. Commit publishes one change;
cancellation or failure restores the previous pending event. Consumers render previews from
`project()` and use committed changes for persistence and processing. Begin, commit and cancel
return no newly created IDs; individual preview commands return their provisional IDs.

Commands and groups catch unwinding panics, restore their engine state and return
`EditError::Session(SessionError::Panicked)` without formatting payloads. What a refused or unwound command restores,
and what it deliberately keeps, is the [rollback contract](state/README.md#rollback). Aborts,
OOM and double-panic termination cannot be recovered; the application owns panic-hook policy.
Successful undo and redo always break merging. App presentation, scopes and preferences are not
Session state.

## Project lifecycle and view state

`Session::create(Command::CreateProject { id, name, fps }, history_limit)` consumes the registered
`project.create` command and uses project's default canvas and timeline. `project.rename` is an
ordinary edit with undo/redo. `project.set_view_state` requires `zoom`, `scroll` and `playhead`;
it validates through project and changes only `Session::metadata`, outside project undo and
committed-project notifications. View changes are refused inside gestures, groups and batches.
`restore_metadata` loads the optional sidecar at open and is also refused during transactions.

`Autosave::open` reads metadata after acquiring its writer lease. Its background save writes the
project and then the sidecar before publishing a receipt. `Autosave::apply(Command::SaveProject
{})` and `Autosave::apply(Command::ThumbnailProject {})` route through `Command::lifecycle()` to
queue the registered save or refresh the cover; callers must poll or flush and check
acknowledgement. `Session::apply` refuses create and save because they need lifecycle
ownership. Destination admission refusals before a persistence job starts (including Busy,
unsupported names and lease IO errors) return their typed error without poisoning the active
autosave owner; choosing a different destination can succeed. Once a job is admitted,
publication and worker failures follow the sticky error policy.

Named caption preset IDs share the existing allocator and loaded maximum with scene, track, item, transition and bookmark IDs. Successful then cancelled previews/groups retire these IDs; failed atomic batches consume none. Bank metadata participates in ordinary snapshots, undo/redo and recovery; cue-table style IDs remain local to each cue track.

## Clipboard

`clipboard.rs` owns `KeyframeClipboard`, `ItemClipboard` and their `Stored | Payload`
paste sources, consumed by `clipboard.copy`, `clipboard.copy_keyframes`,
`clipboard.copy_mask_points`, `clipboard.read`, `clipboard.cut`, `clipboard.paste`,
`clipboard.paste_keyframes`, `clipboard.paste_mask_points` and `timeline.paste_attributes`
([attribute clipboard](clipboard/README.md)). One Session slot holds the most recent
copy of either kind; a successful copy of one clears the other. Copy of an empty or
already-identical scope preserves the store.

`clipboard.cut` takes one `CutTarget` (items, keyframes or the anchors of one path mask).
It captures the same payload as Copy, then runs the matching delete (the item delete
follows the session ripple setting; keyframes use `keyframes.delete_selection`, mask points
`masks.remove_point`) in one transaction, so it is one undo step. The payload is stored
only after that delete commits: a locked, stale or empty target refuses or no-ops and leaves
the previous slot untouched. Mask-point cut is described under
[Mask-point clipboard](#mask-point-clipboard).

### Keyframe clipboard

The version-one closed snapshot contains a minimum original item-local output tick
and strictly ordered path/Param pairs containing only selected keys. It admits one
source item, 64 paths, 4096 refs/keys and 1 MiB compact UTF-8 JSON. Every time,
handle delta, derived offset and pasted time fits ±(2^53−1); invalid values,
versions, duplicates, missing keys and unordered entries refuse atomically.
Empty Copy preserves the store. Source locks permit Copy. Availability and Copy first
admit a borrowed projection: capped reference metadata, selected-key/time/component
checks, then compact JSON byte counting with actual escaping and borrowed normalized
text. No selected key, Text or base is cloned before that admission; availability never
materializes a snapshot. Copy constructs the owned payload only after it passes. Typed
construction and deserialization retain the same validation and byte ceiling.

Paste accepts one active-scene item with matching existing Params, including channels
with zero keys. It adopts each participating channel's base and extrapolation,
retains all curve metadata, and merges translated keys with full replacement at equal
times. Nonparticipating RGBA components and unrelated paths remain exact. The local
anchor is scene output `at` minus destination start, plus each original time minus
snapshot origin: there is no media mapping, snapping or clamping. Destination locks,
missing channels, incompatible types, overflow and final project budgets refuse all
paths. Paste availability borrows the target and visits only snapshot paths/components:
it checks translated times and merged per-channel counts with replacement at equal times,
without cloning any target channel or unrelated value. Actual Paste shares this admission
before constructing participating channels in the ordinary atomic edit candidate. Stored-empty Paste is a no-op without target lookup. Changed Paste is one undo
step and does not move the playhead; equal content preserves redo.

### Item clipboard

The version-one item snapshot holds at most 1024 full `Item` snapshots, each with its
source track kind and zero-based index among tracks of that kind in top-to-bottom screen
order. Every item carries a zero placeholder id and a nonnegative group-relative start;
the group origin is the earliest copied start and `span` is the latest relative end.
Content, body parameters, trims (`source_range`), retime, effects, masks, animations,
blend mode, hidden flag and source-asset identity therefore survive the closed payload.
Compact UTF-8 JSON is capped at 4 MiB; every time and derived end fits ±(2^53−1).
Copy refuses duplicate or missing items, a track whose kind differs from the item kind,
and captures from locked tracks. It never edits project, selection, playhead or history.

Paste resolves each item against the active scene by kind/index and falls back to the
last track of that kind when the index is gone; a kind with no track, a locked target or
a missing source asset refuses the whole paste before any change. It clears every
overlapped range on those tracks first, splitting a containing item so both surviving
heads keep source, retime, fades and cues, then inserts fresh ids in start order. Normal
Paste overwrites at the playhead and moves the metadata playhead to the pasted span end
so repeated Paste advances; Paste Insert ripple remains future work. Stored-empty Paste
is an exact no-op. A changed Paste is one undo step and reports only the freshly inserted
items as created, not overwrite split pieces.

### Mask-point clipboard

`ItemClipboard` also carries mask points: the same version-one payload with `maskPoints`
(1 to 4096 anchors, each with its in and out tangent offsets, in path order) and no items,
so exactly one of the two parts is present and no third payload model exists. It shares the
item slot, the `Stored | Payload` source and the 4 MiB ceiling. `clipboard.copy_mask_points`
names one item, one path mask and its anchor indices; a stale, duplicate or out-of-range
index, a missing mask or a non-path mask refuses, and an empty list stores nothing. Locks
permit Copy. `clipboard.paste_mask_points {item, mask, after, source}` inserts the copied
anchors in order directly after anchor `after` of an editable closed path, in place with
their tangents unchanged, as one undo step. It refuses an item snapshot, an open or short
path, an index past the last anchor, a locked item, and a result over 4096 anchors. Item
Paste refuses a mask-point snapshot and the reverse. Stored-empty Paste is a no-op.

Mask-point cut is `clipboard.cut` with `CutTarget::MaskPoints { item, mask, points }`:
`Session::cut` captures with `ItemClipboard::capture_mask_points`, applies
`Command::RemoveMaskPoint` inside the same `transaction`, then stores the payload through
`Session::store_item_clipboard`, so like the items arm the payload is stored only after the
removal commits. `Session::can_cut` admits it through `Session::can_copy_mask_points` on an
unlocked track.

### Custody and transports

The Session holds one immutable Arc snapshot outside project/history/persistence.
Changed Copy advances the control revision and ends merging, with `Outcome.changed`
false and no project event or dirty save. Identical Copy is a no-op. Receipt replay
never reinstalls an older snapshot. Checkpoints restore clipboard custody on refusal;
Copy/read refuse transactions and batches, while Paste supports atomic batches/groups.
Undo/redo and same-scene edits preserve the store. Successful effective project/scene
identity changes, including acknowledged Save As, clear it at committed completion;
failed or staged changes preserve it. New sessions and other windows start empty.
Read returns the bounded keyframe payload only in its response, never subscription
events. The clipboard has no system-text/image or media-file IO capability; text stays
on the OS clipboard and media-file paste belongs to the import owner.
