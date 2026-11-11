# Artifact cache

Interface: `cache::{ArtifactCache, ArtifactKind, ArtifactLease, CacheKey, KeyInputs, ModelLease}`.

`cache::CacheKey::new(KeyInputs)` hashes versioned, length-delimited BLAKE3 inputs. Owners supply
canonical normalized settings and semantic content hashes, never paths, Debug strings or unstable
JSON map order. Provider, model and version fields admit 1..=256 bytes each; settings admit at
most 1 MiB. Every key includes source content, provider/model/version, settings, transcript, cue,
clip-range and translation fingerprints; unused fields are zero. Artifact kinds and their
additional identity:

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

`ArtifactCache::open` owns one exclusive cache directory and rebuilds bounded metadata after
restart. Application placement chooses the directory and an OS file lock enforces one live
writer (contention composes [`RuntimeError::CacheBusy`](../README.md#background-jobs-and-byte-policy)
directly). Cache writers and native model leases share the lock; the last
logical owner explicitly unlocks it, so inherited or duplicated OS descriptors cannot keep a
dropped cache busy. Open removes atomic staging debris and malformed canonical entries with a
diagnostic count. Unrecognized files are skipped, never deleted, and sit outside cache
accounting; directory scanning streams metadata, and excess entries evict without reading their
payloads.

Headers bind the canonical key; `get` checks admitted size, header and payload digest before
returning a hit. Corruption or a missing file returns direct `RuntimeError::CacheCorrupt`; corrupt-file removal waits
for older reader pins to release, so a caller can record the typed diagnostic and recompute.
Other IO errors retain their source. All cache operations return the Runtime result. Reservation/pin/evict failures and
filesystem IO retain their direct Runtime owner and original source; mixed callers compose
upward at their workflow boundary. Atomic writer failures preserve Kernel Project sources
through transparent `RuntimeError::Edit`.
A failed unlink retains its charge and resets the eviction pin so repair and retry remain possible.
ASR checkpoint writes tolerate only runtime budget pressure, never IO or poisoned accounting.

`put` commits a complete artifact through project's synced atomic-file writer. The 72-byte
envelope is charged along with the payload, so even empty entries consume budget; the shared
budget covers persistent bytes and the temporary envelope and read allocation. `put` refuses an
artifact whose envelope exceeds half the aggregate limit (`ArtifactTooLarge`) and cannot replace
a pinned key. Pressure evicts least-recently-used unpinned entries by bytes, and `max_entries`
also bounds metadata. Reopen resets recency.

A read first pins its requested entry, then uses the shared reservation and reclamation path for
read memory, evicting unrelated unpinned files as needed. It returns an explicit `ArtifactLease`
pin with separately charged read memory. Eviction and `clear` skip pinned files; `clear` returns
the count still in use, and a later clear removes them after release. Dropping the cache keeps
outstanding reader bytes and disk charges alive through their leases; closed, unleased
directories are reaccounted on next open. OS and allocator overhead and unopened cache directories
are outside this accounting.

## ASR checkpoints

Independently complete ASR chunks commit as checkpoints, so a restart executes only the missing
chunks. Encoded-export resume returns a typed refusal in render: it would need independently
finalized segments and matching manifest, codec and source identity, and no partial encoded file
is a reusable artifact.

## Desktop maintenance

`maintenance` captures at most 256 registered ordinary entries and retains the root lock,
without IO or reader pins. `CacheMaintenance::run` measures file lengths including envelopes,
reading at most each header; unknown files, native bundles and substituted non-files remain
untouched. Clear shares atomic eviction with budget reclamation, skips live pins and retains
charges on failed unlink. Reports preserve prior removals, survivors and the first typed Runtime cause
on partial failure or cancellation; a missing pinned file has zero disk bytes but keeps its
charge and immutable read bytes until release and a later pass. Generation can repopulate a
snapshot. Root/lock identity checks refuse settled deletion or replacement, including writes
through a stale owner, without recreating it. Windows compares volume serial numbers and
128-bit file IDs through the safe `file-id` adapter, retaining the original directory and lock
handles for their ownership lifetime so IDs cannot recycle. Creation times are not identities;
unavailable or zero file IDs refuse with IO rather than falling back to timestamps or narrower
IDs. Unix keeps device/inode checks. These checks do **not** prevent hostile concurrent
ancestor replacement; directory-relative filesystem custody is outside this contract.
