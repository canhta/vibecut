# Project commands

`control` is the first external consumer of the
[engine control contract](../../../../../crates/engine/README.md#control-admission-revisions-and-receipts).
One process is one engine Session: it opens the project under the
[writer lease](../../../../../crates/project/README.md#writer-lease-and-save-acknowledgement)
before reading, holds it through the final save, and releases it on exit. Requests
are newline-delimited JSON objects on stdin (`request_id`, `command`, `params`, and
optionally `idempotency_key`, `base_rev`, `api_version`, `registry_hash`,
`session_id`); omitted contract identity is filled from the running catalog, while
supplied values are still validated by engine admission. Each response line carries
the engine `Response` plus the CLI's stable `exit_code`; a frame that cannot become
a request returns a safe error envelope instead. Revisions, receipts, replay,
idempotency conflicts and receipt capacity are owned by the engine Session, so keys
and epochs live exactly as long as this process. `project.save` is the explicit save
point: `Session::control_lifecycle_save` applies the same envelope, policy, revision,
request-ID, key, capacity and receipt rules as every other effect, while this CLI
performs only project/metadata file publication in its callback. A refused or failed
save keeps the session dirty; success and typed execution errors are both receipted,
so the same request replays its error and a retry that must execute again uses a fresh
key. Otherwise a changed project and metadata are saved once on clean EOF. `--no-save`
suppresses that final save. `--allow-overwrite` is the trusted grant for `export`,
`export_subtitles` and `burn_subtitles` with `overwrite: true`; without it engine
policy returns `permission`. `preview.snapshot {time, path}` uses atomic no-clobber
without that grant; `--allow-overwrite` permits snapshot replacement. The request
cannot supply its own grant. External gesture commands remain refused by engine
policy, and agents use atomic batches plus undo. Oversized frames are drained and
returned as an in-band `limit` error, so the session continues with earlier accepted
edits intact.

The control session ticks the engine job runner between requests, so `job_status`
and `job_cancel` operate on real preview/export jobs. `preview.snapshot` renders the
requested frame at canvas size and publishes a PNG without changing project bytes;
the CLI registers the active project and metadata paths as protected, so a snapshot
destination that aliases them is refused. EOF cancels and joins
outstanding jobs through the runner's publication fence; completed outputs are
never partially promoted. There is no signal-to-job-cancel handler: process
interruption follows crash semantics, so the OS releases the lease, in-flight work
stops with the process and unsaved edits are lost. Use `job_cancel` for graceful
job cancellation.

`exec` uses [engine commands](../../../../../crates/engine/README.md#public-entry-points)
and holds the [project lease](../../../../../crates/project/README.md#writer-lease-and-save-acknowledgement)
from load through save. Scene commands use that same registry. Export uses the saved
active scene (including its derived duration) and
[render admission](../../../../../crates/render/src/export/README.md#export-and-publication) through
engine jobs, with a fixed 2 GiB aggregate budget.

`captions import` calls the [engine ASS adapter](../../../../../crates/engine/README.md#ass-caption-import),
which parses and maps styles through the captions owner and returns the
registered `captions.import` command. The CLI prints counts and line-numbered
warnings, applies the command under the same lease/save path and never
reimplements ASS parsing or style mapping.

Project lifecycle commands share engine's registry. `project new` creates through
`project.create`; `project info` acquires the project writer lease before reading
and reports optional sidecar `metadata`. `exec ... project.set_view_state` writes
only metadata, leaving project bytes unchanged; `project.rename` is an edit and
`project.save {}` explicitly republishes the project plus metadata. Missing/corrupt
sidecars default through project. Busy/conflict refusals keep their typed codes.

A shared-field edit uses the same registry, for example:
`exec project.json timeline.set_item_param_many '{"items":[10,11],"patch":{"field":"text_size","value":48}}'`.
See the [engine shared-field contract](../../../../../crates/engine/README.md#transient-selection-and-shared-fields)
for compatibility and atomic refusal rules.

Blend/opacity edits use `exec project.json timeline.set_item_param
'{"item":0,"patch":{"field":"blendMode","value":"screen"}}'`; set
`field` to `opacity` and `value` to a finite 0..=1 number for constant opacity.
The ordinary export path evaluates both through the shared renderer.

Video transition edits use `exec` or `control` with the four registered
[`transition.*` commands](../../../../../crates/engine/README.md#video-transitions).
`preview.snapshot` renders transition frames through the shared compositor;
`control_commands_render_a_transition_midpoint_frame` exercises add and PNG
publication in a real CLI subprocess. No transition-specific adapter path exists.
