# ai

Provider contracts, execution policy, consent-gated OpenAI and ElevenLabs transcription, the streamed agent-chat conversation with its consent scope and spend ledger, local Whisper/Gipformer transcription, bounded local vision inference, recognition assembly, provider credentials and model metadata. Excludes download execution, bounded HTTPS, audio decoding, job scheduling/retries/checkpoints, cache storage, cue segmentation and user-edit policy.

Implement `Transcriber`, `Aligner`, `Translator` or `clips::ClipSuggester`; call the free `transcribe`, `translate`, `assemble` and `clips::suggest_clips`. Remote-capable traits also return one provider-neutral `UploadDisclosure` from `disclosure(&Scope)` (speech from `disclosure(&SpeechRequest)`); local adapters return `None`. Adapter methods are blocking and object safe; engine runs them on its worker and supplies `ExecutionHooks`. Adapters observe cancellation during expensive work and do no IO in `config`, `capabilities` or `estimate`. The crate has no runtime or engine dependency. Typed errors carry categories, safe static diagnostics and an optional retry delay, never provider bodies, text, keys or paths.

Adapters cap response-parser allocation before building transport records; engine owns job-level memory budgets. Offline fixtures prove contract behavior only, not accuracy, timing fidelity, billing, streaming or cross-platform behavior.

| Module | Contract |
|---|---|
| `foundation/` | [Contracts, remote admission, models, transport](src/foundation/README.md) |
| `foundation/contracts/` | [Provider contracts](src/foundation/contracts/README.md) |
| `foundation/models/` | [Model catalog and language-neutral weights](src/foundation/models/README.md) |
| `foundation/remote/` | [Shared remote provider admission](src/foundation/remote/README.md) |
| `foundation/transport/` | [Provider credentials, payload encoding and structured text-model request](src/foundation/transport/README.md) |
| `providers/` | [Remote provider adapters](src/providers/README.md) |
| `providers/anthropic/` | [Anthropic translator](src/providers/anthropic/README.md) |
| `providers/elevenlabs/` | [ElevenLabs transcription and speech synthesis](src/providers/elevenlabs/README.md) |
| `providers/openai/` | [OpenAI adapters](src/providers/openai/README.md) |
| `recognition/` | [Local recognition engines and assembly](src/recognition/README.md) |
| `recognition/alignment/` | [Local Candle alignment](src/recognition/alignment/README.md) |
| `recognition/assembly/` | [Recognition and time](src/recognition/assembly/README.md) |
| `recognition/gipformer/` | [Local Gipformer](src/recognition/gipformer/README.md) |
| `recognition/whisper/` | [Local Whisper](src/recognition/whisper/README.md) |
| `speech/` | [Speech synthesis](src/speech/README.md) |
| `text/` | [Transcript-text operations](src/text/README.md) |
| `text/chapters/` | [Chapter generation](src/text/chapters/README.md) |
| `text/clips/` | [Transcript clip suggestions](src/text/clips/README.md) |
| `text/conversation/` | [Agent chat conversation](src/text/conversation/README.md) |
| `text/emphasis/` | [Caption emphasis selection](src/text/emphasis/README.md) |
| `text/fillers/` | [Filler classification](src/text/fillers/README.md) |
| `text/structured` | [Strict JSON format and parser](src/text/README.md) |
| `text/translation/` | [Contextual text translation](src/text/translation/README.md) |
| `vision/` | [Local vision detectors](src/vision/README.md) |
