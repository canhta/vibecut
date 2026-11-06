# Analysis

Interface: `scene_detection`, `loudness`, `AnalysisRequest`, `AnalysisJob`.

`audio.set_loudness {scope: clip|master, target_lufs, true_peak_db}` stores an EBU R128
target (-50..-5 LUFS, -20..0 dBTP; default -14/-1) and drops stale gains;
`audio.clear_loudness {scope}` removes it; both are ordinary undoable edits that
persist with the project. Measured gains arrive through `audio.apply_loudness {gains}`
(validated range and ordering only). See [loudness normalization](#loudness-normalization).
All three edits use ordinary gestures, transactions, undo/redo and persistence;
render owns mastering, sample timing and latency compensation.

## Loudness normalization

`loudness::submit_loudness(runner, cache, project, sources)` queues one CPU
`MeasureLoudness` job that runs render's [two-pass planner](../../../render/src/audio/README.md#audio-clock-and-headroom)
over the production mix graph for the targets set by `audio.set_loudness`. It reports
`Mix` progress, honours cancellation, and surfaces failures as typed
`JobFailure`. It never edits the project: `LoudnessJob::finish` validates the result
(`LoudnessResult`: gains plus the measured numbers per clip and master), caches it as
`ArtifactKind::Loudness` and the caller applies it with `audio.apply_loudness`.
The gains' `key` is `loudness::content_key`: a hash of the active scene's audible items
(timing, source content, volume/mute and their automation), track mutes, sample rate
and both targets. The master fader is excluded because measurement ignores it.
Identical content and targets hit the cache without decoding. `submit_export*` calls
`admit_loudness` (owned by `runtime/`, also exported as `loudness::admit` with `loudness::content_key`), which refuses `LoudnessStale` when a target is set and the stored
gains are missing or measured for different audio, so preview (which applies stored
gains as is) and export cannot disagree. Undo restores the gains with the content.
`vibecut-cli normalize` composes these steps. Limits: 4,096 clips, 1 MiB result.

## Scene detection

`scene_detection::DetectionRequest::new(asset, source_range, DetectionSettings)`
produces source-time `SceneCandidate`s with exact decoded PTS, `HardCut`/`Fade`
and a normalized measured-change confidence (not a calibrated probability).
`submit` runs one local CPU `DetectScenes` job through the ordinary job runner;
`DetectionJob::finish` validates and caches complete output as `SceneDetection`.
Keys bind source content, source range, all thresholds and the versioned recipe.
A warm hit opens no source. Analysis never touches a Session; cancellation before
runner promotion leaves no cache entry or timeline edit. Cache pressure preserves
the runner output for retry. Shot analysis, used by clip layout consumers, invokes
this same detector. No model, download or new split command is introduced.

The 32x18 RGB sample grid uses 16 histogram bins per channel. A hard cut requires
both normalized adjacent mean absolute RGB difference and normalized histogram
L1 distance >= `hard_threshold` (default 0.28, admitted 0.01..=1). Gradual transitions
collect a regular-PTS run of adjacent RGB changes >= `fade_step` (default 0.005,
admitted 0.001..=0.1 and below hard threshold) and below hard threshold. Once a
run ends, its end-to-anchor RGB change must reach `fade_threshold` (default 0.20,
admitted 0.01..=1). Runs are limited to two seconds: an overlong or irregular-PTS
run remains rejected until a quiet frame or hard cut ends it, preventing continuous
motion from restarting a fade anchor every two seconds. Its marker is the observed
midpoint frame, earlier on ties. Hard cuts take priority over a preceding fade
within the minimum spacing; the close fade is removed instead of suppressing the
hard cut. Defaults leave at least 0.5 seconds from endpoints and previous markers
(`min_scene`, admitted 0..=10 seconds); the first observed frame is an anchor,
never a change candidate. Recipe v2 invalidates the previous scene artifacts.
These deterministic heuristics can miss same-histogram cuts and long, subtle or irregular
fades, and can mistake motion or lighting for a transition; candidates require
review. They are not a general cut/dissolve accuracy claim.

`apply_reviewed(session, item, result, selected_indexes)` validates explicit unique
indexes, matching source identity and strict source-time item bounds. Project's
existing retime mapping converts source to output ticks. Descending ordinary
`SplitAt` commands retain the target ID in existing batches, grouped into one
transaction and one undo step. Each batch respects the existing command limit. Empty
selection does nothing; any refusal publishes nothing. No unselected candidate
creates a split. The desktop owns the review surface.

Limits: one day, 16 Mi-pixel frames, 4095 candidates, 1 MiB serialized result. Decode streams through media's admitted
`decode_shot_frames` in <=300-second, <=100,000-frame slices including preroll.
Only two fixed 1824-byte signatures and a bounded marker vector persist; no frame
history accumulates with duration. Jobs reserve 192 MiB for overlapping decode /
conversion frames, 16 MiB workspace and 1 MiB retained output; these aggregate
reservations are not a native decoder RSS guarantee. Media retains its native
allocation/decoder ceilings, including refusal of geometry changes before packed
conversion. `cargo test -p engine --test it scene_detection::` covers labelled
hard cuts and fade midpoint, independent-run determinism, reviewed subsets,
undo/redo, queued and gated mid-decode cancellation, missing-source cache reuse,
empty/single-frame/constant sources, hostile geometry and dimension/duration scaling.

The Mac-only `duration_scaling_bounds_peak_memory` integration test measures fresh
test subprocesses through `/usr/bin/time -l`, excluding fixture generation. It
checks 300 versus 12,000 decoded 64×64 frames: each peak must stay below 192 MiB
and growth within 16 MiB. This tests resident memory, separately from runner
reservations. A temporary full-frame-retention mutation is rejected by this test;
the fix report contains its reproduction and RSS values. The checked-in test is
reproducible from the branch; Windows/Linux memory measurement remains unverified.
The edge tables cover an exact 300-second cut, adjacent/interrupted fades,
first/last-frame spacing, irregular PTS and every cache-key setting.
