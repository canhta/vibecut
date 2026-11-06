# vibecut-cli

Headless `vibecut-cli` process adapter. Parses native command-line arguments, dispatches
edits through the engine registry, composes project writer leases and engine export
jobs, and maps typed results to stdout/stderr and stable exit codes. It owns no
timeline, codec, export-admission or provider logic and has no UI dependencies.

`lib.rs` wires the public `run`, `Output` and `CliError` entry points.
The module doors follow [structure](../../docs/engineering/structure.md#cli):

| Owner | Responsibility |
|---|---|
| `args.rs` / `args/` | Native argument interpretation with stable typed refusals |
| `output.rs` / `output/` | Process results, progress presentation, upload disclosures |
| `session.rs` / `session/` | Leased control lifetime, shared export admission, runtime settings |
| `commands.rs` / `commands/` | Native command dispatch through `run(context, args)` adapters |
| `mcp.rs` / `mcp/` | MCP surface: `grants` (trusted bounds), `inbox` (engine-side queue), `handler` (tool surface), `server` (desktop listener), `command` (the `mcp` CLI command), `socket`/`transport` (authenticated framing) |

Command siblings are grouped by project operations, media work, AI workflows,
asset libraries and generated catalogs. Each native command module exposes a single
`run(context, args)` entry point and keeps its helpers private. Commands use
only the `args`, `output`, `session` and MCP interfaces, never sibling commands.
The process context carries the existing 2 GiB memory setting; each job retains
its own budget with unchanged admission settings. Engine still owns actual
export admission and execution; `session` only composes the existing entry points.

`mcp` owns the stdio protocol adapter and OS-permissioned attach transport, composed
by the desktop app through this library. [MCP control](../../docs/product/agent-control/mcp.md)
owns the tool surface, grants, bounds, lifecycle, cancellation/retry rules and
Claude Code/Codex/OpenCode configuration. Engine remains the schema, revision,
receipt, job and rendering owner; no shared control crate is introduced.

The [CLI contract](../../docs/product/agent-control/cli.md) owns syntax, supported commands,
flags, output shapes and exit codes. `run(Vec<OsString>)` returns `Output` or
`CliError`; the binary removes the global JSON flag, writes the selected result
and maps `CliError::exit_code`. Paths retain native representation until an output
path must be represented in JSON, where unrepresentable paths return a typed
unsupported error.

`control` is the first external consumer of the
[engine control contract](../../crates/engine/README.md#control-admission-revisions-and-receipts).
One process is one engine Session: it opens the project under the
[writer lease](../../crates/project/README.md#writer-lease-and-save-acknowledgement)
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

`commands --json` returns one generated catalog object (`api_version`,
`registry_hash`, `limits`, `entries`) from the same engine declaration as dispatch;
`schema --json` returns the generated request, response and event envelope schemas
with the same identity. There is no CLI-side command table. The CLI test suite
generates both outputs twice, compares bytes, and checks the catalog digest and
generated command reference against the checked-in engine outputs.

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

`normalize <project> [--clip] [--master] [--target-lufs -14] [--true-peak-db -1]`
sets those [loudness targets](../../crates/engine/README.md#loudness-normalization)
(master by default), runs the measurement job with progress on stderr, stores the gains
and prints the measured clips and master with their gains. `export` then applies them;
it refuses stale gains after an audible edit until `normalize` runs again.

`exec` uses [engine commands](../../crates/engine/README.md#public-entry-points)
and holds the [project lease](../../crates/project/README.md#writer-lease-and-save-acknowledgement)
from load through save. Scene commands use that same registry. Export uses the saved
active scene (including its derived duration) and
[render admission](../../crates/render/README.md#export-and-publication) through
engine jobs, with a fixed 2 GiB aggregate budget. `subtitle`, `providers` and
`model` are adapters over the [engine subtitle workflow](../../crates/engine/README.md#subtitle-workflow):
the CLI parses flags, prints local/remote capabilities and upload disclosures, and
never implements chunking, consent/cost, transcription, translation, checkpoint or
serialization logic. `subtitle` has no graceful signal-to-job cancellation handler;
process interruption follows crash semantics, and completed ASR windows remain
reusable through the content-addressed cache. Local models are never downloaded
implicitly; `model download` requires `--yes-download` after printing the exact
licence, source, size and hashes. A completed export/subtitle run reports its
output paths, not render's detailed receipt.

`captions import` calls the [engine ASS adapter](../../crates/engine/README.md#ass-caption-import),
which parses and maps styles through the captions owner and returns the
registered `captions.import` command. The CLI prints counts and line-numbered
warnings, applies the command under the same lease/save path and never
reimplements ASS parsing or style mapping.

The single integration binary mirrors native command groups under `tests/it/cli/`.
The test `fixtures` door exposes project/media setup, `process` owns subprocess
invocation, and `tests/it/media_fixture.rs` owns external media oracle fixtures.
The existing unit tests follow their argument, output, session or batch interface;
assertions and behaviour names are retained.

`cargo test -p vibecut-cli` covers command dispatch, Unicode/space paths, refused
parameters, subprocess lease contention, progress, export geometry, keyframed
opacity decoded from an export, rectangle masks applied by `exec` and decoded
from an encoded export, degraded consent and source loss before/during
export, offline provider/model catalogs,
subtitle argument refusals and credential refusal before any upload, plus ASS import counts, style persistence and extension refusal. It also
covers clip argument refusals, the offline clip provider catalog and the
suggestion-credential refusal before any media work. Mid-job source loss
uses a synchronous render hook after admission and the first decoded frame,
then checks the real job result and CLI exit code while preserving the prior output.
Progress is latest-only and throttled through engine; stderr stages are observational
and cannot synchronize tests. Oracle tests require external
FFmpeg/ffprobe: missing tools print a skip, required-oracle mode refuses a skip,
and found but failing tools fail. Run `cargo xtask ci --require-oracles` for the
full workspace check and tool/fixture receipts. No speed or memory measurement
is claimed; Windows runtime behavior remains unverified.

`sounds` is an adapter over the [engine sound library](../../crates/engine/README.md#sound-library).
`sounds search --library <dir> [--page N] [--page-size N] [--all-licences] [query]`
uses `FREESOUND_API_KEY` when present and otherwise searches only saved local
sounds; results carry author, licence, attribution and URLs. `sounds save <library>
<sound-json>` and `sounds unsave <library> <id>` manage the durable local library.
`sounds add <project> <library> <id> [--start <ticks>]` requires the exact preview
URL to be cached locally, then commits the asset and audio item as one undo step
through `sounds.add_to_timeline`, copying the sound into `<project>.media/` so later
library cleanup cannot remove it; it never downloads implicitly. Missing cache,
missing credential, auth, quota, rate-limit, network and no-audio-device failures
keep distinct stable codes, and no command prints or persists the token.

`clips` is an adapter over the [engine clip suggestion workflow](../../crates/engine/README.md#clip-suggestion-workflow):
it parses suggestion, ASR, translation and export flags, prints the provider's
upload scope before any remote byte leaves the machine, and maps candidates or
typed failures to stdout/stderr. It owns no segmentation, prompt, selection,
snapping or export logic; `--provider` never falls back. Remote suggestion
credentials are refused before media work, and `providers --operation clips`
lists the same offline catalog without opening an adapter. Boundary adjustment
kinds, including sub-frame `word_fade`, follow the [CLI contract](../../docs/product/agent-control/cli.md).

Project lifecycle commands share engine's registry. `project new` creates through
`project.create`; `project info` acquires the project writer lease before reading
and reports optional sidecar `metadata`. `exec ... project.set_view_state` writes
only metadata, leaving project bytes unchanged; `project.rename` is an edit and
`project.save {}` explicitly republishes the project plus metadata. Missing/corrupt
sidecars default through project. Busy/conflict refusals keep their typed codes.

A shared-field edit uses the same registry, for example:
`exec project.json timeline.set_item_param_many '{"items":[10,11],"patch":{"field":"text_size","value":48}}'`.
See the [engine shared-field contract](../../crates/engine/README.md#transient-selection-and-shared-fields)
for compatibility and atomic refusal rules.

Blend/opacity edits use `exec project.json timeline.set_item_param
'{"item":0,"patch":{"field":"blendMode","value":"screen"}}'`; set
`field` to `opacity` and `value` to a finite 0..=1 number for constant opacity.
The ordinary export path evaluates both through the shared renderer.

Video transition edits use `exec` or `control` with the four registered
[`transition.*` commands](../../crates/engine/README.md#video-transitions).
`preview.snapshot` renders transition frames through the shared compositor;
`control_commands_render_a_transition_midpoint_frame` exercises add and PNG
publication in a real CLI subprocess. No transition-specific adapter path exists.

`scenes` composes the engine's shared scene detector and ordinary jobs/cache,
returning candidates without a project edit. Syntax and output are owned by the
[CLI contract](../../docs/product/agent-control/cli.md#scene-detection); thresholds, limits and
reviewed marker application belong to [engine](../../crates/engine/README.md#scene-detection).
