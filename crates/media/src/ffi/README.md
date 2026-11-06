# Bounds and verification

Individual native FFmpeg allocations are capped at 256 MiB before media I/O.
AVIO uses 32 KiB buffers; probing reads at most 32 MiB. A first-frame thumbnail
reads at most 64 MiB in total (including opening and seek/decode-forward), with a single software decoder
worker, a native 2^21-pixel image/scratch ceiling, and small output RGBA planes
plus the packed thumbnail. Inputs exceeding this thumbnail ceiling return a
typed size refusal; video assets remain usable with a placeholder. The same
native bound applies when geometry must be recovered during opening. This is a
stricter bound than general playback's 2^26-pixel ceiling. It it never reads the file for hashing (the import job
hashes separately by design). Stream count is capped at
256, extradata at 1 MiB, packets at 64 MiB, decoded frame area at 2^26 pixels,
audio frames at 65,536 samples/channel and 32 channels, and one receive batch at
64 decoded frames.

Video encoding requests at most eight codec workers (also capped by available
parallelism), independently of conversion workers. Each returned video packet
batch admits at most 64 MiB of accounted payload, side data and metadata, including
vector capacity slack; packet count is not a codec-delay limit. Oversized batches
return `MediaError::TooLarge`; discard that execution after a refusal. Final
video flush receives until EOF and repeated successful flushes are empty, as
specified by [FFmpeg's send/receive contract](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/avcodec.h).
Regressions cover two- and sixteen-thread encoders with more than 64 delayed packets and a
60 s 1080p30 encode, including decoded frame count and mux duration.

Audio encoding admits up to eight channels and caps its FIFO at
131,072 samples/channel. Native FFmpeg allocation/codec limits still apply;
these checks do not establish safety for every hostile C-library input. The decoder
keeps the native 2^26-pixel ceiling rather than tightening it to the probed frame
area: FFmpeg also checks codec scratch buffers against that ceiling. Visible input
and output dimensions are checked separately. MPEG-4 Part 2 public decoder/reader
regressions cover 320×180 A/V, small and unaligned B-frame inputs, seeking, frame
counts and byte-identical YUV420P pixels against external FFmpeg with both decode
preferences; hardware preference does not imply that this codec uses a GPU.
The native build backports upstream PSSH iterative cleanup and linear append fixes under
[ADR 0002](../../../../docs/adr/0002-desktop-media-and-time.md). A generated 20,000-box
PSSH regression probes on a 512 KiB worker stack in a subprocess with a five-second
deadline. Full-size same-rate and upsampled PCM and AVI-to-MP4 H.264 stream copy
have dedicated public API regressions.

`cargo xtask ffmpeg` resolves/builds the current SDK in the external user
cache. `cargo xtask ci` configures that SDK for workspace checks. Direct Cargo
invocations need `FFMPEG_DIR` and the platform's runtime library search path; use
the xtask commands for the configured development path. See
[development workflow](../../../../docs/engineering/workflows.md#upgrade-policy).

Tests generate synthetic inputs with an independently installed ffmpeg, then
exercise public packet/decode/encode/resample/mux interfaces. Subprocess ffprobe
checks counts, durations, codecs and tags. The CLI and production backend share
FFmpeg algorithms: these are interface/pipeline oracles, not independent codec
implementations. Missing tools are explicit skips; `VIBECUT_REQUIRE_ORACLES=1`
requires them. A found tool that fails is a failure.
Failed oracle subprocesses report the process exit status and the first 4 KiB of each
output stream, with full byte counts; they are never retried or treated as skips.

Observed on macOS: Unicode/space paths, cancellation, deterministic truncation and
bit-flip cases, and a 2,500,002,687-byte sparse MP4 with only 65,536 bytes read while
opening and reading its first packet. Source-rate AAC processing produced 48,000
samples; 48 kHz stereo -> 16 kHz mono produced 16,000 samples, bit-exact against
the CLI resampling fixture. Native codec behavior on other machines is unverified.
