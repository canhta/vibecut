# media

Headless streaming media I/O through dynamically linked FFmpeg (LGPL on macOS and Windows, GPL with libx264 on Linux). Owns
probe, demux, codec parameters, decode/encode, hardware selection, conversion,
resampling and muxing. Rust timeline semantics, compositing, captions, text,
provider policy, jobs and output promotion belong to their respective owners.

| Module | Responsibility | Contract |
|---|---|---|
| `asset.rs` | Owns persistable source identity and probe metadata. | [Source identity](src/asset/README.md) |
| `audio_extract.rs` | Prepares speech audio and energy windows. | [Speech upload chunks](src/audio_extract/README.md) |
| `decode.rs` | Decodes packets into frames and reads video. | [Decoding](src/decode/README.md) |
| `encode.rs` | Encodes video and audio under quality boundaries. | [Encoding](src/encode/README.md) |
| `ffi.rs` | Fills narrow FFmpeg wrapper gaps and bounds native use. | [Bounds and verification](src/ffi/README.md) |
| `model.rs` | Defines time bases, packets and codec parameters. | [Time base and packets](src/model/README.md) |
| `mux.rs` | Writes containers over a seekable sink. | [Muxing](src/mux/README.md) |
| `read.rs` | Probes containers and delivers sequential frames. | [Entry points and timing](src/read/README.md) |
| `resample.rs` | Converts sample rate, layout and format. | [Audio conversion](src/resample/README.md) |
| `summary.rs` | Derives persistable import metadata and capabilities. | [Import metadata and thumbnails](src/summary/README.md) |
| `thumbnail.rs` | Decodes bounded first-frame thumbnails. | [Thumbnails](src/thumbnail/README.md) |
| `vision_prep.rs` | Prepares frames and MFCC for local vision. | [Vision preparation](src/vision_prep/README.md) |

All errors use `MediaError`; logs omit paths and media/user text. FFmpeg's default
logger is disabled because it can disclose filenames. After a codec/mux error,
discard that execution; flush is idempotent after successful completion.
