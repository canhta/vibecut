# media

Headless streaming media I/O through dynamically linked FFmpeg (LGPL on macOS and Windows, GPL with libx264 on Linux). Owns source identity, probe, demux, codec parameters, decode and encode, hardware selection, conversion, resampling, muxing and vision media preparation. Timeline semantics, compositing, captions, text, provider policy, jobs and output promotion belong to their respective owners.

| Module | Owns | Contract |
|---|---|---|
| `source/` | Source identity and what a source contains | [Source identity and import metadata](src/source/README.md) |
| `read/` | Containers, packet decoding, sequential video, thumbnails | [Reading and decoding](src/read/README.md) |
| `write/` | Video and audio encoding, encoder selection, container writing | [Encoding and muxing](src/write/README.md) |
| `audio/` | Speech extraction, chunk planning, speech energy, resampling | [Speech and audio conversion](src/audio/README.md) |
| `vision/` | Exact-time frames, face and scene tensors, MFCC features, preparation limits | [Vision preparation](src/vision/README.md) |
| `ffi` | Narrow FFmpeg wrapper gaps and native allocation bounds | [Bounds and verification](src/ffi/README.md) |

The crate root re-exports the public names; module paths are private.

All errors use `MediaError`. Logs omit paths and media or user text. FFmpeg's default logger is disabled because it can disclose filenames. After a codec or mux error, discard that execution; flush is idempotent after successful completion.

## Evidence

Tests generate synthetic inputs with an independently installed ffmpeg and exercise the public packet, decode, encode, resample and mux interfaces. Subprocess ffprobe checks counts, durations, codecs and tags. These are pipeline oracles, not independent codec implementations. Missing tools are explicit skips; `VIBECUT_REQUIRE_ORACLES=1` requires them. A found tool that fails is a failure. Failed oracle subprocesses report the exit status and the first 4 KiB of each output stream with full byte counts, and are never retried.

Native codec behaviour has been observed on macOS only.
