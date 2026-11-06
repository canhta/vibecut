# Providers

Interface: `Providers`, `Credentials`, `ConsentPrompt`, `provider_options`, `model_info`, `transcribe`, `translate`, `suggest_clips`, `models`.

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

`cargo test -p engine --test it providers::` observes no-fallback defaults, ordered
provider failures, renewed approval before any fallback upload, rejection of
primary-token reuse, refusal/cost/cancellation and exact provenance. Offline
injected adapters establish execution policy, not third-party model quality.

Clip provider outputs preserve AI-owned ordered source segments, resolved source-order
ranks and summed duration; consumers use the [AI clip contract](../../../ai/src/clips/README.md#transcript-clip-suggestions).

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
cannot publish. HTTPS redirects (followed by `runtime/`'s verified transfer, which font downloads share) are bounded to five, forbid credentials/fragments
and keep signed target URLs ephemeral. Hash/size checks reuse ai's streaming owner.
The shared client has finite timeouts; cancellation during a blocking request/read
can wait for the response or timeout. No live remote throughput or Windows runtime
claim follows from offline integration tests. Weights are never Git or installer
content; actual model-manager UI and CLI composition remain downstream work.
