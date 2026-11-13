# Long video to shorts

Interface: `State`, `show`, `Action`, `Context`, `WINDOW`, `ClipCandidate`, `Decision`,
`GenerationRequest`, `Importer`, `desktop_importer`, `Review`, `ReviewSource`, `TaskEvents`.

The Home `home.shorts` card opens a separate native viewport. `show` draws the whole
window and returns the `Action` values the composition root must carry out; `State` owns
bounded presentation and review history. Generating sends the registry command `clips.suggest`
through `services::ai_access` (the [shorts workflow](../../../../crates/shorts/README.md) runs
it as a runner job) on the picked or dropped local video, imported through the injectable
`Importer`; the retained result is decoded into the review. The
window owns no credentials, cache, `Providers` or workflow trait: tests inject the access
opener. Provider and model pickers use offline capability
data; local heuristics are an explicit choice.

Remote ASR and suggestions stop at an exact per-upload disclosure (provider, model,
language, audio duration, payload bytes, cost ceiling, policy links); refusal or
cancellation sends no approval and never selects another provider. Missing models and
credentials are actionable errors; nothing downloads implicitly.

Candidates show title, duration, editorial score, reason and heuristic/AI source. The
shared catalog formats metadata and numbers; trim values parse the same en/vi decimal
and grouping notation and refuse malformed or mixed input. A runtime locale change
refreshes trim drafts without changing source ticks.

Review keeps roles and both play and source ranks, safe boundary snapping, accept and
reject, and a source-normalized 9:16 crop. The reusable `crop` control moves within
source bounds, with arrow keys and Escape rollback. Review actions have a separate
100-step undo and redo history; a whole crop gesture is one step. `ClipBatch` produces
the exact concatenated snapshot and crop placements used by both preview and export, so
playback skips source gaps and follows play order. Only accepted shorts submit batch
jobs, each with independent progress, cancel and retry; editing a review does not alter
an already submitted immutable batch. Source-language captions can be burned and
exported as SRT.

Layouts: after a suggestion the window asks the engine for the per-shot analysis of the
review's pieces through `engine::control::resolve_clip_shots` (the same reuse-or-start as
`clips.export`, on the window's own runner; `State::lend_analysis` is the composition
root's seam and `with_shot_source` the test seam) and passes the shots to every
`ClipBatch::prepare`, so the preview and the export lay shorts out alike. While the
analysis job runs the preview has no evidence (centred Fill) and `tick` asks again once
the job is terminal; a failed job is reported, never resubmitted. Without a detector,
model weights or a working analysis the typed reason shows once in the toast and can be
dismissed. The desktop lends the process media cache and `shorts::installed_faces`, the
installed local detector (never downloaded here), so absent weights report as missing. A short's review crop still replaces the detected
speaker crop of its Fill pieces; other layouts ignore it.

Preview and every export share the one transient render-state owner that resolves
placements against each immutable snapshot, so deleting items and undo or redo leave no
stale placement IDs. Open in editor inserts every video segment and mapped
source-caption item into a fresh
portrait project in one engine transaction, keeping the prepared media's original
dimensions while replaying items so first-import initialization preserves the explicit
portrait canvas, then hands off to the nonblocking recovery and autosave owner. The
window plays the selected short from its own prepared snapshot through
`services::playback`, `services::preview` and `services::progress`. Review crops are
transient; persisted crop channels and later editor crop edits belong to the crop
controls.

Geometry: [Long video to shorts window](../../../../docs/product/ui/shorts.md).

Provider choice/credential/consent operations return their provider-owned result. The review
borrows raw or engine-composed provider refusals for existing setup/consent labels and explicit
Runtime resource categories; opaque failures are not searched for an inner category.

Synchronous acquisition retains the Transcript owner through its engine composition. Review
borrows its explicit provider and Runtime resource categories alongside genuinely mixed
workflow errors, without walking sources or peeling opaque failures.
