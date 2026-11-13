# media

Headless streaming media I/O through dynamically linked FFmpeg. Owns source identity, probe, demux, codec parameters, decode and encode, hardware selection, conversion, resampling, muxing and vision media preparation. Timeline semantics, compositing, captions, text, provider policy, jobs and output promotion belong to their respective owners. FFmpeg licensing and distribution are owned by the [open-source policy](../../docs/engineering/open-source.md#ffmpeg-distribution).

| Module | Owns | Contract |
|---|---|---|
| `source/` | Source identity and what a source contains | [Source identity and import metadata](src/source/README.md) |
| `read/` | Containers, packet decoding, sequential video, thumbnails | [Reading and decoding](src/read/README.md) |
| `write/` | Video, audio and GIF encoding, encoder selection, container writing | [Encoding and muxing](src/write/README.md) |
| `audio/` | Speech extraction, chunk planning, speech energy, source peaks, resampling | [Speech, peaks and audio conversion](src/audio/README.md) |
| `vision/` | Exact-time frames, face and scene tensors, MFCC features, preparation limits | [Vision preparation](src/vision/README.md) |
| `ffi` | Narrow FFmpeg wrapper gaps and native allocation bounds | [Bounds and verification](src/ffi/README.md) |

The crate root re-exports the public names; module paths are private.

All errors use `MediaError`. Logs omit paths and media or user text. FFmpeg's default logger is disabled because it can disclose filenames. After a codec or mux error, discard that execution; flush is idempotent after successful completion.

## Verification

Test and oracle policy, including required-oracle skips and failure handling, is owned by
[verification](../../docs/engineering/verification.md#checks).
