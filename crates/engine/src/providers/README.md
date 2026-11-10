# Providers

Interface: `Providers`, `Credentials`, `Error`, `ConsentPrompt`, `TextAccess`, `TextProvider`,
`provider_options`, `model_info`, `transcribe`, `translate`, `suggest_clips`, `models`.

## Failures

`Error` owns setup, capability and consent refusals: a missing local model or Gipformer runner, a
missing provider credential, auto-detection on a provider that is not local whisper.cpp, and a
remote upload without consent. Display states the fact only, never a command to run: the CLI adds
its `vibecut-cli model download <id>` hint and the desktop renders its own text from the variant.
Callers that still return the kernel union wrap it as `EditError::Providers`; the wire maps consent
to `Permission`, missing setup prerequisites to `Setup`, and unsupported auto-language to `Refused`.

## Provider selection and fallback

`providers::{transcribe, translate, suggest_clips}` execute an AI-owned `ProviderSelection` on
the job worker with its `ExecutionHooks`. A caller that configures a local fallback takes a CPU
resource reservation for the whole run, so switching from network to inference cannot bypass CPU
concurrency limits. Every attempt must exactly match the persisted feature/provider order; no
adapters are inferred, and each adapter is invoked at most once. A successful attempt ends the
run. Auth, quota, network, server, rate-limit, schema and incomplete-response failures advance
only to the next configured provider; cancellation, consent, cost, input and capability errors
stop immediately. Exhaustion returns the last typed failure.

Composition implements `providers::Approval` to show provider identity, its policy link, exact
payload and cost at setup and again immediately before a fallback executes. The callback
receives `fallback=true` for the renewed prompt; a remote attempt without its own exact consent
token cannot upload. Approval errors stop the run. Clip windows are all approved before a
provider's first IO, and partial results from a failed provider are discarded. Local heuristics
are an explicitly configured `ClipSuggester`, never an implicit recovery path.

`ProviderOutput` keeps the exact successful provider configuration beside its output, and
serializable outputs preserve this provenance in job result bytes. Callers store the complete
envelope and include the entire selection and request settings in cache keys. Recognition keeps
its sample-coordinate model, including text-only results when timing is unavailable. The
functions do not mutate projects; translation results go through the registered edit command.
Clip provider outputs preserve AI-owned ordered source segments, resolved source-order ranks and
summed duration; consumers use the [AI clip contract](../../../ai/src/clips/README.md#transcript-clip-suggestions).

## Transcript-text generation

`TextProvider` is the user's explicit transcript-text provider and model for the chapters and
fillers operations, listed by `provider_options` and resolved by `choose_chapter` /
`choose_filler`; there is no fallback, and the catalog constants in
[`catalog.rs`](catalog.rs) own the model ids. `TextAccess::open(provider, credentials, approval)`
builds both the chapter adapter and the filler classifier once; `TextAccess::new(chapters,
fillers, approval)` lends already built ones. Composition roots construct one access per process
and lend it through `control::Caller.chapters`; the desktop shares it with its MCP inbox with
`Inbox::attach_text`.

`TextAccess::prepare_chapter(input)` and `prepare_fillers(input)` bind the complete canonical
transcript, the provider configuration, the operation and a bounded cost estimate into an
`ai::Scope`, then ask the composition's `Approval` for the exact token. `Approval` is where the
human decision lives: a prompt shows provider, model, payload and cost; `StoredConsent::grant`
records already-given human approvals and only returns a token for that exact configuration,
leaving a higher estimate to the token's cost check; no token means `ConsentRequired` and no
adapter call. The access never constructs consent on its own and never retries or switches
provider. Both operations share the `ai::text_model` request path: one official backend check,
one bounded body and one response parser; they differ only in prompt, schema, settings
fingerprint and result validation.

## Local model transfers

`models::ModelTransfers` composes JobRunner, ArtifactCache and ai's shared streaming transport.
`open_default` uses [runtime's `CacheRoot::models()`](../runtime/README.md#default-cache-location);
unavailable discovery returns runtime's `EditError::Unsupported`. `new` accepts an already owned cache and injected
`ModelIo`. The runner must share that cache's ByteBudget. There is one active transfer per cache,
including queue and backoff; state mutexes cover ownership changes only, never network, file IO or
verification.

`submit` requires ai's exact-model `DownloadConsent`, reauthorizes every attempt, and reports
Download/Verify/Publish stages through job events. The app's model manager owns showing source,
licence and size and obtaining the affirmative click before granting consent; provider selection
alone cannot authorize download. Offline `installed` lookup needs no download consent and
verifies all named files before returning a `ModelLease`. Retain that lease for the entire native
adapter lifetime, including lazy reads by the Gipformer child: its root lock and reader pin
prevent another writer or explicit deletion while paths are in use.

A ModelBundle is a native entry kind inside the same cache: a content-key directory contains
immutable catalog-named raw files and `.bundle` metadata. Only an atomically published
verified-complete marker permits a `ModelLease`, so partial checkpoints stay unavailable to
loaders. Models and checkpoints are pinned against LRU and generic clear; only an explicit
`remove` deletes them, and live leases refuse removal. Uncheckpointed tails are reclaimed before
append or restart and credited during a free-disk preflight; a disk-full write still returns
typed `StorageFull` when availability changes after the advisory check. Unknown directories and
symlink targets are never adopted or overwritten.

Transfers resume from the last durable checkpoint after a killed process, truncating its later
tail. Range resumption requires a strong matching ETag and exact Content-Range and length; a full
200 response replaces the prefix, and weak or missing validators restart from zero. Malformed,
encoded, excess, truncated and hash-invalid responses never publish, and the shared HTTPS client
bounds redirects and has finite timeouts. Weights are never Git or installer content.
