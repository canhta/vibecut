# Local model manager


The header Models action opens one modal over `ai::models::catalog` and engine's
`ModelTransfers`; it opens the same `ai::models::cache_dir` bundle cache as the
CLI and never adds a second downloader, checkpoint or cache. Rows show model id,
language scope (including language-neutral weights), exact licence/source links,
total and per-file size/SHA-256, install state and the current job stage/progress.
Only MIT, Apache-2.0 and BSD weights can carry the Suggested default badge; every
other licence shows its restriction and needs the explicit review checkbox before
Download is enabled. A fresh manager hashes installed bundles on a background
check; dropping the manager cancels and joins that checker before releasing its
cache ownership. Opening, selecting a model, accepting a licence or starting transcription
never starts a transfer. The affirmative Download click is the only place a
`DownloadConsent` is granted; closing while a transfer is active is refused, and
Cancel leaves the engine checkpoint available for Resume.

`tests/it/models_ui.rs` drives the real widgets against injected cache, transport and
`ModelIo` boundaries for complete, cancel, resumed Range/ETag, checksum-failure
and disk-full paths, proves zero HTTP requests before the Download click, checks
the non-permissive opt-in, and writes 1x/2x screenshots under `target/`. The
production composition instantiates the manager lazily on first open;
`Editor::open_model` opens it on a missing model. `Editor::set_model_manager`
injects another cache/transport composition for tests and `cargo xtask ui-audit`.
