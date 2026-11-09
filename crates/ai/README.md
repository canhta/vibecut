# ai

Provider contracts, execution policy, consent-gated OpenAI and ElevenLabs transcription, local Whisper/Gipformer transcription, bounded local vision inference, recognition assembly, model metadata and the shared HTTPS transport. Excludes download execution, audio decoding, job scheduling/retries/checkpoints, cache storage, cue segmentation and user-edit policy.

Implement `Transcriber`, `Aligner`, `Translator` or `clips::ClipSuggester`; call the free `transcribe`, `translate`, `assemble` and `clips::suggest_clips`. Adapter methods are blocking and object safe; engine runs them on its worker and supplies `ExecutionHooks`. Adapters observe cancellation during expensive work and do no IO in `config`, `capabilities` or `estimate`. The crate has no runtime or engine dependency. Typed errors carry categories, safe static diagnostics and an optional retry delay, never provider bodies, text, keys or paths.

Adapters cap response-parser allocation before building transport records; engine owns job-level memory budgets. Offline fixtures prove contract behavior only, not accuracy, timing fidelity, billing, streaming or cross-platform behavior.

| Module | Contract |
|---|---|
| `alignment/` | [Local Candle alignment](src/alignment/README.md) |
| `anthropic.rs` | [Anthropic translator](src/anthropic/README.md) |
| `assembly.rs` | [Recognition and time](src/assembly/README.md) |
| `chapters/` | [Chapter generation](src/chapters/README.md) |
| `clips/` | [Transcript clip suggestions](src/clips/README.md) |
| `contracts.rs` | [Provider contracts](src/contracts/README.md) |
| `elevenlabs.rs` | [ElevenLabs Scribe transcription](src/elevenlabs/README.md) |
| `elevenlabs_speech.rs` | [ElevenLabs speech synthesis](src/elevenlabs_speech/README.md) |
| `gipformer/` | [Local Gipformer](src/gipformer/README.md) |
| `models.rs` | [Model catalog and language-neutral weights](src/models/README.md) |
| `openai/` | [OpenAI adapters](src/openai/README.md) |
| `policy.rs` | [Consent and cost](src/policy/README.md) |
| `speech.rs` | [Speech synthesis](src/speech/README.md) |
| `structured.rs` | [source](src/structured.rs) |
| `translation.rs` | [Contextual text translation](src/translation/README.md) |
| `transport.rs` | [source](src/transport.rs) |
| `vision/` | [Local vision detectors](src/vision/README.md) |
| `whisper/` | [Local Whisper](src/whisper/README.md) |
