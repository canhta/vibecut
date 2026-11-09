# Local model manager

The header Models action opens one modal over `ai::models::catalog` and engine's `ModelTransfers`. It uses the same `ai::models::cache_dir` bundle cache as the CLI and adds no second downloader, checkpoint or cache. Rows show model id, language scope (including language-neutral weights), licence and source links, total and per-file size/SHA-256, install state and current job stage and progress.

Only MIT, Apache-2.0 and BSD weights can carry the Suggested badge; any other licence shows its restriction and needs an explicit review checkbox before Download enables. A fresh manager hashes installed bundles in a background check; dropping the manager cancels and joins it before releasing cache ownership. Opening, selecting a model, accepting a licence or starting transcription never starts a transfer: the affirmative Download click is the only place a `DownloadConsent` is granted. Closing during a transfer is refused; Cancel leaves the engine checkpoint for Resume.

Production instantiates the manager lazily on first open. `Editor::open_model` opens it on a missing model; `Editor::set_model_manager` injects another cache/transport composition for tests and `cargo xtask ui-audit`.
