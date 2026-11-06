# Artifact cache

## Reuse and ASR checkpoints

`cache::CacheKey::new(KeyInputs)` hashes versioned, length-delimited BLAKE3 inputs.
Owners supply canonical normalized settings and semantic content hashes (not paths,
Debug strings or unstable JSON map order). Provider/model/version fields admit
1..=256 bytes each; settings admit at most 1 MiB. Every key includes source content,
provider/model/version, settings, transcript, cue, clip-range and translation
fingerprints; unused fields are zero. A changed input yields a different key,
without a generic dependency graph. Artifact kinds and their additional identity:

| Artifact | Inputs supplied by its domain owner |
|---|---|
| SpeechAudio | source content, exact source range, sample format/rate, conversion version |
| AsrChunk | speech/source content, exact invocation sample range, provider/model/version, decoding/language settings |
| Transcript | finalized chunk identities, recognition/assembly settings and version |
| Translation | transcript/text content, source/target language, provider/model/version/settings |
| Cues | transcript, translations, language profile and segmentation settings/version |
| Preview | cues, clip/source ranges, compositor/version and pixel/audio settings |
| Thumbnail | source content hash, requested frame position and output edge |
| ModelBundle | complete ai catalog record including named-file SHA-256, size, source and licence |

`ArtifactCache::open` owns one exclusive cache directory and rebuilds bounded
metadata after restart. Use only one live writer per directory; application
placement chooses the directory and an OS file lock enforces exclusivity (`CacheBusy`
on contention). Cache writers and native model leases share the lock; the last
logical owner explicitly unlocks it, so inherited or duplicated OS descriptors
cannot keep a dropped cache busy. It removes atomic staging debris and malformed
canonical entries with a diagnostic count. Unrecognized files are skipped and
never deleted; they are outside cache accounting. Directory scanning streams metadata;
excess entries evict without reading their payloads. Headers bind the canonical
key; `get` checks admitted size, header and payload digest before returning a hit.
Corruption/missing files return `CacheCorrupt`; corrupt-file removal waits for any
older reader pins to release, so a caller can record the typed diagnostic and
recompute. Other IO errors retain their source.

`put` commits a complete artifact through project's synced atomic-file writer.
The 72-byte envelope is charged along with payload, so even empty entries consume
budget. The shared budget covers persistent bytes and the temporary envelope/read
allocation; put refuses an artifact whose envelope exceeds half the aggregate
limit (`ArtifactTooLarge`), because staging and disk must fit together. Pressure
evicts least-recently-used unpinned entries by bytes; max_entries also bounds
metadata. Reopen resets recency. A read first pins its requested entry, then uses
the shared reservation/reclamation path for read memory, evicting unrelated
unpinned files as needed. A read returns an explicit ArtifactLease pin, with
separately charged read memory. Eviction and `clear` skip pinned files;
clear returns the count still in use and a later clear removes them after release.
Put cannot replace a pinned key. Dropping the cache keeps outstanding reader bytes
and disk charges alive through their leases; closed, unleased directories are
reaccounted on next open. OS/allocator overhead and unopened cache directories are
outside this active-engine accounting.

### ASR checkpoints

A public-job crash test commits chunk one, unwinds before
whole-result completion, reopens the cache and proves only chunk two executes.
This establishes checkpoint mechanics with synthetic bytes, not speech-boundary
quality for long recordings. Encoded-export resume returns a typed refusal in render; independently finalized
segments and manifest/codec/source identity would be required before admitting it. No partial encoded file is treated as a reusable artifact.

Observed verification: `tests/it/runtime/jobs.rs` exercises actual submission with controlled
cancellation, failure, unwind, oversized results, deterministic retry/backoff,
resource/queue pressure, 200 completions with stable entry/charged-byte bounds,
semantic key invalidation, restart checkpoints, corruption, byte eviction, pinning,
clear during use and stale results after edit/undo/save/Session switch. Regression tests cover discarded transaction targets, cache-starved processing and read pressure.
Additional boundary tests cover failed/unwinding/net-zero groups and submitted
outputs, reopened/multiple-cache reclamation, global recency, active pins,
concurrent reclaimers and repair after unlink failure.
No new media/render algorithm is introduced; throughput, RSS, provider quality and
Windows runtime behavior remain unverified by these synthetic lifecycle tests.
