# Model manager dialog

Interface: `State`, `Action`, `ModelManager`.

`State` owns at most one `ModelManager` and the window's visible flag; `show` paints
the manager when it is visible and returns remember requests the frame routes through
the shared write context. `Editor::open_models` and `Editor::open_model` open it, and
the workspace lends the one app-wide instance to the focused window.

`ModelManager` opens one modal over `ai::foundation::models::catalog` and engine's
`ModelTransfers`. It shares [engine's default model path](../../../../../../crates/engine/src/runtime/README.md#default-cache-location)
with the CLI and adds no second downloader, checkpoint or cache. Rows show model id, language scope
(including language-neutral weights), licence and source links, total and per-file
size and SHA-256, install state and current job stage and progress.

Only MIT, Apache-2.0 and BSD weights can carry the Suggested badge; any other licence
shows its restriction and needs an explicit review checkbox before Download enables.
A fresh manager hashes installed bundles in a background check; dropping the manager
cancels and joins it before releasing cache ownership. Opening, selecting a model,
accepting a licence or starting transcription never starts a transfer: the affirmative
Download click is the only place a `DownloadConsent` is granted. Closing during a
transfer is refused; Cancel leaves the engine checkpoint for Resume. The composed runtime
`CacheBusy` refusal keeps the existing Busy presentation and starts no second transfer;
releasing the owning transfer permits a later explicit Download. Installed-file verification
retains the original AI cause in `RuntimeError::Model`; integrity/storage rows inspect the
borrowed contained error while keeping its shared Arc intact. Verification cancellation
remains an opaque resource failure and does not become scheduler cancellation.

AI task progress and per-upload consent live in [`services`](../../../services/README.md).

Model operations and transfer jobs retain `engine::models::Error`; the manager borrows this
owner directly or through `engine::control::Error::Models` for the same problem categories. Installed
checks retain their model-transfer causes, while check-thread unwind remains its separate
Runtime worker failure. Presentation does not walk source chains or unwrap opaque failures.
