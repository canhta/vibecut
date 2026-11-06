# source

Persistable source identity, the probe description of a container and its streams,
and the import-facing summary. Runtime codec handles never enter this module.

| Stage | Responsibility | Contract |
|---|---|---|
| `asset` | Owns persistable source identity, relinking and availability. | [Identity](#identity) |
| `probe` | Describes probed streams, timing and color metadata. | [Probe](#probe) |
| `model` | Models exact container time and encoded units. | [Time and encoded units](#time-and-encoded-units) |
| `summary` | Derives import metadata and transient decoder capabilities. | [Import metadata](#import-metadata) |
| `anchors` | Retains runtime-only decoded-audio sample anchors. | [Sample anchors](#sample-anchors) |

## Identity

`SourceAsset::import` probes an absolute native path and streams its complete
bytes through BLAKE3. `SourceAsset::import_with_cancel` takes one cooperative
`Arc<AtomicBool>` through canonicalization, the FFmpeg probe interrupt callback
and every hash chunk; cancellation returns `MediaError::Cancelled` without
publishing an asset. `AssetData` validates persistable identity/path/hash and
`SourceProbe` metadata, without serializing runtime codec handles. Metadata
retains rational time bases/rates, optional duration/count/VFR evidence, audio
start offsets/priming and video color/bit depth. Stream indexes are unique and
capped at 256; paths require lossless Unicode serialization. Paths/hashes are
redacted in asset debug output. Callers schedule imports and keep source files
stable while probing/hashing; this is not a filesystem snapshot operation.

`AssetData.ephemeral` is required persisted metadata, false for native imports.
`SourceAsset::ephemeral` lets library consumers omit internal sources without
removing them from the project or restricting rendering. Relinking preserves it.
`is_image` recognizes PNG, JPEG, WebP, BMP and TIFF pipe demuxers plus GIF/APNG by
probed format (never the filename). Other containers retain video semantics,
including one-frame movies. The [engine import policy](../../../engine/src/session/edit/README.md#import-policy)
owns how this classification affects project FPS.

`availability` checks file presence without hashing; missing media is a relink
state, not a malformed project. `relink` accepts a new absolute path only if its
full content hash matches, preserving asset identity and probed metadata.
`ContentHash::from_bytes` hashes already bounded content using the same BLAKE3
identity as `ContentHash::read`, reused by project publication receipts.
Presence alone does not prove unchanged content. `ContentHash::read_with_cancel`
checks cooperative cancellation throughout streaming full-content hashing.
Project persistence retains these media-owned contracts; it does not recreate
probe or asset models.

## Probe

`Probe` describes one opened container: detected format, optional presentation
duration and the streams with their indexes, codecs, rational time bases, start
and duration, optional frame rate/count and VFR evidence, video color and bit
depth, audio rate/channels/priming, and owned codec parameters. Unknown counts
and VFR stay unknown; no filename-based capability guesses are made. Source-rate
audio retains its channel layout and presentation clock.

## Time and encoded units

`TimeBase` is positive rational seconds per unit, converted through `time` with
checked integer arithmetic and explicit rounding. FFmpeg applies container
edits, codec priming and packet/frame reordering once; never reapply an offset.
`Packet` retains an FFmpeg buffer with original PTS/DTS/duration; `shifted`
rebases checked timestamps without changing payloads and `try_clone` retains the
buffer without copying it. `CodecParameters` owns its extradata;
`display_transform_required` checks sample aspect ratio and display matrix so a
pixel consumer can refuse unsupported presentation instead of losing it.

## Import metadata

`SourceAsset::import_with_summary` runs the same canonicalize/probe/hash pass and
also returns transient `ProbeCapabilities`, which opens (never decodes) every
audio/video decoder in this FFmpeg build. `MediaMetadata::from_probe` derives the
persistable summary: `MediaKind` (video/image/audio), container duration, video
dimensions, `has_audio` and `DisplayFrameRate` (the exact rational plus the
nearest integer chosen for compact display, so `24000/1001` shows 24 and
`30000/1001` shows 30). Capabilities never enter the persisted asset.

Image demuxers may report coded dimensions only after the first frame. The probe
therefore decodes exactly one frame in that case, records its real geometry and
rewinds the input before any packet consumer runs. Recovery keeps the 32 MiB
opening read allowance and a 4096-packet ceiling until it completes.
`SourceAsset::import_prepared` hashes first and accepts validated content-keyed
probe metadata, avoiding recovery decode on a warm import. On a cold recovery it
returns the bounded thumbnail alongside the imported asset, for reuse by the
engine cache; no decoded native frame enters project state. A build without a
required decoder returns `UnsupportedReason::DecoderUnavailable`; no filename or
browser codec hint is inferred.

## Sample anchors

`MfccAnchors` is runtime-only, source-content-bound state: clones share it,
persistence and relinking start cold, and no codec handle or lock is retained
across I/O. It records exact decoded-sample positions for repeated MFCC analysis
and is excluded from asset identity, serialization and schema. It lives on the
source because every analysis range of one asset reuses it, never because it is
persisted.
