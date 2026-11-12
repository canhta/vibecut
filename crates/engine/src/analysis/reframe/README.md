# Reframe analysis

Turns a source range into a subject path in source ticks and, for one timeline item, into
`crop.x` / `crop.y` / `crop.auto` keys. A library only: it names no command, edits no project and
downloads nothing. Face following is not speaker awareness; nothing here or in its copy claims it.

## Contract

- `ReframeRequest::new(asset, range, target, models, AnalysisSettings)` binds the video source,
  an explicit range of at most one hour, a target (`9:16`, `1:1`, `16:9`), the loaded adapter's
  complete `VisionFingerprint` and the shot-analysis thresholds (`AnalysisSettings`, so cut
  discovery matches shot analysis). `submit` queues one local CPU `AnalyzeReframe` job and
  `ReframeJob::finish` validates, caches and returns the `ReframeAnalysis`; `submit_for_session`
  binds the job to one Session revision and `publish` validates and caches without consuming its
  retained output, which `reframe.apply {job}` reads. Cancel and progress travel through the job
  context, which is the detector's `ExecutionHooks`. The composition root supplies the
  `VisionDetector` (tract YuNet, already pinned in the model catalog; download is a user action).
- `emit_reframe(&analysis, &item)` is pure. It converts the shots under the item to the key set
  of the design (shot = span between `crop.auto` keys): `crop.x` and `crop.y` scalar keys, one `crop.auto` `Step` key (true) at
  the item start and at every cut inside it, and the static `Crop` that carries the constant
  window size with the first position. Output ticks are item-relative: `Item::output_time_of`
  (ceiling) for samples and cuts, `Item::source_time_at` (floor) for the start value, so a trim, a
  speed change or a second item over the same span each get their own keys from one analysis. The
  last key before a cut is `Step`; nothing interpolates across a cut. A shot shorter than one
  output tick gets no keys.
- Key reduction: in one forward pass per channel and shot, a key is dropped when
  removing it (with the keys already dropped since the last kept one) moves the linear path by
  less than half a source pixel. Shot ends are always kept. More than 4096 keys in a channel is
  `EditError::Limit`, never a truncation.

## Pipeline

1. Cuts: the shared scene detector in hard-cut mode, as shot analysis uses it. Every shot is
   analysed alone; no path crosses a cut.
2. Sampling: every 1/8 s of source time from the shot start, each sample taking the decoded frame
   whose presentation interval contains it (`media::decode_shot_frames`, slices of at most 300 s).
   One retained frame and one 640x384 `face_tensor` are resident at a time.
3. Tracking: boxes associate to tracks by IoU of at least 0.2 (the measured R1 harness value) with
   a gap of up to 0.5 s. At most 64 tracks per shot.
4. Target: a track is stable at 4 samples (0.5 s), or half of a shorter shot. The largest stable
   track by mean box area is followed (ties: the earlier). Two stable tracks on at least 2 common
   samples give `ambiguous` and still follow the largest. No stable track gives `no_subject` and a
   centred crop. The followed face is held up to 0.5 s after it vanishes, then the target moves to
   the centre (`lost`). Before it first appears the crop waits where it will be.
5. Smoothing (R1): positions in source pixels; no movement within 12 px, otherwise a step of
   `1 - exp(-dt/0.35)` of the gap, capped at 768 px/s; run forward and then backward over its own
   output, each end extended by 12 samples of odd reflection so the passes settle before the shot.
   The first sample of a shot is its target. The window is the largest of the target aspect that
   fits the frame, constant for the whole analysis; positions clamp into `[0, 1 - size]`.

## Cache

`ArtifactKind::ReframeAnalysis`, keyed by source content hash, exact range, target, cut threshold,
minimum shot, model fingerprint, the scene-detection recipe and the reframe recipe version. A
warm hit opens no source and never calls the detector. A reopened payload is validated (sizes,
partition, sample times, bounds); a stale one is `CacheCorrupt`, never a hit. Result limit 8 MiB.

## Measured

Fixture: generated 1280x720 25 fps, four shots divided by black/grey cuts; the first is a 160 px
square sweeping `640 + 400 sin(2 pi t / 6)` (peak 419 px/s, 2.6 box widths per second), 9:16 target
(405 px window). Evaluated by `Crop::at` on the emitted keys at all 150 output frames
(`tests/it/analysis/reframe.rs`):

| Oracle | Measured | Bound asserted |
|---|---|---|
| Subject centre inside the central 80 % of the window (+-162 px) | 150 of 150 frames | at least 95 % |
| Worst offset of window centre from subject centre | 50.9 px | at most 60 px |
| p95 adjacent-frame step of the window, fraction of source width | 0.01137 | at most 0.012, and not above the subject's own 0.01302 |

Shot reasons on the same fixture: sweeping subject none, two squares `ambiguous` (follows the
140 px one), empty shot `no_subject`, subject gone after 2 s `lost`. Two analyses are byte-equal.
Windows only; speed and other platforms are unverified (R3 stays research).
