# source

Persistable source identity, the probe description of a container and its streams, and the import-facing summary. Runtime codec handles never enter this module.

## Identity

`SourceAsset::import` probes an absolute native path and streams its complete bytes through BLAKE3. `import_with_cancel` takes one cooperative `Arc<AtomicBool>`, checks it before canonicalization, passes it into the FFmpeg probe interrupt callback and checks it before every hash chunk; cancellation returns `MediaError::Cancelled` and publishes no asset. `AssetData` validates persistable identity, path, hash and `SourceProbe` metadata and never serializes codec handles. Metadata keeps rational time bases and rates, optional duration, count and VFR evidence, audio start offsets and priming, and video color and bit depth. Stream indexes are unique; the stream list is capped at 256. Paths require lossless Unicode serialization. Paths and hashes are redacted in debug output. Callers schedule imports and keep source files stable while probing and hashing; this is not a filesystem snapshot.

`AssetData.ephemeral` is required persisted metadata, false for native imports. `SourceAsset::ephemeral` lets library consumers omit internal sources without removing them from the project or restricting rendering. Relinking preserves it. `is_image` recognizes PNG, JPEG, WebP, BMP and TIFF pipe demuxers plus GIF/APNG by probed format, never filename. Other containers, including one-frame movies, keep video semantics. [Engine import policy](../../../engine/src/session/edit/README.md#import-policy) owns how this affects project FPS.

`availability` checks file presence without hashing; missing media is a relink state, not a malformed project. `relink` accepts a new absolute path only if its full content hash matches, preserving asset identity and probed metadata. Presence alone does not prove unchanged content. `ContentHash::read_with_cancel` checks cancellation throughout full-content hashing and reports bytes hashed and file length so callers can show in-file progress. `ContentHash::from_bytes` hashes already bounded content with the same BLAKE3 identity as `ContentHash::read`.

## Probe

`Probe` describes one opened container: detected format, optional presentation duration and the streams with indexes, codecs, rational time bases, start and duration, optional frame rate, count and VFR evidence, video color and bit depth, audio rate, channels and priming, and owned codec parameters. Unknown counts and VFR stay unknown. Source-rate audio keeps its channel layout and presentation clock.

## Time and encoded units

`TimeBase` is positive rational seconds per unit, converted through `time` with checked integer arithmetic and explicit rounding. FFmpeg applies container edits, codec priming and packet or frame reordering once; never reapply an offset.

`Packet` holds an FFmpeg buffer with original PTS/DTS/duration. `shifted` rebases checked timestamps without changing payloads; `try_clone` shares the buffer without copying. `CodecParameters` owns its extradata. `display_transform_required` checks sample aspect ratio and display matrix so a pixel consumer can refuse unsupported presentation instead of losing it.

## Import metadata

`SourceAsset::import_with_summary` runs the same canonicalize, probe and hash pass and also returns transient `ProbeCapabilities`, which opens (never decodes) every audio and video decoder in this FFmpeg build. Capabilities never enter the persisted asset. `MediaMetadata::from_probe` derives the persistable summary: `MediaKind` (video, image, audio), container duration, video dimensions, `has_audio` and `DisplayFrameRate` (the exact rational plus the nearest integer for compact display, so `24000/1001` shows 24).

Image demuxers may report coded dimensions only after the first frame, so the probe then decodes one frame per missing video stream, records its geometry and rewinds before any packet consumer runs. Recovery keeps the 32 MiB opening read allowance and a 4096-packet ceiling until it completes. `SourceAsset::import_prepared` hashes first and accepts validated content-keyed probe metadata, skipping recovery decode on a warm import. Its progress callback reports hash bytes and file length. On a cold recovery it returns the bounded thumbnail with the asset for the engine cache. A build without a required decoder returns `UnsupportedReason::DecoderUnavailable`.

## Sample anchors

Runtime-only sample anchors are bound to source content: clones share them, and persistence and relinking start cold. No codec handle or lock is held across I/O. They record exact decoded-sample positions for repeated MFCC analysis and are excluded from asset identity, serialization and schema. They live on the source because every analysis range of one asset reuses them.
