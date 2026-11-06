# Source identity

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
including one-frame movies. The [engine import policy](../../../engine/README.md#import-policy)
owns how this classification affects project FPS.

`availability` checks file presence without hashing; missing media is a relink
state, not a malformed project. `relink` accepts a new absolute path only if its
full content hash matches, preserving asset identity and probed metadata.
`ContentHash::from_bytes` hashes already bounded content using the same BLAKE3
identity as `ContentHash::read`, reused by project publication receipts.
Presence alone does not prove unchanged content. `ContentHash::read_with_cancel`
checks cooperative cancellation throughout streaming full-content hashing.
Project persistence retains
these media-owned contracts; it does not recreate probe or asset models.
