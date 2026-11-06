# media

Headless streaming media I/O through dynamically linked FFmpeg (LGPL on macOS and
Windows, GPL with libx264 on Linux). Owns source identity, probe, demux, codec
parameters, decode/encode, hardware selection, conversion, resampling, muxing and
vision media preparation. Rust timeline semantics, compositing, captions, text,
provider policy, jobs and output promotion belong to their respective owners.

| Module | Responsibility | Interface | Contract |
|---|---|---|---|
| `source/` | Identifies a source and describes what it contains. | `SourceAsset`, `Probe`, `MediaMetadata`, `ProbeCapabilities` | [Source identity and import metadata](src/source/README.md) |
| `read/` | Opens containers, decodes packets and renders thumbnails. | `PacketReader`, `probe`, `VideoReader`, `VideoDecoder`, `AudioDecoder`, `Thumbnail` | [Reading and decoding](src/read/README.md) |
| `write/` | Encodes frames and writes containers. | `VideoEncoder`, `AudioEncoder`, `Muxer`, `Container` | [Encoding and muxing](src/write/README.md) |
| `audio/` | Extracts and resamples speech audio into upload chunks. | `extract_speech_audio`, `SpeechWindow`, `AudioResampler` | [Speech and audio conversion](src/audio/README.md) |
| `vision/` | Prepares frames, face and scene tensors and MFCC features. | `VisionFrame`, `VisionPrepLimits`, `mfcc_range` | [Vision preparation](src/vision/README.md) |
| `ffi` | Fills narrow FFmpeg wrapper gaps and bounds native use. | internal | [Bounds and verification](src/ffi/README.md) |

The crate root re-exports the same public names in the same order as before the
folder split; module paths inside the crate are private.

## Contracts

- [source](src/source/README.md) owns identity, probe description, model values,
  import metadata and runtime sample anchors.
- [read](src/read/README.md) owns container I/O, packet decoding, sequential video
  access and thumbnails.
- [write](src/write/README.md) owns video/audio encoding, encoder selection and
  container writing.
- [audio](src/audio/README.md) owns speech extraction, chunk planning, speech
  energy and resampling.
- [vision](src/vision/README.md) owns exact-time frames, face and scene tensors,
  MFCC features and preparation limits.
- [ffi](src/ffi/README.md) owns the native allocation and verification bounds.

All errors use `MediaError`; logs omit paths and media/user text. FFmpeg's default
logger is disabled because it can disclose filenames. After a codec/mux error,
discard that execution; flush is idempotent after successful completion.

## Evidence

Tests generate synthetic inputs with an independently installed ffmpeg, then
exercise public packet/decode/encode/resample/mux interfaces. Subprocess ffprobe
checks counts, durations, codecs and tags. The CLI and production backend share
FFmpeg algorithms: these are interface/pipeline oracles, not independent codec
implementations. Missing tools are explicit skips; `VIBECUT_REQUIRE_ORACLES=1`
requires them. A found tool that fails is a failure. Failed oracle subprocesses
report the process exit status and the first 4 KiB of each output stream, with
full byte counts; they are never retried or treated as skips.

Observed on macOS: Unicode/space paths, cancellation, deterministic truncation and
bit-flip cases, and a 2,500,002,687-byte sparse MP4 with only 65,536 bytes read
while opening and reading its first packet. Source-rate AAC processing produced
48,000 samples; 48 kHz stereo -> 16 kHz mono produced 16,000 samples, bit-exact
against the CLI resampling fixture. Native codec behavior on other machines is
unverified.
