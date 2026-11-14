# Control

`color.import_lut` borrows the process media access only long enough to clone the durable store, then submits a bounded CPU job without a lock across IO. Its descriptor is `JobOutput::Lut`; a later Human `color.register_lut` edit plus ordinary grade/reference/strength edits consumes it. Agents cannot import arbitrary source paths. Registered-reference edits retain Agent policy and ordinary revision/replay/transaction guards. The [copied-resource contract](../store/README.md#imported-lut-bundles) owns publication, quota and relink.

Interface: `build_request`, `run_wire`, `Caller`, `MediaAccess`, `Policy`, `Session::control_request`,
`Session::control_typed`, `Session::control_command`, `snapshot_and_subscribe`, `JobOutput`; `Caller.fonts` and
`Caller.sounds` lend the process asset libraries and `Caller.chapters` lends the process
`TextAccess`; `Caller.captions` lends [caption workflow resources](../subtitles/README.md#caption-workflow-commands).

`Error` and `Result` belong to this top composition owner; the engine root only wires
lower interfaces and exposes no aggregate error facade. The aggregate retains each existing
owned cause, safe diagnostic, source and delegated policy. Lower Kernel/Runtime/features
never depend on it; natural jobs retain their own producer types. Heterogeneous publication,
font/sound callbacks and trusted Policy callbacks use this mixed result. The access-presence
guards themselves return the existing FontError or Kernel EditError.

`MediaAccess::source_availability(asset)` answers whether one source file is still on disk with a single metadata read (no decode, no hashing) and returns the media crate's `AssetAvailability`; a missing file is the relink state, and an unreadable path is a typed `Media` error rather than `Missing`.

`MediaAccess` setup composes Runtime caches and the durable LUT store; maintenance composes
Runtime resources with the protocol entry bound. Waveform and scene generation/display return
Analysis results, and thumbnail generation/display return Artifact results. Its finisher
composes those features. `snapshot_and_subscribe` returns only Kernel `SessionError`; its
transaction refusal precedes observation and leaves state and runner events unchanged.

## String entry

`run_wire(session, id, params)` is the only function that takes a command id string. It decodes
`id` and `params` against the registry schemas (`decode.rs`, private to `control`), then applies
the command with wire validation. Its callers are transports and tools that hold only a typed id
and JSON: the CLI `exec` subcommand and the wire-semantics tests. `Session` has no method that
takes a `&str` command, and `Autosave` routes only typed lifecycle commands.

`Session::control_command(command, caller, runner)` is the in-process control entry for a caller
that holds a `Command` (the font picker, the timeline visuals, the LUT import). It keeps `Policy`
authorization of the whole command tree and the lent access objects, and skips what only a
transport needs: request ids, registry hash, base revision, idempotency receipts and response
events. A refusal returns its typed cause.

## Forks

A [`Fork`](../session/README.md#forks) is a run target: `Fork::control_request` and
`Fork::control_command` are the live session's entries over the fork's session, with the same
admission, `Policy`, lent access objects, receipts and events. A request names the fork's
`session_id` and revision (`build_request(fork.session(), ..)`); one addressed to the parent
refuses `SessionGone`. The fork's runner and jobs are the caller's; the log records only the edits.
Applying is `Session::apply_fork`, which takes no `Caller`.

## Control admission, revisions and receipts

`control::{Request, Caller, Policy, Response}` is the transport-neutral boundary; the envelopes,
limits and digest it exchanges are the kernel's [wire contract](../session/wire/README.md#wire-contract),
which `control` re-exports. `Session::control_json` parses a frame under those limits;
`control_request` also bounds already-allocated requests. Unknown commands, wrong types or fields
and nested batch publications refuse before effects. Effectful calls require the exact digest and
version; catalog reads need no digest.

`ControlError` is the kernel wire owner's typed refusal, re-exported here.
`build_request`, `Session::check_receipt_capacity`, `replay_receipt` and `events` return it directly;
`control_fingerprint` and `store_receipt` also return it directly. Mixed workflows
compose it through this owner’s transparent `control::Error::Control`.
`Policy::authorize` retains its heterogeneous `control::Result` so trusted policies can
report their actual owner failures.

One admission step, `Session::admit`, runs before dispatch for `control_request`, `control_typed`,
`control_command` and the lifecycle save: it applies the declaration's availability (the
`idle` open-transaction rule, then the `Availability` condition) and its declared collection
limits, and refuses with a typed `SessionError` (`TransactionActive`, `NoGesture`, `NoHistory`
or `Unavailable`) or `ControlError::Limit`. No arm of the dispatcher repeats those checks; the
UI reads the same rule through `CommandSpec::enabled`.

Effectful requests require `base_rev`; a revision that is not current refuses with
`StaleRevision` before any child of a batch runs (revision rules:
[wire](../session/wire/README.md#wire-contract)).
An invalid candidate returns InvalidParams without consuming IDs, advancing revision or emitting
a project change; its refusal receipt can be replayed.

`Session::control_typed` is the in-process adapter entry: the same bounds, admission, receipts
and events as `control_request`, but an execution refusal keeps its typed cause so the adapter
can map the exit codes its own transport documents. These mixed methods use
`control::Error` / `control::Result`; pure Session command failures compose through its `Edit` variant,
while direct control/runtime/provider refusals keep their actual owner composition. Wire callers use `control_request` and see
only the closed `ErrorCode`.

Session and project lifecycle creation and disk saves belong to `Session::create`/`Autosave`;
direct Session edit and control execution return a typed refusal for those commands, and their
registry examples declare those routes. A composition root that owns the project file may call
`Session::control_lifecycle_save`: it applies the same version, hash, session, policy,
request-ID, key, revision, capacity and receipt rules as other effects, then runs a closure that
performs only project and metadata publication, acknowledges the save and stores the terminal
receipt. Success and typed execution errors are both receipted; a failed publication keeps the
composition root's dirty state, the same request replays the stored error, and a retry that must
execute again uses a fresh key.

`project.save_as {path, replace}`, `project.export_bundle {path}` and `project.import_bundle {bundle,
destination}` are Human lifecycle commands that need no composition-root lease, so
`control_request` runs them (`publish.rs`) through ordinary admission. Save-as writes a copy of the
committed project under a fresh identity with its view metadata. `replace` is required: an
existing destination is refused unless it is true, in which case the writer lease replaces it
atomically; the session's protected paths (its own project and sidecar, aliases included) are
refused either way. The open project, its lease and its history are untouched. The bundle pair calls
[project bundles](../../../project/src/persistence/README.md#bundles): export copies media and
imported LUT bundles; import extracts into a new directory. All three return
`ControlResult::Published`; save-as and export refuse while a gesture is open.

The composition root supplies `Caller` and a mandatory trusted `Policy`, outside client JSON.
Engine invokes it on the command and every batch child before effects and before receipt replay.
It enforces source, destination, overwrite and provider consent through their owners; it is not
an authenticated transport. External callers cannot begin, commit or cancel gestures, including
through generic run or batches. External mutations refuse while the UI holds a gesture. Agents
use atomic batches and undo, and reads never expose uncommitted gesture snapshots.

Effectful requests require a 1..=128-byte idempotency key. Receipts are scoped by trusted
principal, Session and key; the fingerprint covers command, params, base revision and contract
identity, excluding transport and request correlation. Replay returns the original revision and
result even after later edits; conflicting semantics refuse. Mutable Session ownership
serializes concurrent callers. The ledger retains at most 4096 receipts and 32 MiB without
eviction until Session end; new effects reserve 128 KiB of available capacity before execution.
Mutation results have at most 1024 created IDs; returned publication paths are capped at 4096
encoded bytes. Request IDs and principals are 1..=128 bytes. Refusals before reservation do not
occupy a key; accepted execution errors do. A new Session refuses the old epoch rather than
rerunning an uncertain mutation. Inspection and catalog responses are capped at 16 MiB; oversize
reads refuse instead of truncating or mixing revisions.

Commands that need process state declare it. `fonts.*` needs the attached font store, the media
generation commands and their display reads need the attached `MediaAccess`, `sounds.*` needs
the attached sound library, and `chapters.generate`, `chapters.export_sidecars` and
`fillers.classify` and `captions.suggest_emphasis` need the lent
[`TextAccess`](../providers/README.md#transcript-text-generation) that owns the chosen adapters
and the human approval. A caller without them refuses `Unsupported` or `Permission` before any
input, adapter or network work. `MediaAccess::open` / `open_in` create the waveform and thumbnail
caches under the shared cache root; one instance is lent to the timeline and the MCP inbox so a
display never decodes media or builds a cache key. Export starts the production render job
against its admitted immutable snapshot and returns job ID, revision and paths; `preview` accepts
1..=4 exact output times, starts a CPU job and returns inline JPEG frames bounded by
`MAX_PREVIEW_EDGE`, which `job_status` retains without consuming.

## Job commands

`clips.suggest` and `clips.export` are declared here and implemented above the engine: `ClipWorkflow`
(`control/clips.rs`) is lent through `CaptionAccess::with_clips` because it shares that access's
providers, consent and checkpoint cache. `clips.suggest` returns `JobAccepted`; its `clips` output
decodes from the retained `SuggestClips` bytes. `clips.export {job}` is a `FilePublish` over a
directory, decodes the retained suggestion (it is bound to its asset, not the session revision),
reads cues from the optional caption `item` and returns `ClipExports`: one job per short, plus `shots`: `None` when the cached per-shot analysis of the selected pieces laid the shorts out, else a typed diagnostic (`detector_unavailable`, `models_missing`, `models_busy` while a model transfer holds the cache (retry after it ends), `pending` with the analysis job, `failed`) and every short stays a centred Fill. A missing analysis starts as a job; a later export reuses it.

A command that starts a job is `JobControl`, declares `grant` explicitly and returns
`JobAccepted { job, revision, paths }`. `job_status` returns a `JobView` whose single
`output` is a tagged `JobOutput` (`preview`, `pauses`, `chapters`, `fillers`, `emphasis`, `loudness`,
`scenes`, `captions`, `translations`, `reframe`) while the job is `completed`, and `null` otherwise. `control/output.rs` decodes it from
the runner's retained bytes with one exhaustive `match` on `JobKind`, so a new kind does not
compile until it names its output or states it has none. Reading does not consume the output; a
completed job whose bytes were evicted refuses `job_not_retained`.

- **Edits.** A job never edits the project over control. The caller reads `output` and sends an
  ordinary edit command with `base_rev` = the `revision` from `JobAccepted`; a changed revision
  refuses `stale_revision`. `audio.apply_loudness {gains}` is that edit for
  `audio.measure_loudness`, and `media.detect_scenes` candidates are applied by the split
  commands. When an output is too large to round-trip, the edit command names the finished job
  instead of carrying the data; it is still an ordinary edit with `base_rev`, and it refuses when
  the output was evicted or the target changed since the job started. `Session::apply_job_result`
  is for in-process owners only.
- **Reframe.** `reframe.generate {item,target}` submits one CPU `AnalyzeReframe` job over the
  item's source span through the lent `VisionAccess` and the media artifact cache. `job_status`
  returns a small per-shot report (`start`, analysed key count, fallback reasons) while the full
  analysis stays in the runner's retained output; `reframe.apply {job,replace_manual}` reads those
  bytes as one ordinary undo step, sets the target's static crop size and writes `crop.x`,
  `crop.y` and `crop.auto` for every shot that is not manual. A shot is manual where `crop.auto`
  is false; `replace_manual` is its explicit confirmation and `reframe.set_manual`/`reframe.clear`
  are the ordinary edits that mark one shot or remove the three channels.
- **Lent implementations.** Engine declares the command and its output type. A workflow that
  lives above `engine` is implemented by a crate above it and lent through one optional access
  object per workflow family in `Caller` (`fonts`, `media`, `chapters`, `captions`, `vision`). Absent, a local workflow
  refuses `Unsupported` and a remote-capable one refuses `ConsentRequired`.
- **Grants and consent.** Local workflows are `grant = Agent`. A remote-capable workflow is
  startable by an agent only when its access object already carries the human approval for that
  provider, scope and cost ceiling; client JSON has no consent field. Disclosure text and links
  stay with the access object's owner.
- **Inputs and outputs.** A command names admitted project assets or items by id, never a source
  path. A file output is a `FilePublish` command whose every path the trusted `Policy` authorizes.
- **Process caches.** `MediaAccess` also serves `media.detect_scenes`: it submits against the
  lent artifact cache and publishes the entry when `job_status` first sees the job completed.
  Loudness measurement takes no cache. `footage.analyze {item, kind}` is the same shape: one CPU
  `AnalyzeFootage` job over the item's source that edits nothing, published into the lent cache
  when `job_status` first sees it completed ([footage correction](../analysis/correction/README.md)).
  `export` and `preview` read the stored corrections of the switched-on items from that cache and
  hand them to render; `export` refuses with `CorrectionMissing` naming any item that has none.
  `kind: person_matte` needs the local segmenter the root lends with
  `MediaAccess::set_segmentation(SegmentationAccess)`; a cold matte without it, or without
  installed weights, refuses with the typed `ModelMissing`; `kind: flicker` needs no model.

## Asset-library commands

`sounds.search`, `sounds.save` and `sounds.unsave` are read-effect commands over the lent
`SoundAccess`: they never enter project history or advance a revision, and a caller without the
access refuses `Unsupported`. The access object owns the library root and the provider
credential, so a command carries neither — no path and no token enters its parameters. It
chooses remote search when the credential is present and degrades to the saved local library
without it. All three are `grant = Human`, so an agent transport refuses them and a human-owned
composition supplies the one library root. `sounds.save` is an idempotent upsert and
`sounds.unsave` an idempotent removal, so replaying either is safe.

`stickers.search` and `stickers.insert` run over the lent `StickerAccess` (a search function and an id resolver). Both are `grant = Agent`. Search is a read; insert resolves the id to its entry, then applies one ordinary `InsertItem` with `ItemParams::Sticker` (one undo step), so the kernel stores only the canonical id and the intrinsic size the catalog reports and holds no sticker data. `stickers.search` takes a category of `all`, `emoji`, `flags` or `icons`; the lent search and resolver cover flags and every fetched collection (resolving an id does not need the collection downloaded; rendering refuses it until it is). A caller without the access refuses `Unsupported`; the CLI `control` session and the desktop and headless MCP inboxes lend the catalog.

## Control events and resynchronization

Command replies and receipts are authoritative; large read and image results stay on the response
path. Project changes require a full snapshot, since created IDs are not a complete diff. Gesture
previews and cancellation emit revision changes, never committed project changes.

`snapshot_and_subscribe(&runner)` captures committed project, revision, current jobs and a cursor
while Session ownership is held. The owning loop calls `observe_jobs` after runner ticks and
accepted responses; it uses retained snapshots without draining other consumers. Acceptance is
visible before job lifecycle events. At most 128 small live events are retained for all listeners
together; cursors hold no queue. A slow or foreign cursor gets ResyncRequired and must install a
fresh snapshot. No durable replay or exhaustive transition log is promised; progress can
coalesce. Terminal snapshots follow the runner's publication fence, and repeated polls do not
duplicate unchanged terminal events. Dropping a cursor never cancels work.

Refusal responses, receipts, events and job views consume the error owner's `Failure::code`.
They keep only the safe category; typed in-process causes and static messages remain intact.

`MediaAccess::cache_maintenance` forwards snapshots of exactly its stores (waveforms and scene
analysis, thumbnails and, when attached, preview proxies), refusing more
than 256 ordinary entries in total. It neither opens other roots nor adds a control command;
the desktop releases the access mutex before running maintenance IO.

The Agent-granted `clipboard.copy`, `clipboard.copy_keyframes`, `clipboard.copy_mask_points`,
`clipboard.cut`, `clipboard.paste`, `clipboard.paste_keyframes` and
`clipboard.paste_mask_points` use ordinary
revision/receipt admission. `clipboard.read` returns the same Session's
nullable bounded keyframe snapshot without a receipt. Replay leaves a newer store intact;
snapshot events omit it.
Transfer/custody rules: [clipboard](../session/README.md#clipboard).
