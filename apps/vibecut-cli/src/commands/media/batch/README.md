# Batch export

`batch <job-file>` is a bounded job-file adapter over the export job owner. The file is a JSON object of at most 1 MiB with `version: 1` and 1–1000 ordered entries (64 MiB aggregate for distinct project snapshots). Each entry has `project`, `output` and optional `aspect`, `allow_degraded`, `overwrite`. Relative paths resolve from the job file directory. Each distinct project loads once without saving, and `engine::submit_export` jobs run in file order through one `JobRunner`.

Parsing rejects oversize bytes, duplicate keys, unknown fields, wrong versions, invalid paths and empty or oversized job lists before project IO. Preflight refuses duplicate or aliased destinations, project/source or job-file overwrites, missing output directories, existing outputs without per-entry overwrite, and any `overwrite` or `allow_degraded` request lacking the command-line grant.

The first failed or cancelled job stops the batch and later entries are reported as `skipped`. Completed files stay complete, and render's staging/publication fence never promotes a partial output or replaces an existing destination on failure. There is no signal-to-job-cancel handler; interruption follows crash semantics.
