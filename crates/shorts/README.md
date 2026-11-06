# shorts

Produces accepted shorts from source segments for desktop review, CLI clips and
the fixed clip evaluation harness. Engine owns the edit kernel, provider selection,
transcript acquisition, jobs, cache and typed execution errors; domain crates own
media processing, AI adapters, captions and rendering. This crate owns no UI,
transport, model download or independent scheduler.

Use `clip_suggest::{suggest, prepare_export, export}` for the complete workflow,
`clip_boundaries::snap_clip_boundaries` for reviewed source edges, or `ClipBatch`
for explicit accepted clips. Callers supply engine resources, consent, progress
and cancellation. Failures retain the engine's typed error contract; its
`BoundaryFailure` payload belongs to that lower contract rather than this feature.

| Module | Responsibility | Contract |
|---|---|---|
| `boundaries.rs` | Snaps ordered source edges to safe frames. | [Boundary adjustment](src/boundaries/README.md) |
| `suggestion/` | Produces source-snapped clip candidates. | [Suggestion workflow](src/suggestion/README.md) |
| `batch_export/` | Exports accepted shorts through bounded jobs. | [Batch export](src/batch_export/README.md) |

`tests/it/shorts/` preserves the workflow, snapping and publication coverage.
`cargo test -p shorts` runs those contracts; `cargo xtask clip-eval synthetic`
compares the unchanged pipeline against its pinned labelled baseline. The
boundary benchmark remains in `examples/clip_boundary_benchmark.rs`.
