# Analysis

Interface: `scene_detection`, `loudness`, `AnalysisRequest`, `AnalysisJob`.

`audio.set_loudness {scope: clip|master, target_lufs, true_peak_db}` stores an EBU R128 target (-50..-5 LUFS, -20..0 dBTP; default -14/-1) and drops stale gains. `audio.clear_loudness {scope}` removes it. Measured gains arrive through `audio.apply_loudness {gains}`, which validates range and ordering only. See [loudness normalization](#loudness-normalization).

## Loudness normalization

`loudness::submit_loudness(runner, cache, project, sources)` queues one CPU `MeasureLoudness` job that runs render's [two-pass planner](../../../render/src/audio/README.md#audio-clock-and-headroom) over the production mix graph for the targets set by `audio.set_loudness`. It reports `Mix` progress, honours cancellation, and surfaces failures as typed `JobFailure`. It never edits the project: `LoudnessJob::finish` validates the result (`LoudnessResult`: gains plus the measured numbers per clip and master), caches it as `ArtifactKind::Loudness`, and the caller applies it with `audio.apply_loudness`.

The gains' `key` is `loudness::content_key`: a hash of the active scene's audible items (timing, source content, volume/mute and their automation), track mutes, sample rate and both targets. The master fader is excluded because measurement ignores it. Identical content and targets hit the cache without decoding.

`submit_export*` calls `admit_loudness` (owned by `runtime/`, also exported as `loudness::admit`), which refuses `LoudnessStale` when a target is set and the stored gains are missing or measured for different audio, so preview (which applies stored gains as is) and export cannot disagree. Undo restores the gains with the content. Limits: 4,096 clips, 1 MiB result.

## Scene detection

`scene_detection::DetectionRequest::new(asset, source_range, DetectionSettings)` produces source-time `SceneCandidate`s with exact decoded PTS, `HardCut`/`Fade` and a normalized measured-change confidence (not a calibrated probability). `submit` runs one local CPU `DetectScenes` job; `DetectionJob::finish` validates and caches complete output as `SceneDetection`. Keys bind source content, source range, all thresholds and the versioned recipe. A warm hit opens no source. Analysis never touches a Session; cancellation before runner promotion leaves no cache entry. Cache pressure preserves the runner output for retry. Shot analysis invokes this same detector.

The 32x18 RGB sample grid uses 16 histogram bins per channel. A hard cut requires both normalized adjacent mean absolute RGB difference and normalized histogram L1 distance >= `hard_threshold` (default 0.28, admitted 0.01..=1). Gradual transitions collect a regular-PTS run of adjacent RGB changes >= `fade_step` (default 0.005, admitted 0.001..=0.1 and below hard threshold) and below hard threshold. Once a run ends, its end-to-anchor RGB change must reach `fade_threshold` (default 0.20, admitted 0.01..=1). Runs are limited to two seconds: an overlong or irregular-PTS run stays rejected until a quiet frame or hard cut ends it, so continuous motion cannot restart a fade anchor every two seconds. A fade's marker is the observed midpoint frame, earlier on ties. Hard cuts take priority over a preceding fade within the minimum spacing; the close fade is removed. Defaults leave at least 0.5 seconds from endpoints and previous markers (`min_scene`, admitted 0..=10 seconds); the first observed frame is an anchor, never a change candidate.

These heuristics can miss same-histogram cuts and long, subtle or irregular fades, and can mistake motion or lighting for a transition. Candidates require review; no general accuracy is claimed.

`apply_reviewed(session, item, result, selected_indexes)` validates explicit unique indexes, matching source identity and strict source-time item bounds. Project's retime mapping converts source to output ticks. Descending `SplitAt` commands keep the target ID in batches, grouped into one transaction and one undo step, each batch within the command limit. An empty selection does nothing; any refusal publishes nothing. Unselected candidates never create a split. The desktop owns the review surface.

Limits: one day, 16 Mi-pixel frames, 4095 candidates, 1 MiB serialized result. Decode streams through media's `decode_shot_frames` in <=300-second, <=100,000-frame slices including preroll. Only two fixed 1824-byte signatures and a bounded marker vector persist, so memory does not grow with duration. Jobs reserve 192 MiB for overlapping decode and conversion frames, 16 MiB workspace and 1 MiB retained output; these are reservations, not a native decoder RSS guarantee. Media keeps its native allocation and decoder ceilings, including refusal of geometry changes before packed conversion.

The Mac-only `duration_scaling_bounds_peak_memory` test measures resident memory of fresh subprocesses through `/usr/bin/time -l` for 300 versus 12,000 decoded 64×64 frames: each peak must stay below 192 MiB and growth within 16 MiB. Windows and Linux memory measurement is unverified.
