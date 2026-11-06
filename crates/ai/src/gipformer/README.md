# Local Gipformer

Windows native builds use a single dynamic MSVC CRT (`/MD`) with Rust and Whisper.
The upstream Rust Sherpa downloader selects `/MT` archives, so workspace xtask
commands provision the locked `sherpa-onnx-sys` version's upstream **static `/MD`**
release archive and pass its `lib` directory through `SHERPA_ONNX_LIB_DIR`.
This retains static Sherpa/ONNX linking without introducing Sherpa runtime DLLs.
macOS continues using the upstream static archive.

`cargo xtask sherpa [--arch x86_64|aarch64]` prefetches the Windows archive and
prints its library directory; direct Cargo builds outside xtask must set
`SHERPA_ONNX_LIB_DIR` to that directory, alongside the normal FFmpeg environment.
An explicitly supplied `SHERPA_ONNX_LIB_DIR` is retained and must contain
architecture-matched static `/MD` libraries. Provisioning verifies the published
GitHub release-asset SHA-256 and size before extraction and keeps an atomic cache
receipt; incomplete caches fail with recovery guidance. The library version comes
from `Cargo.lock`. Native Windows CI separately links and runs Whisper-only and
Gipformer-only ai tests, then links/runs the combined default workspace tests.
The [workflow](../../../../docs/engineering/workflows.md#ffmpeg-provisioning) owns build
environment setup. Windows hardware/runtime acceptance remains unverified.

The default `gipformer` feature includes the safe sherpa-onnx bindings and the
`vibecut-local-asr` native runner binary. `gipformer::Gipformer::load` accepts the
pinned model, immutable cache directory, absolute shipped runner path, explicit
device/thread count and execution hooks. Load verifies all four files with the
shared integrity policy and starts a reusable native process. No local unsafe,
model download, engine dependency, second transfer cache or scheduler is added.

`models::suggested_transcriber` returns metadata for the Vietnamese profile's MIT
Gipformer model; other profiles retain the multilingual Whisper suggestion. This
never executes a provider, falls back to another provider or starts a download.
Composition still presents explicit provider choice and obtains a download click
using `DownloadConsent`; engine's existing model-transfer/cache owner consumes
the same catalog records. The app must ship the native runner alongside its
executable and pass its absolute trusted path. Windows packaging/runtime remains
unverified.

CPU execution is supported. The released static runtime silently falls back from
CoreML to CPU, so this adapter refuses `Device::CoreMl` with `Unsupported`. It does
not claim accelerator execution. A different upstream native build needs separate
validation; no third-party source or timing behavior is patched here.

Invocations accept finite mono 16 kHz f32 PCM and absolute source offsets, capped
at five minutes. The native runner buffers and decodes one 20 s chunk at a time,
including the short final tail. Engine owns longer recordings, speech-safe outer
windows and reconciliation. Hard chunk boundaries can affect recognition quality;
there is no additional VAD model download. The settings fingerprint includes the
model hashes, native binding, chunk/decode policy, hardware request and thread count.

The native runtime lives outside the application process because it can write
paths in diagnostics and terminate on native failures. Its stderr is discarded;
private framed pipes carry bounded metadata, PCM and an explicit recognition reply.
One invocation per adapter is supported; overlapping calls return `Unsupported`
for engine scheduling. Pool locks cover ownership changes only, never IO. Cancellation
acknowledges once, kills/reaps native work and joins pipe threads before returning
`Cancelled`; no partial output escapes. Hook unwinding also kills/reaps before
joining. A subsequent call replaces the stopped process. Poisoned ownership locks
fail with `Inference`; there is no speculative recovery.

Native RNNT token start points lack durations. The adapter preserves their raw
lexical bytes and ordinal order with explicit missing ranges and omits timing
capabilities. It preserves the leading BPE space rather than rewriting bytes to
match the runtime's trimmed display text; any other text discrepancy is an error.
`Recognition::to_transcript` returns `TimingUnavailable` until an explicitly
selected aligner supplies speech ranges. Shared assembly then retains raw/NFC
correspondence, forced provenance and sample-to-tick conversion. Neither adjacent
starts nor chunk ends become invented speech ends; no aligner or ASR provider is
selected automatically. Native timing MAE is consequently unavailable.

`cargo test -p ai` covers bounded framing, native failure, blocked-pipe cancellation,
hook unwind, missing timing, forced alignment/serialization and catalog consent.
Unix fake-process tests exercise actual OS pipe/kill/join behavior. The ignored
`local_gipformer` test requires explicit pinned weights and local speech; it uses
public execution, serialization, context reuse, 300 s cancellation and recovery.
`cargo run --release -p ai --example local_gipformer -- MODEL_DIRECTORY RUNNER
cpu THREADS AUDIO_F32LE...` benchmarks multiple inputs on one loaded model. JSONL
is deliberate caller-requested recognition output. Hardware/runtime performance,
model quality and accelerator acceptance require separate measured evidence;
builds and offline tests do not establish them.
