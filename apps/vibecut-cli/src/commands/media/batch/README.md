# Batch export

`batch <job-file>` is a bounded job-file adapter over the same export job owner.
It accepts a 1 MiB JSON object with `version: 1` and 1–1000 ordered entries
(64 MiB aggregate for distinct project snapshots)
(`project`, `output`, optional `aspect`/`allow_degraded`/`overwrite`), resolves
relative paths from the job file directory, loads each distinct project once
without saving it, and submits `engine::submit_export` jobs in file order through
one `JobRunner`. Preflight refuses duplicate or aliased destinations, project/source
or job-file overwrites, missing output directories, existing outputs without
per-entry overwrite, and any `overwrite`/`allow_degraded` request lacking the
command-line grant. Parsing rejects oversize bytes, duplicate keys, unknown fields,
wrong versions, invalid paths and empty/oversized job lists before project IO.
The first failed or cancelled job stops the batch; later entries are reported as
`skipped`. Completed files remain complete, and render's staging/publication fence
never promotes a partial output or replaces an existing destination on failure.
There is no signal-to-job-cancel handler; process interruption keeps the existing
crash semantics.
