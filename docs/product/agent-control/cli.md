# CLI

`vibecut-cli` opens projects, runs editor commands, probes media and exports MP4; adapter boundaries follow [architecture](../../engineering/architecture.md#contracts).

Build with `cargo build -p vibecut-cli` through the [build wrapper](../../agents/orchestration.md#placement-and-builds); output is `target/<profile>/vibecut-cli`.

## Commands

| Command | Does |
|---|---|
| `vibecut-cli scenes <file>` | Detect reviewed-marker candidates without editing a project |
| `vibecut-cli probe <file>` | Probe a media file and report format, duration and streams |
| `vibecut-cli project new <path>` | Create an empty project, refusing to replace an existing file |
| `vibecut-cli project info <path>` | Summary plus optional view metadata under the project writer lease |
| `vibecut-cli exec <project> <command_id> <json>` | Run one engine command and save the result |
| `vibecut-cli control <project>` | Run a revision-bound control session over newline-delimited JSON |
| `vibecut-cli mcp <project>` / `mcp --attach --socket <endpoint>` | Serve [MCP control](mcp.md) over stdio, headless or attached to the app |
| `vibecut-cli export <project> --out <mp4>` | Export the saved active scene to MP4 |
| `vibecut-cli batch <job-file>` | Render the validated export jobs in a JSON job file |
| `vibecut-cli subtitle <in> --to <lang>` | Transcribe, translate and export SRT/burned subtitles |
| `vibecut-cli captions import <project> <file.ass>` | Import ASS cues and styles as one caption item and print counts |
| `vibecut-cli clips <in>` | Suggest ranked shorts from a long video and optionally export them |
| `vibecut-cli providers` | List transcription/translation/clip providers and capabilities offline |
| `vibecut-cli model list\|info\|download` | Inspect or install pinned local model files |
| `vibecut-cli sounds search\|save\|unsave\|add` | Search Freesound with a user token or local saved sounds; save, unsave and add a cached sound |
| `vibecut-cli fonts list\|resolve\|download` | Search bundled, system and catalog fonts, resolve a family, or download a catalog font |
| `vibecut-cli commands` | Print the generated command registry |
| `vibecut-cli schema` | Print the generated control request/response/event schema |

Add `--json` to any command for a machine-readable result on stdout. Without it, results are human-readable on stdout. Progress and diagnostics are always on stderr, so stdout stays a single result. Errors use exit codes below and, with `--json`, one JSON object on the last stderr line.

`export` options: `--aspect 9:16` renders 1080×1920 and `--aspect 16:9` renders 1920×1080; without it the project canvas is used. `--overwrite` replaces an existing output. `--allow-degraded` is the explicit consent described below.

Timeline group edits use `exec <project> timeline.move_items
'{"items":[1,2],"delta":100}'` and `exec <project> timeline.trim
'{"items":[1,2],"edge":"end","delta":-100}'` (PowerShell single-quoted JSON).
Times are integer output ticks. Optional move `placement` uses the same tagged
track/above/below placement as insertion; omitted placement retains each track.
Each invocation saves one atomic edit. Interactive preview stays in an app-owned
Session through the [engine preview contract](../../../crates/engine/README.md#timeline-placement).

## Fonts

`vibecut-cli fonts list [--query <text>]` lists families whose name contains the text, prefix matches first and then alphabetical: bundled families, OS-installed families and the curated downloadable catalog, one entry per name (bundled over a cached download over a system install over a download not cached yet). Each entry carries `origin` (`bundled`, `system`, `google`), `state` (`available` or `not_downloaded`), `styles`, and for bundled and catalog families the `license` id and `source` URL. `fonts resolve <family>` loads a family the way a render would and prints `state` (`loaded`, `not_downloaded`, `cache_invalid` or `unknown`) and the `effective_family` layout will use; any state other than `loaded` exits 4 and layout falls back to Inter. Neither command uses the network.

`fonts download <family> --accept-license` is the only network action. It prints the family, licence id, source URL and byte count to stderr and refuses with exit 5 unless the flag acknowledges them. Files are fetched over HTTPS, verified against their pinned size and SHA-256, stored with a licence file and a per-file attribution record under the per-user cache `<cache>/VibeCut/fonts`, and published as one directory; a failed or interrupted download leaves nothing behind. A cached family whose files no longer match their pins reports `cache_invalid` until it is downloaded again. The same operations are the `fonts.list`, `fonts.load` and `fonts.download` entries of the command registry in a `control` session.

## Captions

`vibecut-cli captions import <project> <file.ass|file.ssa> [--language <tag>] [--start <ticks>] [--track <id>] [--name <name>]` parses the script through `captions::parse_ass`, prints cue/style/skipped counts and line-numbered warnings, and applies the registered `captions.import` command. The asset, caption track and caption item are one undo step; `--track` targets an existing text track and `--name` labels a newly allocated one (`Captions` by default). `--language` is the cue source language (default `en`) and `--start` offsets the caption item in output ticks. Input is capped at 16 MiB; malformed lines warn and skip, while an oversized file or a document with no surviving cue fails with a typed error and leaves the project unchanged.

## Subtitles

`vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual] [--srt out.srt] [--burn out.mp4] [--aspect 9:16] [--provider <id>] [--model <id>] [--translate-provider openai|anthropic] [--translate-model <id>] [--yes-upload] [--overwrite]` runs the engine subtitle workflow: speech extraction, selected ASR, checkpointed chunks, local alignment when the provider has no native timing, cue segmentation, optional contextual translation, and the existing SRT/burn export paths. The input media is probed once; `--aspect 9:16` selects 1080×1920 for burn and `16:9` selects 1920×1080; without it the source geometry is used. At least one of `--srt` and `--burn` is required.

Providers and models are explicit and are never switched automatically. `auto` language detection requires the local `whisper.cpp` provider and resolves the source before deciding whether a translator is needed; a same-language local run therefore needs no cloud translation credentials. A different-language run still requires the explicitly selected translation provider and its key. `vibecut-cli providers --operation transcribe|translate --language <tag>` lists offline capability data; `--model` must be one of the listed models. Local defaults are `whisper-large-v3-turbo` (vi/en/zh) and `gipformer-68m-rnnt-int8` (vi); cloud choices are OpenAI `gpt-transcribe` and ElevenLabs `scribe_v2`. Translation choices are OpenAI `gpt-6.1-sol` and Anthropic `claude-sonnet-5-5`; `--to zh` selects Simplified (`zh-Hans`), and `zh-Hant` is available explicitly.

Remote requests are refused before any upload unless `--yes-upload` is present. The command prints the exact provider/model, languages, audio duration, upload payload bytes, per-invocation cost ceiling and the provider's official policy links for every request scope; the flag authorizes only those displayed scopes, never a model download or a different provider. Local providers need no upload consent.

ASR checkpoints are content-addressed complete windows in the default model cache. A restarted command reuses valid windows and executes only missing ones; the outcome reports the reused count. Checksum failure or an incomplete window is recomputed; an alignment failure keeps the completed recognition checkpoint and alignment is retried next run. The engine workflow also exposes an operation-wide cooperative cancellation token connected to extraction, active jobs, translation and export; the CLI has no signal-driven cancel gesture, so process interruption remains crash semantics. Local weights are separate from checkpoints and are never downloaded implicitly: `vibecut-cli model download <id> --yes-download` prints licence, source, size, every file SHA-256 and requires the explicit flag. A missing model returns an actionable `setup` error naming the download command.

`VIBECUT_ASR_THREADS` selects 1–32 local threads (default 4). `VIBECUT_ASR_GPU=0` forces the CPU path. `VIBECUT_LOCAL_ASR` names the shipped Gipformer runner; without it the Gipformer provider returns a typed setup error. Credentials come from `OPENAI_API_KEY`, `ELEVENLABS_API_KEY` and `ANTHROPIC_API_KEY`; keys are never printed. Credential absence is a `setup` refusal before media or network work.

## Commands and schemas

`vibecut-cli commands --json` is the authoritative generated catalog: one object with `api_version`, `registry_hash`, the static control `limits` and `entries`, where every entry carries its command ID, label, effect, JSON Schema 2020-12 input/output shapes and fixture example. `vibecut-cli schema --json` returns the same `api_version`/`registry_hash` with the generated request, response and event envelope schemas. Registry ownership follows [architecture](../../engineering/architecture.md#contracts); there is no CLI-side command table. `exec` takes that command ID and its `params` object:

```sh
vibecut-cli project new demo.vibecut
vibecut-cli exec demo.vibecut timeline.add_track '{"name":"Video","kind":"video"}'
vibecut-cli exec demo.vibecut batch '{"commands":[
  {"command":"timeline.add_track","params":{"name":"Overlay","kind":"video"}},
  {"command":"add_bookmark","params":{"start":0,"duration":1,"note":"intro","color":[0,0,0,255]}}
]}'
vibecut-cli project info demo.vibecut --json
```

`project.set_view_state` takes `{"zoom":2,"scroll":40,"playhead":0}` (logical-pixel scroll, integer-tick playhead), persists optional metadata and leaves editable project bytes unchanged. `project.rename` takes `{"name":"My project"}`; `project.save` takes `{}`. Missing/corrupt metadata defaults on open. `project new` uses the registered creation command; `exec` refuses creation inside an existing session. Project info and edits refuse a busy writer instead of reading stale state or overwriting another session.

`timeline.set_speed` takes `{"items":[1,2],"speed":2.0,"maintain_pitch":true}`: it retimes the listed video/audio items together (pass a linked video and audio item in one call to keep them in sync), keeping each source window, and `export` renders the result at that speed with pitch preserved or following speed.

Keyframed item parameters are ordinary registry commands: `keyframes.set_param`, `keyframes.add`, `keyframes.remove`, `keyframes.retime`, `keyframes.set_handle` and `keyframes.scale_time` accept the project animation shapes. `export` evaluates the saved curves through render's single project evaluator; for example an `opacity` channel animates a clip in the exported frames.

`audio.set_fade {item, edge, duration, curve}` (`edge` `in`|`out`, `curve` `equal_power`|`linear`|`exponential`, duration in ticks) fades an audio or video clip edge; the same envelope is applied to the exported audio and to preview.

Fields are required; unknown command, parameter and model fields are refused. Model wire types are the Serde shapes owned by their crates; the [engine contract](../../../crates/engine/README.md#public-entry-points) describes the registry and JSON limits.

## Control session

`vibecut-cli control <project> [--allow-overwrite] [--no-save]` is the first CLI consumer of the [engine control contract](../../../crates/engine/README.md#control-admission-revisions-and-receipts). One process is one engine Session: the writer lease is acquired before the first frame and held until the final save, so a desktop app holding it returns `busy`/3 before any request is read. Sessions have an OS-random epoch; separate `control` invocations are separate sessions and do not share revisions or receipts.

Requests are newline-delimited JSON objects on stdin: `request_id`, `command`, `params`, and optionally `idempotency_key`, `base_rev`, `api_version`, `registry_hash` and `session_id`. Omitted contract identity is filled from the running catalog; supplied values are still admitted by the engine, so a wrong version or digest returns `schema_mismatch` and a wrong session returns `session_gone`. Read commands need no revision or key. Effectful commands need the current `base_rev` and a 1–128 byte `idempotency_key`; stale revisions return `stale_revision` before any child of a batch, and replaying an accepted key returns its original result while a different request under the same key returns `idempotency_conflict`. Receipts are not evicted: the next new effect after 4096 retained keys returns `receipt_capacity`. Unknown commands, unknown/extra fields, duplicate JSON keys, frames over 8 MiB and integers outside ±(2^53−1) are refused before effects; an oversized frame is drained and returned as an in-band `limit` error, so the session continues with earlier accepted edits intact. A session retains 100 undo steps, so `undo` and `redo` are ordinary frames; lifecycle `project.create` stays with `project new` and is refused inside a session.

stdout is always newline-delimited JSON; `--json` is implied and no human formatting is applied. Each response line is the engine `Response` plus a stable `exit_code` field. A frame that cannot become a request returns `{"status":"error","exit_code":N,"error":{"code":"…","message":"…","exit_code":N}}` instead. The process exits with the last frame's mapped code (0 for success), so a one-frame session can be used like `exec`; earlier refusals stay in-band so the session can retry. Mapping: usage/schema errors 2, stale revision 3, permission/idempotency/receipt/transaction/refusal 5, and session/job/execution failures 7. Engine replay returns the original effect revision even after a later save; use `inspect` for the current revision.

`project.save` is the explicit save point. Its admission, request-ID bounds, idempotency conflict/replay and receipt capacity are owned by `Session::control_lifecycle_save`; the CLI callback performs only project/metadata file publication. A refused or failed save keeps the session dirty; success and typed execution errors are both receipted, so the same request replays its error while a retry that must execute again uses a fresh key. Otherwise a changed project and metadata are saved once on clean EOF; `--no-save` suppresses that final save while leaving explicit `project.save` available. Saves use the project owner's atomic publication. Explicit save failures are reported in-band so the session can continue; a final EOF save failure exits non-zero. `--allow-overwrite` is the trusted policy grant for `export`, `export_subtitles` and `burn_subtitles` with `overwrite: true`; without it the engine refuses with `permission`. External gesture commands stay refused; agents use atomic `batch` plus `undo`.

The session ticks the engine job runner between requests. `preview` and `export` return job IDs; poll `job_status`, including retained inline preview JPEGs, and use `job_cancel` for graceful cancellation. `preview.snapshot {time, path}` is synchronous: it renders the requested frame at canvas size through the production compositor and publishes a PNG atomically without changing project bytes; the CLI registers the active project and metadata paths as protected, so an aliasing destination is refused. EOF cancels and joins outstanding jobs through the runner's publication fence. There is no signal-to-job-cancel handler: process interruption follows crash semantics, so the OS releases the lease, running work stops with the process and unsaved edits are lost; `job_cancel` is the graceful path.

## Batch job files

`vibecut-cli batch <job-file> [--allow-degraded] [--allow-overwrite]` renders an
ordered set of saved projects through the same `export` engine/render job path.
The command never edits or saves a project: each distinct project is loaded once
as an immutable snapshot before any job is submitted. It performs no network,
provider, cache-download or subtitle work, so the command is offline.

The file is a JSON object with exactly `version` (`1`) and `jobs` (1–1000 entries).
Distinct project files held as immutable snapshots are capped at 64 MiB aggregate.
Each entry has exactly `project` and `output`, plus optional `aspect` (`project`,
`9:16` or `16:9`; default `project`), `allow_degraded` and `overwrite` (default
`false`). Relative paths resolve from the job file's directory. Paths must be
UTF-8, non-empty, at most 4096 bytes and name a file; `output` parents must already
exist. All outputs must be distinct and must not alias a project, project source or
the job file. Hard-linked outputs are refused; on Windows and macOS case-only
destination collisions are conservatively refused even in case-sensitive
directories. macOS protection also equates composed/decomposed Unicode spellings.
Existing outputs refuse unless that entry sets `overwrite` and the
command has `--allow-overwrite`. An entry setting `allow_degraded` requires
`--allow-degraded`; neither grant can be inferred from the file alone.

```json
{
  "version": 1,
  "jobs": [
    {"project": "a.vibecut", "output": "out/a.mp4"},
    {"project": "b.vibecut", "output": "out/b-vertical.mp4", "aspect": "9:16"}
  ]
}
```

The file is bounded to 1 MiB before parsing; malformed/truncated JSON, duplicate
keys, unknown fields, an unsupported version, wrong field types, empty or oversized
`jobs`, invalid paths and unsafe/ambiguous destinations are typed refusals before
any job or output. A non-regular job file is refused; a missing or unreadable file
is an I/O failure.

Jobs run in file order with one engine export worker. Results are an object with
`status` (`completed` or `failed`) and a `jobs` array in file order; each entry has
its zero-based `index`, `project`, `output`, `status` (`completed`, `failed` or
`skipped`) and, on failure, a typed `error` object. The first failed or cancelled
job stops the batch: later entries are `skipped`, already completed files remain
complete, and the process exits with that first failure's code. Render's staging and
publication fence means a failed/cancelled job never promotes a partial file and
preserves an existing destination. Process interruption follows the existing crash
semantics: the OS ends the process, staging files are not final outputs, and no
partial MP4 is promoted. There is no signal-to-job-cancel handler; use the engine
`job_cancel` path for graceful cancellation.

## Project files and concurrency

A project is the `.vibecut` file owned by the project crate. Its schema, atomic publication, recovery snapshots and writer lease are documented in the [project contract](../../../crates/project/README.md#persistence-and-limits); the CLI does not define a second format page.

Lease admission for reads and writes follows the [CLI owner contract](../../../apps/vibecut-cli/README.md).

## Export admission

The output extension selects MP4/H.264, MOV/H.264 or WebM/VP9 through the shared
render owner. `--settings <JSON>` accepts the versioned `render::OutputOptions`
contract: `version: 1`, nullable `resolution: [width,height]`, `fps: {num,den}`,
`bit_rate`, `codec: automatic|h264|vp9`, `profile: automatic|main|high`,
`range: {kind: full}` or `{kind: in_out|selected, start, end}` in ticks, and
`scene: {kind: active|all}` or `{kind: selected, scene: <id>}`. All fields are
required; older versions and unknown fields refuse. Defaults and bounds belong
to the [render contract](../../../crates/render/README.md).
`--aspect` remains an output geometry override; explicit settings resolution wins.
All scenes use distinct `<stem>-scene-<id>.<ext>` destinations. JSON success adds
`paths` containing every completed file; `path` remains the requested output stem.

`export` uses render's finished-file path: output is staged and published only after encoding, muxing and a final source revalidation. A required source that is missing or whose content hash changed refuses the export with exit code 4 and `unavailable` entries naming the asset ID, item ID, source tick range and reason (`missing`, `changed`, `metadata`, `video` or `audio`). No partial final file is promoted and an existing destination is preserved.

Degraded output is never a silent default. `--allow-degraded` is the explicit consent that admits initially missing or changed visual sources as black/silence; a source that fails during execution still refuses. Details of the admission and publication fence are in the [render contract](../../../crates/render/src/export/README.md#export-and-publication).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage, malformed arguments or JSON, unknown command or invalid parameters |
| 3 | Project busy or changed on disk; nothing written |
| 4 | Required media, model, runner or credential unavailable; nothing published |
| 5 | Project or command rejected by validation, or upload consent refused |
| 6 | Media or render execution failed |
| 7 | I/O or background job failure |

With `--json`, an error is `{"status":"error","error":{"code":"busy","message":"…", "exit_code":3,"unavailable":[…]}}`; `unavailable` appears only for source admission failures. Messages never contain project text, media text, provider payloads or API keys. Subtitle errors add the stable codes `setup` (missing local model/runner/credential), `ai` (provider execution) and `rejected` (refused upload consent or unsupported language choice). A `control` session returns the same codes per response line and exits with the last frame's code; its error envelope and mapping are described in [Control session](#control-session).

`batch` writes its multi-job result to stdout (JSON with `--json`); admission
refusals of the job file itself use the same stderr error envelope as other
commands. Its process status is the first failed job's code from this table, or 0
when every job completed.

## Sounds

`sounds search --library <dir> [--page N] [--page-size N] [--all-licences] [query]`
calls Freesound APIv2 directly when `FREESOUND_API_KEY` is set and otherwise
searches saved local sounds. Results are commercial-only by default and sorted by
downloads; each carries author, licence, attribution, source page and preview URL.
The token is never printed, stored or appended to a URL. Provider failures use the
stable codes `auth`, `quota`, `rate_limit`, `network` and `setup`; a missing
credential degrades to local search rather than failing.

`sounds save <library> <sound-json>` and `sounds unsave <library> <id>` update the
durable local library; `sounds add <project> <library> <id> [--start <ticks>]`
requires the preview to be cached and commits one undoable audio item with
`sourceType=library`, attribution and licence metadata. `clear_all` is exposed by
the engine library owner for the panel. Cached previews are keyed by the exact
provider URL, so changing it invalidates the old bytes.

## Clip suggestions

`vibecut-cli clips <in> [--from vi|en|zh|auto] [--count N] [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s] [--overlap R] [--keywords a,b] [--provider heuristic|openai|anthropic] [--model <id>] [--asr-provider <id>] [--asr-model <id>] [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual] [--aspect 9:16] [--translate-provider openai|anthropic] [--overwrite] [--allow-degraded]] [--yes-upload]` transcribes the input through the engine subtitle workflow, ranks ordered-segment shorts, snaps every source edge and prints the candidates as JSON. `--provider` selects the suggestion source explicitly; `heuristic` is the local default and needs no upload. Remote suggestion uploads only transcript segments, never audio, and require `--yes-upload` after the command prints the provider, endpoint, exact byte count, cost ceiling and the provider's data-retention/training policy link. A failed provider is a typed failure; the command never falls back to another provider or to the heuristic.

`--select` takes 1-based candidate indices; without it every candidate is exported. `--export` requires `--subs` and prepares one engine batch export per selected short at 1080×1920, burning the requested language. When `--subs` differs from the detected source, or `--bilingual` is set, the same subtitle translation path runs before export. Outputs are `<title>.mp4`; selected sidecars are none by default. Existing files are preserved unless `--overwrite` is set, and `--allow-degraded` is the explicit consent for initially missing/changed visual sources.

Candidate output preserves the AI ordered-segment contract: `pieces` are the provider's play-order pieces with roles, `segments` are the snapped source ranges, and `adjustments` carries the signed original/adjusted delta per edge. The `word_fade` kind identifies frame rounding at a sub-frame word gap; [shorts snapping](../../../crates/shorts/src/boundaries/README.md) owns its overlap and export-audio policy. Rejected snap candidates and dropped suggestions are reported with typed reasons. `vibecut-cli providers --operation clips` lists the offline suggestion catalog, including each remote provider's policy link.


## Scene detection

`vibecut-cli scenes <file> [--threshold 0.28] [--fade-threshold 0.20]
[--min-scene-ticks <ticks>] [--json]` imports a local known-duration video and
runs the engine's [scene detection job](../../../crates/engine/README.md#scene-detection).
Human output prints source ticks, change kind and confidence; JSON includes the
content key, source identity, requested range, ordered `candidates` and
`decoded_frames`. Confidence is measured change, not a calibrated probability.
Explicit settings enter the content cache key. The command never edits a project.
There is no signal-driven graceful cancellation; process interruption follows the
CLI's crash semantics. Engine callers use the returned job cancellation token.
