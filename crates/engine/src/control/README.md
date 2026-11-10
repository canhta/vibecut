# Control

Interface: `build_request`, `Caller`, `MediaAccess`, `Policy`, `Session::control_request`,
`snapshot_and_subscribe`; `Caller.chapters` lends the process `TextAccess`.

## Control admission, revisions and receipts

`control::{Request, Caller, Policy, Response}` is the transport-neutral boundary; the envelopes,
limits and digest it exchanges are the kernel's [wire contract](../session/wire/README.md#wire-contract),
which `control` re-exports. `Session::control_json` parses a frame under those limits;
`control_request` also bounds already-allocated requests. Unknown commands, wrong types or fields
and nested batch publications refuse before effects. Effectful calls require the exact digest and
version; catalog reads need no digest.

Effectful requests require `base_rev`; a revision that is not current refuses with
`StaleRevision` before any child of a batch runs (revision rules:
[wire](../session/wire/README.md#wire-contract)).
An invalid candidate returns InvalidParams without consuming IDs, advancing revision or emitting
a project change; its refusal receipt can be replayed.

Session and project lifecycle creation and disk saves belong to `Session::create`/`Autosave`;
direct Session edit and control execution return a typed refusal for those commands, and their
registry examples declare those routes. A composition root that owns the project file may call
`Session::control_lifecycle_save`: it applies the same version, hash, session, policy,
request-ID, key, revision, capacity and receipt rules as other effects, then runs a closure that
performs only project and metadata publication, acknowledges the save and stores the terminal
receipt. Success and typed execution errors are both receipted; a failed publication keeps the
composition root's dirty state, the same request replays the stored error, and a retry that must
execute again uses a fresh key.

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
rerunning an uncertain mutation. Inspection and catalog responses are capped at 8 MiB; oversize
reads refuse instead of truncating or mixing revisions.

Commands that need process state declare it. `fonts.*` needs the attached font store, the media
generation commands and their display reads need the attached `MediaAccess`, and
`chapters.generate`, `chapters.export_sidecars` and `fillers.classify` need the lent
[`TextAccess`](../providers/README.md#transcript-text-generation) that owns the chosen adapters
and the human approval. A caller without them refuses `Unsupported` or `Permission` before any
input, adapter or network work. `MediaAccess::open` / `open_in` create the waveform and thumbnail
caches under the shared cache root; one instance is lent to the timeline and the MCP inbox so a
display never decodes media or builds a cache key. Export starts the production render job
against its admitted immutable snapshot and returns job ID, revision and paths; `preview` accepts
1..=4 exact output times, starts a CPU job and returns inline JPEG frames bounded by
`MAX_PREVIEW_EDGE`, which `job_status` retains without consuming.

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
