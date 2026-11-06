# Long video to shorts


The Home `home.shorts` card opens a separate native viewport. `Shorts` owns its
bounded presentation/review history; `LocalVideoWorkflow` composes the [shorts workflow](../../../../crates/shorts/README.md)
through `clip_suggest::suggest` against the picked/dropped local video. Provider and model
pickers use offline capability data. Local heuristics are an explicit choice;
remote ASR and suggestions stop at an exact per-upload disclosure (provider,
model, language, audio duration, payload bytes, cost ceiling and policy links).
Refusal or cancellation sends no approval and never selects another provider.
Missing models and credentials are actionable errors; no implicit downloads occur.

Candidates show title, duration, editorial score, reason and heuristic/AI source.
The shared catalog formats metadata and numeric fields; trim values parse the
same en/vi decimal and grouping notation, refusing malformed or mixed input.
Changing the runtime locale refreshes trim drafts without changing source ticks.
Review retains roles and both play/source ranks, safe shorts boundary snapping,
accept/reject and a source-normalized 9:16 crop. The reusable `crop` control moves
within source bounds, with keyboard arrows and Escape rollback. Review actions
have a separate 100-step undo/redo history; a whole crop gesture is one step.
`ClipBatch` produces the exact concatenated snapshot and crop placements used by
both preview and export, so playback skips source gaps and follows play order.
Only accepted shorts submit batch jobs, with independent progress/cancel/retry;
editing a review does not alter an already submitted immutable batch.

Open in editor inserts every video segment and mapped source-caption item into a
fresh portrait project through one engine transaction, retaining the prepared
media's original dimensions while replaying items so first-import initialization
preserves the explicit portrait canvas, then uses the existing
nonblocking recovery/autosave owner. Editor preview and export share the same
transient render-state owner, resolving
placements against each immutable snapshot so deleting items and undo/redo do
not leave stale placement IDs or recenter editor exports. Review crops are
transient (preview/export);
persisted crop channels and subsequent full-editor crop edits belong to the crop
controls feature. Source-language captions can be burned and exported as SRT.
`tests/it/shorts_ui.rs` injects generation and preview sources, uses real automation
IDs and engine commands, and asserts state only. UI image evidence is an offscreen
audit, separate from the test suite; no native window is opened by verification.

`cargo xtask ui-audit shorts-pick shorts-setup shorts-review shorts-pick-vi
shorts-setup-vi shorts-review-vi` captures the three Shorts screens in en/vi,
dark/light, at 1x/2x and the minimum 1280×800 logical size. These shared scenes
use injected candidates and synthetic frames by default; `SHORTS_AUDIT_VIDEO`
optionally selects one local audit source, imported once for the run. The audit
is separate from tests and never makes provider calls.
