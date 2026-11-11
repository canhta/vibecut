# shorts

Produces accepted shorts from source segments for desktop review, CLI clips and the fixed clip evaluation harness. Engine owns the edit kernel, provider selection, transcript acquisition, jobs, cache and typed execution errors. Domain crates own media processing, AI adapters, captions and rendering. This crate owns no UI, transport, model download or scheduler.
Cue-session construction returns Kernel Edit errors; translation returns Transcript errors.
Shorts composes each through its existing engine aggregate boundary.

Use `clip_suggest::{suggest, prepare_export, export}` for the complete workflow, `clip_boundaries::snap_clip_boundaries` for reviewed source edges, or `ClipBatch` for explicit accepted clips. Callers supply engine resources, consent, progress and cancellation. `shorts::Error` owns boundary and export refusals and passes engine failures through; `BoundaryFailure` names the boundary reason.

| Module | Responsibility | Contract |
|---|---|---|
| `boundaries.rs` | Snaps ordered source edges to safe frames. | [Boundary adjustment](src/boundaries/README.md) |
| `suggestion/` | Produces source-snapped clip candidates. | [Suggestion workflow](src/suggestion/README.md) |
| `batch_export/` | Exports accepted shorts through bounded jobs. | [Batch export](src/batch_export/README.md) |

Verification and the labelled acceptance path are owned by [verification](../../docs/engineering/verification.md#checks) and [clip-eval](../clip-eval/README.md).
