# shorts

Produces accepted shorts from source segments for desktop review, CLI clips and the fixed clip evaluation harness. Engine owns the edit kernel, provider selection, transcript acquisition, jobs, cache and typed execution errors. Domain crates own media processing, AI adapters, captions and rendering. This crate owns no UI, transport, model download or scheduler.

Use `clip_suggest::{suggest, prepare_export, export}` for the complete workflow, `clip_boundaries::snap_clip_boundaries` for reviewed source edges, or `ClipBatch` for explicit accepted clips. Callers supply engine resources, consent, progress and cancellation. `shorts::Error` owns boundary and export refusals and passes engine failures through; `BoundaryFailure` names the boundary reason.

| Module | Responsibility | Contract |
|---|---|---|
| `boundaries.rs` | Snaps ordered source edges to safe frames. | [Boundary adjustment](src/boundaries/README.md) |
| `suggestion/` | Produces source-snapped clip candidates. | [Suggestion workflow](src/suggestion/README.md) |
| `batch_export/` | Exports accepted shorts through bounded jobs. | [Batch export](src/batch_export/README.md) |

`cargo test -p shorts` runs the contracts. `cargo xtask clip-eval synthetic` compares the pipeline against its pinned labelled baseline.
