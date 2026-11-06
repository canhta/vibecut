# Per-shot analysis

## Per-shot analysis

`AnalysisRequest::new` binds a shared `SourceAsset`, explicit source
range, ordered disjoint source-time speech ranges, the loaded adapter's complete
`VisionFingerprint` bytes and `AnalysisSettings`. `submit` queues one local CPU
`AnalyzeShots` job through `JobRunner`; the composition root supplies the owned
`VisionDetector`. The optional `vision` feature permits composing the tract adapter.
There is no download, remote call, project edit, layout selection or fallback.

The request admits at most one day, 4096 detected shots, 64 tracks per shot,
16 Mi-pixel packed video frames and 32 MiB of serialized output. Work reserves
192 MiB for overlapping frames, 384 MiB for bounded crops, summaries and JSON,
and 32 MiB for retained output under the existing aggregate policy. Model weights
and adapter plans remain caller-owned; reservations are not an RSS guarantee.
Oversize admission is typed, rather than leaving an impossible job queued.

Cut discovery uses the shared [scene detector](../README.md#scene-detection) in its explicit
RGB-only hard-cut mode, preserving the previous adjacent mean-absolute RGB
threshold, per-channel floating normalization/accumulation order and minimum
shot length. This mode disables histogram gating and fade
tracking, so equal-histogram spatial cuts remain shot boundaries and fades cannot
suppress a real cut. Reviewed scene candidates use the histogram/fade policy below.
The detector recipe enters the shot cache version (v5); discovery still has one
implementation. A spatial half-frame swap with identical histograms is covered by
`same_histogram_spatial_cut_retains_rgb_only_shot_boundaries`.

Within each cut, frames are sampled on the source-audio-anchored 25 Hz grid by
presentation interval. YuNet runs every third sample (8.33 Hz); gated IoU >=0.3
matches boxes one-to-one within 0.4 s. Boxes are held between detections, never
interpolated across a missing observation. Tracks reset at every cut, have stable
shot-local integer IDs and require at least 13 observed samples (0.52 s).
Returned boxes use source pixels; cross-shot identity requires the optional
embedding evidence described below. A shot can refuse if it accumulates more than 64 tracks.

Each shot submits successive <=3 s ranges to `media::mfcc_range_with_progress`
on the same source asset. Its automatic cold-source pass reports `Preparing`,
then requested-range work reports `Analyzing`; the ordinary runner may coalesce
progress. The shared cancellation flag reaches media decoding directly, and both
MFCC callbacks check the job token. Preparation checkpoints never become retries
or enlarged shot budgets. Exact 100 Hz MFCC timestamps are joined four-to-one to
video crops. Only contiguous 25..=75-frame ASD runs are submitted; shorter tails,
missing audio and track gaps are not padded. Inference cancellation/errors abort
without publishing partial scores.

One-second bins and whole shots aggregate +/-2-frame-smoothed ASD probabilities.
A new visible speaker requires mean >=0.5; within one shot a previously visible
track can persist at >=0.4 for the next bin. Ties choose the lower track ID.
People counts use tracks present for at least half a second in a bin (all samples
for a shorter tail); the shot count is the median bin count. The face gate yields
camera evidence; faceless frames run CLIP at most once per second. Camera/screen
votes determine the shot class, with camera on a tie and Unknown without evidence.
These heuristics do not inherit the /12 research corpus's accuracy measurements.

`SpeakerState` distinguishes Visible (a shot-local reference), OffScreenSpeech
and Unknown. OffScreenSpeech requires supplied speech evidence plus audio and
adequate negative evidence: either no observed faces, or ASD scores covering every
observed face sample. Face presence and score coverage include short tracks removed
by the people-count dwell filter. Incomplete coverage keeps the affected bin and
whole shot Unknown even if another visible track has negative scores; a confirmed
visible speaker still takes precedence. Missing audio or insufficient ASD evidence remains
Unknown. Speech overlap is not inferred from a face detector or arbitrary noise.
`continuity_hint` retains the last confirmed shot-local reference across cuts,
including listener shots. It never assigns that reference to another shot's face;
cross-shot identity is carried separately by `FaceTrack::identity`.

When an embedding adapter is attached, at most three fresh aligned observations
per track (about one second apart) are sampled. Missing/poor landmarks or an
unavailable adapter leave no descriptor. Matching compares all sampled crops
against fixed earlier anchors: every cosine must be at least 0.30 and the winner
must exceed every alternative anchor by at least 0.08. Two simultaneous tracks
claiming one anchor are both Unknown; a shot's tracks never match each other.
Unmatched tracks seed separate anchors for future observations but their public
identity remains None (Unknown). Confirmed matches never update anchor embeddings,
preventing prediction feedback and identity drift. Layout consumers resolve a
visible speaker's local reference through its shot's track identity, rather than
reinterpreting the continuity hint. No face identity implies speech activity.

The transient gallery admits 1024 anchors with at most three descriptors each;
excess is a typed Limit. Embeddings are never serialized; only confident anchor
references enter artifacts. Version 5 and the recipe invalidate old cached output.
Reopened identities must refer to a prior existing anchor, never a future/current
shot or another predicted match, and cannot be shared by simultaneous tracks.
The 0.30 threshold uses a small agent-labelled interview calibration; it is a
conservative sample-specific operating point, not a general face accuracy claim.

`AnalysisJob::finish` returns None while pending, propagates typed terminal
failures/cancellation, validates a complete retained result, then stores it in the
existing `ArtifactCache`. Cache pressure leaves the runner output available for
another finish attempt. Cancelling before runner promotion leaves no cache entry;
no partial shot or checkpoint is persisted. `ShotAnalysis` keys bind source
content, exact requested range, speech evidence, detector/model fingerprint,
settings and analysis recipe. Reopened payloads additionally validate their bound
fingerprints, partition, tracks, probabilities and continuity hints; stale payloads
are `CacheCorrupt`, never accepted as a hit. Changed keys recompute. Existing cache
LRU/accounting and corruption diagnostics retain ownership.

`cargo test -p engine --test it shot_analysis::` uses labelled synthetic cuts and injected
inference to exercise jobs/cache/failures. The ignored `real_vision_analysis_smoke`
requires `--features vision`, `VIBECUT_VISION_MODELS` (the verified catalog-named
files), `VIBECUT_VISION_SOURCE` (the permitted bounded source) and optional integer
`VIBECUT_VISION_START`/`VIBECUT_VISION_DURATION` in seconds. It prints only aggregate
analysis. Cross-shot matching has focused uncertainty/collision/drift tests and
an injected job/cache composition test. media's `identity_prepare` and ai's
`face_embedding` example measure the actual media/tract adapters on local labelled
boundary crops without logging private media. Local measurements record per-boundary
agent-labelled results and rejection coverage. Seven-layout integration and general
speaker/cut accuracy remain separate acceptance, not established by a green build.
