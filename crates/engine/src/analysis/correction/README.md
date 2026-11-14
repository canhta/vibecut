# Footage correction

Interface: `CorrectionRequest`, `CorrectionJob`, `Correction`, `resolve`, `Resolution`,
`CorrectionKind`, `Segmenter`, `PERSON_MODEL`, `SEGMENTATION_THREADS`.

A correction is derived data per source frame, computed once by an engine job and kept in the
artifact cache. It is not project data and not keyframes: it is dense and reproducible. The item
carries only the switch and its setting (`VideoParams.stabilize` with its level,
`VideoParams.background_removal` with its feather, `VideoParams.remove_flicker`); render looks the correction up and applies it
inside the item's own picture ([render](../../../../render/src/scene/correction/README.md)). Three
kinds exist, stabilize, the person matte and flicker; a kind is added by a recipe and a render
application, not by a new store.

## Contract

- `CorrectionRequest::new(asset, kind)` admits a source with a video stream of known duration and
  a frame of at most 16 Mi pixels that holds at least one analysis block (56 grid pixels square).
  The cache key (`ArtifactKind::FootageCorrection`) binds the source content hash, the kind, the
  whole-source range and the versioned recipe; the person matte's key also binds the pinned
  weights' SHA-256, `ai::vision::PERSON_RECIPE`, `render::matte::RECIPE` and the fixed
  `SEGMENTATION_THREADS` (2). Trim, split, retime, level, feather and item position never enter
  it, so none needs a new analysis; a source replaced by another file has another hash and so a
  new key. A stabilize source of more than 262,144 frames (2.4 hours at 30 fps) refuses with
  `Limit`; a person matte of more than 65,536 frames (36 minutes at 30 fps) or 128 MiB fails the
  job.
- `submit(runner, cache, segmenter)` queues one local CPU `AnalyzeFootage` job
  (`footage.analyze {item, kind}`) that decodes the whole video once in slices of at most 300 s
  and reports `Analyzing` progress. A warm entry completes without opening the source or loading a
  model. A cold person matte needs the lent `Segmenter` and refuses with the typed
  `ModelMissing("pphumanseg-2023mar")` without one; `MediaAccess` loads it from the root's
  `SegmentationAccess` only when the entry is cold. `CorrectionJob::finish` validates the retained
  bytes and publishes the cache entry; the control dispatcher calls it where `job_status` first
  sees the job complete. A cancelled or failed job publishes nothing. The command edits no
  project.
- `cached` returns the stored `Correction`. A corrupt or undecodable entry counts as absent, so
  the next analysis replaces it.
- `resolve(project, cache)` reads the cache only, for every switched-on correction of every video
  item of every scene, and never decodes media. `Resolution.tracks` and `Resolution.mattes` go to
  render through `Resolution::attach` (`RenderSettings.stabilization` and `.person_mattes`);
  `Resolution.missing` lists each switched-on `(item, kind)` without a stored correction. Those
  items render without that correction, the properties row shows `Needs analysis` with a button,
  and export refuses to start naming them ([admission](../../runtime/README.md)). Items over one
  source share one decoded correction. `Lookup` is the same `resolve` over a shared media cache,
  lent to the stores that capture project covers; `uncorrected()` is the lookup of a process with
  no cache. Snapshot, freeze-frame and cover renders take the resolution the same way preview does.

## Stabilize

Method from the footage clean-up study: the decoded frame is box-averaged to a 480-wide luma grid (a narrower
source keeps its width); 32x32 blocks on a 32-pixel pitch with a 12-pixel margin are matched
against the previous frame by exhaustive SAD over +/-8 grid pixels (about 32 pixels at 1080p) with
a parabola for the sub-pixel position. A block is dropped when its SAD surface is flat (mean below
2 per pixel), ambiguous (best above 0.6 of the mean) or best on the search edge. The surviving
block displacements fit a four-parameter similarity (translation, rotation, scale) by
least squares with three rounds of dropping points whose residual exceeds three times the median
(at least 0.15 px). Fewer than six blocks, or a singular fit, leave the frame at the previous pose
(identity step). Steps chain into one pose per frame measured against the first frame about the
frame centre (16 bytes per frame, about 29 KB per minute at 30 fps); smoothing and the level are
applied at lookup, so changing the level costs no analysis. Integer SAD and a fixed evaluation
order make two runs byte-equal; the analysis is single-threaded, so thread count cannot change a
result.

Measured on the study's synthetic handheld fixture (4.34 px of corner motion at 1080p): 0.60 px after
correction at the Normal level, 1.6 ms per frame at 1080p on one thread. The oracle bound is 1.0 px
at 1080p and is asserted in the engine's footage correction test. Not measured: real
handheld footage, moving subjects, rolling shutter, motion beyond +/-32 px per frame at 1080p
(needs a pyramid), drift over minutes (harmless once smoothed; a hold-still mode would need
re-anchoring and is not built).

Levels map to the Gaussian width of the smoothing: Flexible 0.13 s, Normal 0.4 s, Stable 1.2 s
(4, 12 and 36 frames at 30 fps from the study's measurements). The reference names the three levels and
shows no numbers, so the widths are our choices, not parity.

## Flicker

`flicker.rs` decodes every frame once, in slices of at most 300 s, and stores per frame the mean
linear luma and whether the frame starts a new shot (13 bytes per frame with its time, about 2.3 MB
per hour at 30 fps). The mean is an integer sum of Rec. 709 weights (54, 183, 19 of 256) over a
table of 16-bit linear samples of the 256 sRGB values, so a frame always gives the same mean. The
shot change score is the mean absolute difference of two consecutive frames' 80x45 box-averaged
luma grids, each divided by its own mean, so a pure gain change never scores; above 0.25 (the
study measured 0.49-0.68 on cuts and at most 0.10 elsewhere) the frame starts a shot, and black next to a lit
frame counts as a cut while two black frames do not. The gain, its window and its clamp are applied
at lookup from the stored means, so no level or window setting needs a new analysis; the method and
its oracles are in [render](../../../../render/src/scene/correction/README.md#flicker). The key
binds the source content hash, the kind, the whole-source range and the two recipes; it needs no
model. A flicker source of more than 262,144 frames fails the job. Memory is one decoded frame and
two grids. Measured oracles, the recorded bounds and what is not measured:
`tests/it/analysis/footage_correction.rs`. Not measured here: time per minute of 1080p (the study
estimated 6 s per minute on its host, decode-bound), real footage.

## Person matte

`person.rs` decodes every frame once, prepares the 192x192 person tensor
(`media::VisionFrame::person_tensor`), runs the lent segmenter (the pinned PP-HumanSeg weights on
the admitted tract runtime, [card, licence and shapes](../../../../ai/src/foundation/models/README.md))
and stores its plane quantized to one byte per cell, PackBits-compressed per frame
(`render::matte::MatteBuilder`). Memory is one decoded frame (at most 16 Mi pixels, so 4K fits),
one 110,592-value tensor and the compressed planes so far; cancellation is checked per frame and
inside inference. The same pinned weights, thread count and source give byte-equal output, so two
uncached runs retain the same bytes. The model is never downloaded here: the composition roots lend
`control::SegmentationAccess` over their one verified models cache (`shorts::installed_person_matte`),
and the model manager owns download, consent and integrity. The feather is applied at render, so
changing it costs no analysis. Model accuracy evidence: the matte IoU fixtures in render's
`tests/it/matte.rs` and the segmentation layer's ignored real-weights run.
