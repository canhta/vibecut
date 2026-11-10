# Per-shot analysis

`AnalysisRequest::new` binds a shared `SourceAsset`, an explicit source range, ordered disjoint
source-time speech ranges, the loaded adapter's complete `VisionFingerprint` bytes and
`AnalysisSettings`. `cut_threshold` admits 0.01..=1 (default 0.28) and `min_shot` 0.04..=10 s
(default 0.5 s). Speech ranges must be ordered, disjoint and inside the requested range; without
speech evidence an absent visible speaker is Unknown, never a speech claim.

`submit` queues one local CPU `AnalyzeShots` job through `JobRunner`; the composition root
supplies the owned `VisionDetector`, and the optional `vision` feature permits composing the
tract adapter. There is no download, remote call, project edit, layout selection or fallback.

Limits: one day, 4096 detected shots, 64 tracks per shot, 16 Mi-pixel packed video frames and
32 MiB of serialized output. Work reserves 192 MiB for overlapping frames, 384 MiB for bounded
crops, summaries and JSON, and 32 MiB for retained output. Model weights and adapter plans remain
caller-owned; reservations are not an RSS guarantee. Oversize admission is a typed refusal, and a
shot refuses if it accumulates more than 64 tracks.

Cut discovery uses the shared [scene detector](../README.md#scene-detection) in its RGB-only
hard-cut mode, which disables histogram gating and fade tracking, so equal-histogram spatial cuts
remain shot boundaries and fades cannot suppress a real cut. Reviewed scene candidates keep the
histogram and fade policy.

Each shot reports source-pixel `FaceTrack` boxes with stable shot-local integer IDs, a
`people_count`, a `SpeakerState`, a `Scene` and one `SecondAnalysis` per second of its range.
`SpeakerState` is Visible (a shot-local reference), OffScreenSpeech or Unknown. OffScreenSpeech
requires supplied speech evidence plus audio and adequate negative evidence: either no observed
faces, or ASD scores covering every observed face sample. Incomplete coverage keeps the affected
bin and whole shot Unknown even if another visible track has negative scores; a confirmed visible
speaker still takes precedence, and missing audio or insufficient evidence stays Unknown. Speech overlap is never inferred from a face detector or arbitrary noise.
`continuity_hint` retains the last confirmed shot-local reference across cuts, including listener
shots, but never assigns that reference to another shot's face; cross-shot identity is carried by
`FaceTrack::identity`.

With an embedding adapter attached, the matcher compares sampled crops against fixed earlier
anchors: every cosine must be at least 0.30 and the winner must exceed every alternative anchor
by at least 0.08. Two simultaneous tracks claiming one anchor are both Unknown, a shot's tracks
never match each other, and unmatched tracks seed separate anchors while their public identity
stays None. Confirmed matches never update anchor embeddings; embeddings are never serialized and
only confident anchor references enter artifacts. The transient gallery admits 1024 anchors with
at most three descriptors each, and excess is a typed `Limit`. Reopened identities must refer to a
prior existing anchor, never a future or current shot or another predicted match, and cannot be
shared by simultaneous tracks. Layout consumers resolve a visible speaker's local reference
through its shot's track identity, not the continuity hint.

`AnalysisJob::finish` returns None while pending, propagates typed terminal failures and
cancellation, validates a complete retained result, then stores it in `ArtifactCache`. Cache
pressure leaves the runner output available for another finish attempt; cancelling before runner
promotion leaves no cache entry. `AnalysisRequest` keys bind source content, exact requested
range, speech evidence, detector and model fingerprint, settings and analysis recipe. Reopened payloads
validate their bound fingerprints, partition, tracks, probabilities and continuity hints, and a
stale payload is `CacheCorrupt`, never a hit.
