# Speech synthesis

`speech::SpeechSynthesizer` is the first speech adapter contract. Its `config`,
`capabilities` and `estimate` methods perform no IO; `synthesize` is blocking,
object safe and single attempt with an explicit `SynthesisConsent` and shared
`ExecutionHooks`. Call `speech::synthesize` for preflight, cancellation checks
before/after adapter execution and output validation. Adapters also call
`prepare_synthesis` before IO when directly invoked, observe cancellation during
long work, bound network/decoder allocation and reject malformed/truncated wire
audio. The contract slice introduced no provider adapter; the [OpenAI module](../openai/README.md) adds one. There is still no voice-listing operation, UI, job stage,
fallback, timeline placement or mixing. A future optional
voice listing operation must be separate from the IO-free capability path and
must obtain its own operation-specific disclosure/approval.

`SpeechRequest::new` accepts exact NFC text (nonblank, at most 64 KiB), a provider
configuration, explicit stock/built-in voice ID, exact language/script pair,
output format/rate and positive maximum duration in integer microseconds.
`SpeechCapabilities` resolves supported language/script pairs, stock/built-in
voice IDs/categories, WAV PCM16/raw PCM16 format-rate pairs, grapheme limit,
streaming status, concurrency/rate limits, cost model, fingerprint and provider
policy URL with retention/training/voice-disclosure text. There is no implicit
locale or script fallback. Unknown limits use `None`, never an unlimited claim.
Provider-specific scalar/byte limits remain the adapter's responsibility.

After displaying the exact payload, provider policy and quote, the composition
root calls `SynthesisConsent::grant`. The in-memory, non-serializable token binds
the full request by value, SHA-256 of NFC UTF-8, extended-grapheme character count,
synthesis operation, full provider configuration (endpoint/model/settings/auth
reference), voice, language/script, output format/rate, duration ceiling, exact
capabilities/fingerprint/policy disclosure, quoted cost and approved budget. Any
change requires fresh approval, including a smaller quote or shorter payload.
Equivalent decomposed Vietnamese normalizes to the same NFC payload.

`SynthesisEstimate` is Free, Exact, UpperBound or explicitly unresolved Unknown.
Unknown, currency mismatch and over-budget quotes are refused before execution;
actual cost must fit the quote. Money reuses integer currency millionths.
Approval is per invocation: engine reserves aggregate retry/chunk spending before
reusing a token. No provider or model fallback is performed. Tokens constrain
cooperating adapters and are not a sandbox for arbitrary implementations.

`SpeechAudio` carries finite mono f32 PCM, matching output sample rate and
fingerprint, duration rounded **up** from sample count to integer microseconds,
usage/actual cost and an optional NFC transcript. The shared seam rejects empty,
nonfinite, over-64-MiB PCM, wrong rate/identity, inconsistent duration, duration
beyond the approved maximum and any supplied transcript differing from the exact
request. Missing transcript is allowed by the generic contract; adapters promising
exact transcripts must require it themselves. Container integrity and semantic
speech fidelity cannot be inferred from decoded samples alone. Cancellation
discards all output. Debug omits audio and redacts text/configuration; errors use
the crate's safe categories, including UnsupportedLanguage, UnsupportedVoice,
Payload, Auth, Quota, RateLimit, ContentPolicy, Server, Network, Timeout, Cost,
OutputRejected and Cancelled.

Observed offline evidence: `cargo test -p ai --test it speech::` exercises the production
seam with injected success/failure fixtures, exact scope/policy/quote approval,
NFC/grapheme boundaries, transcript mismatch, malformed/truncated decoded audio,
cost overruns and cancellation before/after execution. It establishes the
contract only; provider wire schemas, live output and voice quality belong to
the adapter tickets. No new dependency or persisted schema is introduced.
