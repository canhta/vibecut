# Runtime

Interface: `jobs`, `cache`, `ByteBudget`, `submit_export`, `submit_export_publication`, `admit_loudness`.

`Cost::add` accumulates provider estimates with the existing saturation and currency
rules. `JobRunner::batch_capacity` bounds downstream batches by resource concurrency
and retained terminal results, keeping scheduling policy inside engine.

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

`ByteBudget::child(limit)` adds per-project admission while retaining leases
against the parent app policy. Admission refuses on either limit and rolls back
partial reservations; releasing a child lease releases aggregate accounting.
Artifacts registered by a child participate in the parent's aggregate LRU using
one root recency clock. Model disk reporting also propagates through child
leases. Separate root policies reserve persistence headroom independently from
export/preview memory at the app composition boundary.

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
second time by its adapter. The preferences owner supplies the aggregate limit, rather
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

## Render execution

`submit_export` submits an immutable Project snapshot to the real runner's export
resource class. JobContext implements render's ExecutionHooks, leasing actual
buffers from the shared ByteBudget under decoded/compositor/export owners rather
than declaring a second whole-job charge. Render owns pixels/audio/output policy;
the job never applies editing commands. Completion returns an empty JobOutput
after the complete file has been atomically published.

Stages expose admission, rendering, mixing, encoding, finalization and publication.
Cancellation uses the [shared publication fence](../runtime/staged_publication/README.md#completed-export-publication).
`tests/it/runtime/export_jobs.rs` verifies both sides of the fence with real production
render hooks, worker unwind, persistent-cache reclamation and oversized-frame
refusal before allocation. This does not establish crash/power-loss durability.

`submit_export_with_sources` is the existing export job with an injected render
source factory, constructed on the engine worker. `submit_export` supplies
`FileSources`; desktop tests supply gated sources to order cancellation/failure.
Both share one scheduler, byte accounting, typed result and publication fence.

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
pixel and codec ownership. `tests/it/runtime/render_kernels.rs` covers failure joins, stack
admission, released charges and parallel/sequential pixel equality.
