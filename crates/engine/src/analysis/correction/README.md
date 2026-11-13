# Footage correction

Interface: `CorrectionRequest`, `CorrectionJob`, `resolve`, `Resolution`, `CorrectionKind`.

A correction is derived data per source frame, computed once by an engine job and kept in the
artifact cache. It is not project data and not keyframes: it is dense and reproducible. The item
carries only the switch and its level (`VideoParams.stabilize`); render looks the correction up
and applies it inside the item's own picture ([render](../../../../render/src/scene/README.md#stabilization)).
Flicker removal is the second kind and reuses this contract: a kind is added by a recipe and a
render application, not by a new store.

## Contract

- `CorrectionRequest::new(asset, kind)` admits a source with a video stream of known duration and
  a frame of at most 16 Mi pixels that holds at least one analysis block (56 grid pixels square).
  The cache key (`ArtifactKind::FootageCorrection`) binds the source content hash, the kind, the
  whole-source range and the versioned recipe. Trim, split, retime, level and item position never
  enter it, so none needs a new analysis; a source replaced by another file has another hash and
  so a new key. A source of more than 262,144 frames (2.4 hours at 30 fps) refuses with `Limit`.
- `submit` queues one local CPU `AnalyzeFootage` job (`footage.analyze {item, kind}`) that decodes
  the whole video once in slices of at most 300 s and reports `Analyzing` progress. A warm entry
  completes without opening the source. `CorrectionJob::finish` validates the retained bytes and
  publishes the cache entry; the control dispatcher calls it where `job_status` first sees the job
  complete. A cancelled or failed job publishes nothing. The command edits no project.
- `cached` returns the stored track. A corrupt or undecodable entry counts as absent, so the next
  analysis replaces it.
- `resolve(project, cache)` reads the cache only, for every switched-on video item of every scene,
  and never decodes media. `Resolution.tracks` is ready for `RenderSettings.stabilization`;
  `Resolution.missing` lists switched-on items without a stored correction. Those items render
  uncorrected, the properties row shows `Needs analysis` with a button, and export refuses to
  start naming them ([admission](../../runtime/README.md)). Items over one source share one decoded
  track.

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
