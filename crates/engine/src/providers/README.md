# Providers

Interface: `Providers`, `Credentials`, `Error`, `ConsentPrompt`, `TextAccess`, `TextProvider`,
`provider_options`, `model_info`, `models`.

## Failures

`Error` owns setup, capability and consent refusals: a missing local model or Gipformer runner, a
missing provider credential, auto-detection on a provider that is not local whisper.cpp, and a
remote upload without consent. Display states the fact only, never a command to run: the CLI adds
its `vibecut-cli model download <id>` hint and the desktop renders its own text from the variant.
Callers that still return the engine aggregate wrap it as `engine::Error::Providers`; the wire maps consent
to `Permission`, missing setup prerequisites to `Setup`, and unsupported auto-language to `Refused`.

## Adapters, scopes and consent

`from_parts` receives the resolved ASR mode explicitly: remote file adapters use
their `file` mode, while injected/local adapters supply their own mode. Scope and
consent retain that exact choice; composition does not rewrite it during execution.

`Providers` holds one `Arc<dyn ai::Transcriber>`, an optional aligner and an optional `Arc<dyn
ai::Translator>`; it never names an adapter after `open`. Operation scopes come from the AI
constructors (`Scope::transcription`, `alignment`, `translation`, `clip_suggestion`, `chapters`,
`fillers`), so no caller fills `Scope` fields. `Credentials::from_env` is the one place that reads
the API-key environment variables; every desktop and CLI composition root calls it.
Credential capture is infallible. Only the chosen provider's accessor validates its key and
returns an owned, redacting `ApiKey`: a missing key is `CredentialMissing`, and an invalid key
retains AI's typed refusal. Malformed values belonging to other providers never block a run.

`UploadRequest::for_scope` builds transcription, translation and transcript-text consent from the adapter's neutral
`ai::UploadDisclosure` for the exact scope. An adapter without a disclosure that has a local
configuration has no upload and is refused; a remote adapter composed without one (a test
provider) is described from its configuration and scope. `ConsentPrompt` is the only consent
shape: the prompt shows the request and returns `Ok` or `ConsentRequired`; `approve_upload` binds the display to the scope and grants the `ai::Consent` token for the displayed estimate. Unknown pricing calls `approve_unknown_price`, whose default refuses; money-only approval never authorizes unknown spending. Shorts keeps its operation-specific disclosure
terms on the same `UploadRequest` shape.

Each run uses one provider. A fallback runs only when the user configured it, and nothing lets a
user configure one yet, so there is no fallback runner; it returns with the setting that
configures it.

Language detection is `ai::Transcriber::detect_language`; only local whisper.cpp implements it,
every other adapter returns `Unsupported`, which engine reports as `AutoLanguageUnsupported`.

## Transcript-text generation

`TextProvider` is the user's explicit transcript-text provider and model for the chapters and
fillers operations, listed by `provider_options` and resolved by `choose_chapter` /
`choose_filler`; there is no fallback, and [`ai::defaults`](../../../ai/src/remote/defaults.rs) owns the shipped model ids. `TextAccess::open(provider, credentials, consent)`
builds both the chapter adapter and the filler classifier once; `TextAccess::new(chapters,
fillers, consent)` lends already built ones. Composition roots construct one access per process
and lend it through `control::Caller.chapters`; the desktop shares it with its MCP inbox with
`Inbox::attach_text`.

`TextAccess::prepare_chapter(input)` and `prepare_fillers(input)` bind the complete canonical
transcript, the provider configuration, the operation and a bounded cost estimate into an
`ai::Scope`, then ask the composition's `ConsentPrompt` to approve the adapter's disclosure.
The prompt is where the human decision lives: it shows provider, model, payload and cost;
`StoredConsent::grant` records already-given human approvals per exact `ProviderConfig` and cost
ceiling, using `UploadRequest::config` rather than display labels. A different configuration
returns `ConsentRequired`; a higher, unknown or different-currency estimate returns `ai::Error::Cost`,
before any adapter call. The access grants the token only after approval and never retries or
switches provider. Both operations share the `ai::text_model` request path: one official backend check,
one bounded body and one response parser; they differ only in prompt, schema, settings
fingerprint and result validation.

## Local model transfers

`models::ModelTransfers` composes JobRunner, ArtifactCache and ai's shared streaming transport.
`open_default` uses [runtime's `CacheRoot::models()`](../runtime/README.md#default-cache-location);
unavailable discovery returns kernel `engine::EditError::Unsupported` composed through `engine::Error::Edit`. `new` accepts an already owned cache and injected
`ModelIo`. The runner must share that cache's ByteBudget. There is one active transfer per cache,
including queue and backoff; another submission or removal during that ownership returns
[`RuntimeError::CacheBusy`](../runtime/README.md#background-jobs-and-byte-policy) through
`engine::Error::Runtime`, as does removal while a native model lease is live. State mutexes cover
ownership changes only, never network, file IO or verification.

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
`engine::Error::Runtime(RuntimeError::StorageFull)` when availability changes after the
advisory check. Malformed transfer representations compose `RuntimeError::ModelResponse`;
both are source-free, nontransient `Refused` failures owned by [Runtime](../runtime/README.md).
Other IO and foreign AI integrity/storage errors keep their existing types. Unknown directories and
symlink targets are never adopted or overwritten.

Transfers resume from the last durable checkpoint after a killed process, truncating its later
tail. Range resumption requires a strong matching ETag and exact Content-Range and length; a full
200 response replaces the prefix, and weak or missing validators restart from zero. Malformed,
encoded, excess, truncated and hash-invalid responses never publish, and the shared HTTPS client
bounds redirects and has finite timeouts. Weights are never Git or installer content.

Native cache metadata and installed-file verification failures arrive through
`RuntimeError::Model` with the original AI source; this opaque resource failure does not
turn AI verification cancellation into direct scheduler cancellation. Transfer callbacks
retain their declared mixed engine error, consent, retry and publication policy.
