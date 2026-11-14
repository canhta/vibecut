# Runtime

Interface: `jobs`, `cache`, `ByteBudget`, `submit_export`, `submit_export_publication`, `submit_sequence_publication`.

Single- and multi-scene export submission, completed-export publication, subtitle mapping
and the Session export entrypoints implemented in this execution owner return `runtime::Result`.
Their jobs retain the actual Runtime producer through one error erasure; mixed higher
feature/control/app workflows compose it at their own boundary. Render and Media keep their
original source wrappers, and genuine Kernel Edit/Session/Control/Project/Captions causes
compose through Runtime Edit without changing classification. Foreign render hooks retain
their render results; only direct Runtime cancellation is terminal cancellation. Publication
writers in these owned jobs return Runtime results, while the generic publication operation
continues to preserve genuinely mixed caller errors unchanged.

## Default cache location

`cache::CacheRoot::discover()` selects the per-user OS cache base using native environment
paths: macOS `HOME/Library/Caches`, Windows `LOCALAPPDATA`, other systems `XDG_CACHE_HOME`
when present, otherwise `HOME/.cache`. A missing or nonabsolute selected base returns
kernel `engine::EditError::Unsupported` composed through `RuntimeError::Edit` with safe static context. A present empty or relative XDG value
refuses rather than falling back to HOME. Discovery performs no filesystem IO, creation,
download, eviction or migration; Debug redacts the location.

The named paths preserve the existing layout: `models()` is `<base>/VibeCut/models`,
`fonts()` is `<base>/VibeCut/fonts`, `waveform()` is `<base>/VibeCut/models/waveform`, and
`thumbnails()` is `<base>/VibeCut/models/thumbnails`. These defaults do not reinterpret
caller-injected `ArtifactCache` or `MediaAccess::open_in` roots. Cache consumers retain
their existing storage, budget and cleanup policy.

## Host-sized default budget

`ByteBudget::for_host(&dyn MemoryProbe)` is the one default every adapter opens with: the
desktop app (unless the user picks a cache limit), the CLI and, through them, MCP. The limit
is `clamp(min(total / 4, available / 2), 1 GiB, 8 GiB)`, read once at start.

- Physical memory is the stable part: a quarter leaves the OS, the GPU, decoders and other
  apps the rest, and the same host gets the same budget every launch. Available memory only
  lowers it, so launching under pressure does not push the host into swap.
- The 1 GiB floor is what a 4K single-layer export declares (about 1005 MiB), so it is never
  refused on a small host; the 8 GiB ceiling matches the settings maximum, since more adds
  no measured export speed.
- Rows (total/available): 8 GB/4 GB gives 2 GiB, 8/2 gives 1 GiB; 16/10 gives 4 GiB, 16/2
  gives 1 GiB; 64/50 and larger give 8 GiB. An unreadable probe (zero) gives the floor.
- `SystemMemory` reads macOS, Windows and Linux through `sysinfo` (no `unsafe` of ours);
  tests inject a `MemoryProbe`.

## Cache result boundary

Cache location discovery, artifact keys/storage, native model leases/state and maintenance
return `runtime::Result` directly. `CacheMaintenanceReport::cause` retains the original
`RuntimeError`, including partial work and cancellation; mixed workflows compose through
`engine::control::Error::Runtime` only at their own boundary. Resource IO/Limit/Invalid and cache
failures use their direct Runtime variants. Project atomic-write and unsupported capability
failures preserve their lower Kernel cause through transparent `RuntimeError::Edit`.
Native model metadata validation and SHA verification retain `ai::Error` in the consumed
`RuntimeError::Model` source wrapper: the static `AI execution failed` diagnostic, `Ai`
code, no transience and ordinary scheduling policy. Verification cancellation remains that
opaque model error, distinct from direct Runtime cancellation. Other AI workflow producers
retain their own policies.

## Verified transfer result boundary

Verified response and header operations return `runtime::Result`. Retryable GET failures
normalize to direct `NetworkTransient`, without inheriting an AI retry-after advisory.
Nontransient GET and redirect-validation failures retain their original `ai::Error` in
`RuntimeError::Transfer`, with `AI execution failed`, no transience and ordinary policy.
Its code borrows the one foreign AI projection in the verified-transfer execution owner:
Chapters is Refused, other AI errors are Ai. The engine aggregate's genuine AI composition
also borrows that projection's transience and direct rate-limit advisory; native `Model`
validation remains opaque Ai. Foreign cancellation is not direct Runtime cancellation.
Higher model/font workflows compose Runtime failures at their own mixed result boundary;
redirect/header bounds, consent, checkpoint and publication policies stay with their owners.

## Background jobs and byte policy

`runtime::Error` / `runtime::Result` own budget, artifact reservation and model accounting
failures, cache ownership contention and unusable cached artifacts; `RuntimeError` is the engine re-export.
`CacheBusy` owns cache writer-lock, native model pin/lease and active-transfer contention:
its original static diagnostic is `cache is in use`, with code `Refused`, no source and no
transience. Cache, provider and sound-download callers compose it through the one transparent
`engine::control::Error::Runtime` wrapper. It does not authorize automatic retry, reclaim or fallback;
explicit `Failed` wrappers keep the refusal code and remain nontransient.
`CacheCorrupt` is source-free, nontransient and classified as `Job`; its static
message does not authorize fallback. Each artifact consumer owns whether corruption
means recompute, a failed display or refusal; explicit `Failed` wrappers do not
change that policy. `QueueFull` owns pending admission refusal
and `WorkerPanicked` owns worker-unwind diagnostics; both are nontransient and source-free. `ByteBudget`, artifact pin/evict and the
runner's private reservation return this closed result. Higher feature, job and Session operations
compose it through transparent `engine::control::Error::Runtime`. IO keeps its owned source. `NetworkTransient` owns retryable
transport failures: static message `transient network failure`, `Job`, source-free
and transient only as a direct runtime variant. It is the `transport` crate's
`Network` category; its `Invalid` and `Payload` categories map onto the existing
`ai::Error` variants retained by `Transfer` without a new runtime variant. All other runtime variants remain
nontransient. `ModelResponse` owns unusable model-transfer representations and
`StorageFull` owns model-storage refusal; both have static diagnostics, no source
and code `Refused`. Transfer and cache consumers compose them upward without
changing their refusal, checkpoint or publication policy. `Failed` shares a typed erased cause and delegates its code,
without inheriting transience or adding a source-chain link. Only `BudgetFull` permits reclaim or queued waiting;
`ArtifactTooLarge` is immediate refusal even though both publish the `budget` wire code.
Task closures return their declared `Failure` owner through generic `submit` and
`submit_for_session`. The task adapter erases each returned error once into its actual
`Arc<dyn Failure>`; execution, retained snapshots and coalesced events share that Arc.
Existing higher workflows declare their composed `engine::control::Error`; runtime-generated
cancellation, unwind and reservation failures remain direct runtime errors.
Snapshots retain the raw
`Arc<dyn Failure>` after typed cancellation/retry decisions; only explicit failed-workflow
propagation adds a `RuntimeError::Failed` envelope.

`jobs::JobRunner::new(config, budget, clock)` schedules blocking adapters on std threads.
`submit(spec, task)` returns a `JobId` and `CancelToken`; `tick()` joins finished workers,
admits queued work and observes progress, and the application calls it from its event loop.
`memory_budget()` lends that same aggregate policy to blocking composition stages; they
reserve before invoking adapters and retain leases for their allocation lifetime.
`JobContext` implements `ai::ExecutionHooks`; other domain owners receive its cancellation and
progress functions through their own hooks. Engine disables ai's default native feature:
composing an adapter is an app choice.

The task closure returns complete bytes or its declared typed failure with retained sources. It
must not apply commands or publish partial files. Independently complete ASR chunks may be
committed as checkpoints inside a task. Executing closures run inside `catch_unwind`; OS thread
creation failures are returned. `shutdown` and Drop cancel and join all active workers.
Adapters must check cancellation and put finite timeouts on blocking IO; a noncooperating
adapter can delay shutdown, and abort or OOM cannot be recovered.

`ByteBudget::child(limit)` adds per-project admission while retaining leases against the parent
app policy. Admission refuses on either limit and rolls back partial reservations; releasing a
child lease releases aggregate accounting. One shared budget is cloned into every owner and its
snapshot publishes total used/limit and per-owner bytes. `reserve(owner, bytes)` returns a
lifetime-bound lease, and owners reserve before allocating, including overlapping batches and
actual pixel storage. On byte pressure every reservation reclaims unpinned persistent artifacts
in one aggregate LRU order; oversized reservations fail before eviction, and pressure from active
work and readers can defer queued jobs until their leases release. Reclamation unlinks files
synchronously outside accounting and registry locks, so reserve before execution and never in an
audio callback. Unlink failures return typed IO errors and leave charges intact for a later
retry. Atomic reader pins exclude in-use files and a read's requested entry. `used` includes
every category; `limited_used()` excludes model bytes, which free-disk preflight admits. Model
files stay pinned and cannot starve ordinary work admission. Reservations are declared
allocation ceilings, not an allocator sandbox or RSS measurement; captured task data is
caller-owned and must be bounded. The preferences owner supplies the aggregate limit.

Scheduler configuration, submission, tick, cancellation and JobContext progress return
`runtime::Result` directly. Higher feature/control/app boundaries compose their Control result
through ordinary From. `JobNotRetained` owns absent/evicted jobs and missing completed data,
with the original static diagnostic, its existing wire code, no source or transience.
`IdExhausted` owns only the checked runtime job counter and classifies as Refused; edit/domain
ID allocation remains separate. Neither failure authorizes regeneration or retry.
`commit_publication` returns its caller's error type via `E: From<RuntimeError>` for seal and
progress-lock failures; the synchronous publish callback's own error passes through unchanged.

`RunnerConfig` admits 1..=16 workers per CPU/GPU-encoder/network class, 1..=10,000 pending jobs,
1..=10,000 terminal results and 1..=10 network attempts. Admission checks the sum of declared
work and maximum result bytes against the aggregate, and a full queue returns `RuntimeError::QueueFull` through the composed Control result.
Resource- or byte-blocked tasks stay queued while other classes run. Result memory is reserved
before execution and the returned `Vec` capacity must fit that reservation; terminal results keep
their reservation until taken, dropped or evicted oldest-first, and consumers must handle an
evicted ID as absent. Outputs taken by consumers keep accounting leases until dropped.

`drain_events()` is a coalescing event channel: one latest snapshot per admitted or retained job,
bounded by pending jobs plus configured concurrency plus terminal retention. Intermediate events
can be replaced; this is not an audit log. Worker progress is finite 0..=1 and accepts at most
one update per configured interval (minimum 10 ms, including stage changes); `tick` forwards only
changed snapshots, and terminal and cancelling states bypass the limit. Slow consumers cannot
block workers.

`Cancelled` owns the source-free, nontransient `job cancelled` diagnostic and
`Cancelled` wire code. `JobContext::check_cancelled` and `begin_publication` return
the runtime result; ordinary engine callbacks compose it through `engine::control::Error::Runtime`.
Direct runtime cancellation and its transparent root composition publish terminal
cancellation through the local `Failure::job_policy`. Explicit
`Failed` and recovery persistence wrappers retain their original causes and remain
Failed, even when their delegated code is `Cancelled`.

Cancellation is checked before execution and again before promoting success; late partial
success is discarded. `JobContext::commit_publication` atomically seals cancellation before
publishing an irreversible complete artifact and sets the Publish stage. A cancellation that
wins the seal prevents publication; once sealed, late cancellation is ignored and a transient
publication error is not retried. The closure must do its final cancellation-aware validation
before sealing, and failure never yields a `JobOutput`.

`Transcribe`, `Translate`, `SuggestClips`, `GenerateChapters`, `ClassifyFillers`,
`DownloadModel`, `SoundDownload`, `DownloadFont` and `DownloadSticker` jobs retry direct `RuntimeError::NetworkTransient` or its transparent root composition, or an
`ai::Error` whose `is_transient` is true (network, provider server, rate limit or structured
schema failure). An `ai` rate limit carrying
`Retry-After` raises that attempt's backoff, capped at five minutes. Auth, quota, payload,
invalid, cancelled and all other errors, and local, preview, export and import jobs, run once.
Retry attempts are bounded by `RunnerConfig::network_attempts` with configured exponential
backoff through an injected monotonic `Clock`; workers and bytes release during backoff. If the
pending queue filled while the attempt ran, retry is refused and the typed transient failure is
kept. The closure must revalidate request consent and aggregate monetary authorization on every
attempt; the runner never grants consent or performs automatic provider fallback.

`submit_for_session(session, spec, task)` binds its `JobOutput` to the original Session and
revision; `output.apply(session, command)` returns the Kernel Edit result using that identity.
Completed-output validation returns the Runtime result: absent retained output is
`JobNotRetained`, while missing or stale Session targets retain Kernel Session `StaleResult`
through Runtime Edit composition. `Session::job_target()` captures
the identity for externally adapted work, and `apply_job_result(target, command)` refuses stale
identity and active previews or groups before using the normal atomic command boundary. Every
changed edit, undo/redo and changed preview invalidates older targets, even if undo restores
equal content, as does every group or gesture squash and transaction rollback. `saved()` refuses
during a transaction and invalidates pre-save targets. Opening another Session gives a new
identity; a valid old artifact may stay cached without being applied.

## Render execution

The Runtime rendering composition implements the local Failure contract for `render::Error`:
Unavailable maps to the existing Unavailable code, every other variant to Render, with ordinary
nontransient/default scheduling policy. This is the one foreign render projection consumed by
both the engine aggregate and durable LUT store/publication guards. The same execution
owner implements the Media projection once (`Media`, ordinary defaults), consumed by root,
managed Library and Runtime. Kernel learns neither foreign execution projection.


`loudness_admission::admit` (public as `analysis::loudness::admit`) returns the runtime result directly. `LoudnessStale` refuses missing or
mismatched measured gains with code `Refused`, no source and no transience. Export
submission composes this error before admitting any single- or multi-scene job;
no target needs no measurement. The content key excludes the master fader.

`footage_admission::admit` refuses `CorrectionMissing(items)` (code `Refused`) when a visible video item of the exported scene has stabilization switched on and the export's `RenderSettings.stabilization` holds no track for it, background removal switched on and `RenderSettings.person_mattes` holds no matte for it, or flicker removal switched on and `RenderSettings.flicker` holds no track for it. It runs beside the loudness admission in every single- and multi-scene submission, so an export never differs silently from a preview that drew the correction; the error names the item ids.

`submit_export` submits an immutable Project snapshot to the runner's export resource class.
`JobContext` implements render's `ExecutionHooks`, leasing actual buffers from the shared
`ByteBudget` under decoded, compositor and export owners rather than declaring a second
whole-job charge. Render owns pixels, audio and output policy; the job never applies editing
commands. Completion returns an empty `JobOutput` after the complete file has been atomically
published.

`ByteBudgetHooks::new(budget, cancel)` is the one adapter from render's memory kinds to the
aggregate owners and owns no policy of its own; JobContext delegates its reserve to it, and cover
images, snapshots, effect previews and transition thumbnails construct it. Refusal keeps an IO
source or reports `Budget` with the real remaining bytes, so render's conversion loop can lower
its worker count before failing; the cancellation probe is optional.

Stages expose admission, rendering, mixing, encoding, finalization and publication. Cancellation
uses the [shared publication fence](staged_publication/README.md#completed-export-publication).
Crash and power-loss durability are not established.

`submit_export_with_sources` is the export job with an injected render source factory,
constructed on the engine worker; `submit_export` supplies `FileSources`. Both share one
scheduler, byte accounting, typed result and publication fence.
`Session::execute_export(runner, key, Command)` submits one already-admitted export command;
callers never decode JSON at this layer.

`submit_clip_export` is `export.clip`: one export job over the clip's span from
`clip_export_range`. `Duration` renders the whole project over that span. `Clip` renders a
snapshot whose timeline holds only the clip (other items emptied, transitions cleared), so
its pixels and audio are the clip's own. The session is never edited.

## Render kernel execution

`JobContext` supplies render's scheduling hook: up to eight workers, capped by available
parallelism. Each row batch borrows disjoint buffers, runs one task on the job thread and uses
scoped engine workers for the rest. It reserves 512 KiB per extra worker stack before spawning,
runs sequentially when the extra stacks do not fit, catches worker panics as typed render
failures, and joins all tasks on success, cancellation, refusal and unwind. There is no detached
work or retained pixel pool; default hooks outside engine are sequential. Each job can have at
most seven additional row workers during a batch. Native FFmpeg conversion gets the same
concurrency bound and holds its separately charged worker allowance for the converter/encoder
lifetime.
