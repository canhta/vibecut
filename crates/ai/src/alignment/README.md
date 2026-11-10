# Local Candle alignment

`alignment::CandleAligner` (feature `candle-align`, default) implements `Aligner` on a safe CPU runtime independent of whisper.cpp. It loads the pinned `models::alignment_model()` from immutable app-managed files; all four files pass size/SHA-256 verification before buffered safetensors loading. No mmap, unsafe, transfers, remote inference or fallback. App downloads need a user click through the model manager.

The runtime teacher-forces the supplied transcript, takes the checkpoint's selected cross-attention heads, softmaxes over real audio frames, standardizes over query tokens, applies a seven-frame median filter and runs bounded monotonic DTW. Missing, nonfinite or degenerate attention returns `AttentionUnavailable`; zero-width lexical spans return `TimingUnavailable`. There is no `(0,0)` fallback or proportional timing. The prescribed text is preserved, never translated or replaced.

Input is finite mono 16 kHz PCM, 201..=480000 samples (at most 30 s), 1..=64 nonblank identified segments, at most 16 KiB of source text, IDs up to 256 bytes and at most 443 model text tokens. Longer input fails before model execution; engine owns longer speech-safe windows. The complete segment texts are bound to the shared alignment Request, and consent uses the AI policy. Local execution estimates Free.

Captions owns normalization and Unicode grouping. English words, Vietnamese syllables and Chinese characters use its timing profiles; bare `zh` uses the Han-character policy of zh-Hans/zh-Hant. Tokenization respects group boundaries rather than splitting an inferred duration. Punctuation may stay untimed. Forced unit records carry the profile's unit and the source IDs/text. Native source ranges constrain the returned intervals; invalid constraints refuse. Shared AI assembly keeps original native layers and does the transcript conversion.

`AudioWindow` coordinates are absolute source samples. Media supplies decoded, resampled PCM with codec delay handled; this adapter neither resamples nor adds delay, and centered STFT adds no offset. DTW frame boundaries map to 320 samples (20 ms), clamp to the window and native segment, and add the window start once.

One invocation per loaded adapter; engine schedules concurrency. Runtime ownership uses a nonblocking lock; contention or poisoning returns Inference. Cancellation is checked during mel framing, each encoder/decoder block and each DTW row; an in-flight matrix operation finishes first, then the adapter returns Cancelled and discards output. The settings fingerprint hashes all model metadata, runtime policy and language profiles.

CPU output proves integration only; there is no accuracy gate, and accelerator/Windows runtime is unverified (see the AI integration verification policy).

Candle/tokenizers impose transitive version limits; skips, parent constraints and removal conditions live in [deny.toml](../../../../deny.toml).
