# Local Candle alignment

`alignment::CandleAligner` (feature `candle-align`, enabled by default) implements
`Aligner` with its own safe CPU runtime, independent of whisper.cpp. Load the
pinned `models::alignment_model()` from immutable app-managed model files; all four
files pass existing size/SHA-256 verification before buffered safetensors loading.
No mmap, local unsafe, transfers, remote inference, provider fallback or duplicate
model store is introduced. The catalog includes source/licence/size/hash metadata;
app downloads still require a user click through the existing model manager.
The coordinator-approved development download stays outside Git under `.spikes/`.

The runtime teacher-forces caller-supplied transcript text, obtains the checkpoint's
selected cross-attention heads, softmaxes over real audio frames, standardizes over
query tokens, applies a seven-frame median filter, and runs bounded monotonic DTW.
Missing/nonfinite/degenerate attention returns `AttentionUnavailable`; zero-width
lexical spans return `TimingUnavailable`. No `(0,0)` fallback or proportional text
timestamps are returned. Prescribed text is preserved, never translated or decoded
as a replacement transcript.

Inputs are finite mono 16 kHz PCM, 201..=480000 samples (at most 30 seconds), 1..=64
nonblank identified segments, at most 16 KiB total source text, IDs up to 256 bytes,
and at most 443 model text tokens. Overlong inputs fail before model execution;
engine owns longer speech-safe windows and must not treat this as automatic chunking.
The complete original segment texts are bound to the shared alignment Request;
capabilities and consent use the existing AI policy. Local execution estimates Free.

Captions owns normalization and Unicode grouping. English words, Vietnamese
syllables and Chinese characters use its existing timing profiles; bare `zh` now
shares the Han-character policy with zh-Hans/zh-Hant. Tokenization respects those
group boundaries rather than subdividing an inferred token duration. Punctuation
can remain untimed. Forced unit records carry the profile's actual unit and preserve
source IDs/text. Native source ranges constrain the returned intervals; invalid
constraints refuse rather than fabricating durations. Shared AI assembly retains
any original native timing/token layers and performs the sole transcript conversion.

Input AudioWindow coordinates are absolute source samples. Media supplies decoded,
resampled PCM with codec/filter delay already handled; this adapter neither
resamples nor adds delay. Centered STFT contributes no extra offset. DTW frame
boundaries map to 320 samples (20 ms), clamp to the actual window/native segment,
and add the window start exactly once. The public native test checks offset
invariance through the real AI assembly and transcript conversion.

One invocation per loaded adapter is supported; the engine schedules concurrency.
Runtime ownership uses a nonblocking lock; contention/poisoning returns Inference.
Cancellation is checked during mel framing, each encoder/decoder block and each
DTW row. An in-flight matrix operation finishes before the next check; the adapter
then returns Cancelled and discards output. No extra worker/runtime is created.
The settings fingerprint hashes all model metadata, runtime policy and language
profiles, keeping model/grouping changes out of previously derived work.

Offline tests cover malformed/empty/nonfinite inputs and attention, monotonic DTW,
normalization/median filtering, Han/VI/EN grouping, bounded token counts, cancellation,
metadata refusal and analytical mel/offset fixtures. Opt-in `candle_alignment`
requires `VIBECUT_CANDLE_MODEL_DIR`, `VIBECUT_ALIGN_PCM` and `VIBECUT_ALIGN_SEGMENTS`;
it exercises actual local output through shared assembly, exact nonzero source
coordinates, cancellation and silence/empty refusal. `local_alignment` is a direct
local smoke example; its six arguments are model directory, f32le PCM, language,
segments JSON, output JSON and source offset in samples. No model accuracy or timing
quality gate is part of integration acceptance; see the AI integration verification
policy. CPU model output proves integration only. Accelerator/Windows runtime is
not established by these checks.

Candle/tokenizers impose approved transitive library-version limits alongside
existing dependencies; exact skips, parent constraints and removal conditions live
in [deny.toml](../../../../deny.toml). Direct dependencies remain latest stable, with no
third-party patch or downgrade. The repository had no duplicate gate wired into CI;
this file records the scoped exceptions for cargo-deny rather than adding a new
unrelated check. Remove each skip when its parent unifies with the workspace version.
