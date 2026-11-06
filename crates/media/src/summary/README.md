# Import metadata and thumbnails

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
engine cache; no decoded native frame enters project state. A build without a required
decoder returns `UnsupportedReason::DecoderUnavailable`; no filename or browser
codec hint is inferred.
