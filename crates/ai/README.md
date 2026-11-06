# ai

Provider contracts (including offline speech synthesis), execution policy, consent-gated OpenAI and ElevenLabs file
transcription and local Whisper/Gipformer transcription. Owns provider configuration,
capability lookup, bounded local vision inference, scoped upload consent, invocation cost bounds, recognition
assembly, model metadata, download-consent tokens, streaming integrity verification,
verified model loading and the shared HTTPS transport. Excludes model download
execution, audio decoding, job scheduling/retries/checkpoints, cache
storage/promotion, cue segmentation and user-edit policy.

## Entry points

Implement `Transcriber`, `Aligner`, `Translator` or `clips::ClipSuggester`; call the free
`transcribe`, `translate`, `assemble` and `clips::suggest_clips` execution functions. Adapter methods are blocking and
object safe; engine runs them on its worker and supplies `ExecutionHooks`.
Adapters must observe cancellation during expensive work and must not perform IO
in `config`, `capabilities` or `estimate`. No runtime or engine dependency is
introduced. Typed errors carry categories, safe static diagnostics and optional
retry delay, never provider bodies, text, keys or paths.

| Module | Responsibility | Contract |
|---|---|---|
| `alignment/` | Aligns untimed recognition against source audio. | [Local Candle alignment](src/alignment/README.md) |
| `anthropic.rs` | Translates and suggests clips through Anthropic Messages. | [Anthropic translator](src/anthropic/README.md) |
| `assembly.rs` | Validates, aligns and assembles recognition chunks. | [Recognition and time](src/assembly/README.md) |
| `clips/` | Plans, executes and validates transcript clip suggestions. | [Transcript clip suggestions](src/clips/README.md) |
| `contracts.rs` | Defines the provider configuration and capability contracts. | [Provider contracts](src/contracts/README.md) |
| `elevenlabs.rs` | Transcribes through ElevenLabs Scribe. | [ElevenLabs Scribe transcription](src/elevenlabs/README.md) |
| `elevenlabs_speech.rs` | Synthesizes speech through ElevenLabs. | [ElevenLabs speech synthesis](src/elevenlabs_speech/README.md) |
| `gipformer/` | Runs the isolated local Gipformer recognizer. | [Local Gipformer](src/gipformer/README.md) |
| `models.rs` | Publishes the model catalog and language-neutral weights. | [Model catalog and language-neutral weights](src/models/README.md) |
| `openai/` | Transcribes, translates and synthesizes through OpenAI. | [OpenAI adapters](src/openai/README.md) |
| `policy.rs` | Owns consent, cost bounds and provider selection. | [Consent and cost](src/policy/README.md) |
| `speech.rs` | Defines the speech synthesis contract. | [Speech synthesis](src/speech/README.md) |
| `structured.rs` | Parses the shared structured-output envelope. | [source](src/structured.rs) |
| `translation.rs` | Defines contextual text translation windows. | [Contextual text translation](src/translation/README.md) |
| `transport.rs` | Provides the shared bounded HTTPS transport. | [source](src/transport.rs) |
| `vision/` | Runs bounded local vision inference. | [Local vision detectors](src/vision/README.md) |
| `whisper/` | Runs local Whisper transcription. | [Local Whisper](src/whisper/README.md) |

## Verification and limits

`cargo test -p ai` exercises JSON native-timed and text-only fixtures through fake
adapters using the production seams: NFC Vietnamese, source offsets, missing-span
alignment, text-only refusal, capabilities/limits, consent/cost, malformed response,
translation, overlap ambiguity, raw-token public execution/serialization,
UTF-8-split NFD/Han/emoji correspondence, explicit missing timing, hostile token
responses, redaction, typed failure and cancellation. Model-catalog tests cover
language-neutral metadata, non-match of neutral weights in speech selection,
unchanged language-bound entries, pinned vision file hashes and refused
truncated/mismatched bytes.
`cargo xtask ci` also checks architecture, formatting and Clippy.

The generic provider fixtures establish offline contract behavior. Adapter-specific
live and local runtime evidence is documented in the module READMEs; these fixtures do not
establish general model accuracy, timing fidelity, billing, live streaming or
cross-platform behavior. Adapters own response-parser allocation caps before
constructing transport records; engine owns job-level memory budgets.
