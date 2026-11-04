# CLI

`vibecut` is the headless adapter over the engine command registry. It opens
projects, runs editor commands, probes media and exports MP4 without a window,
UI toolkit or audio device. The same command IDs and parameters serve the
desktop app, MCP and plugins; see [control](../engineering/architecture.md#contracts).

Build it with `cargo build -p vibecut-cli`; the binary is `target/<profile>/vibecut`.

## Commands

| Command | Does |
|---|---|
| `vibecut probe <file>` | Probe a media file and report format, duration and streams |
| `vibecut project new <path>` | Create an empty project, refusing to replace an existing file |
| `vibecut project info <path>` | Read-only summary of a saved project |
| `vibecut exec <project> <command_id> <json>` | Run one engine command and save the result |
| `vibecut export <project> --out <mp4>` | Export the saved timeline to MP4 |
| `vibecut commands` | Print the generated command registry |

Add `--json` to any command for a machine-readable result on stdout. Without it,
results are human-readable on stdout. Progress and diagnostics are always on
stderr, so stdout stays a single result. Errors use exit codes below and, with
`--json`, one JSON object on the last stderr line.

`export` options: `--aspect 9:16` renders 1080×1920 and `--aspect 16:9` renders
1920×1080; without it the project canvas is used. `--overwrite` replaces an
existing output. `--allow-degraded` is the explicit consent described below.

## Commands and schemas

`vibecut commands --json` is the authoritative generated schema: every command
ID, label and required parameter field with its wire type. It comes from the same
single declaration as typed and JSON dispatch, so do not maintain a second command
list. `exec` takes that command ID and its `params` object:

```sh
vibecut project new demo.vibecut
vibecut exec demo.vibecut add_track '{"name":"Video","kind":"video"}'
vibecut exec demo.vibecut batch '{"commands":[
  {"command":"add_track","params":{"name":"Overlay","kind":"overlay"}},
  {"command":"add_bookmark","params":{"start":0,"duration":1,"note":"intro","color":[0,0,0,255]}}
]}'
vibecut project info demo.vibecut --json
```

Fields are required; unknown command, parameter and model fields are refused.
Model wire types are the Serde shapes owned by their crates; the
[engine contract](../../crates/engine/README.md#public-entry-points) describes the
registry and JSON limits. The CLI adds no project logic of its own.

## Project files and concurrency

A project is the `.vibecut` file owned by the project crate. Its schema, atomic
publication, recovery snapshots and writer lease are documented in the
[project contract](../../crates/project/README.md#persistence-and-limits); the CLI
does not define a second format page.

`project new` and every `exec` read-edit-save acquire the project writer lease
before reading and hold it through the save. A concurrent desktop or CLI writer
therefore gets a typed `busy` or `conflict` refusal, exit code 3, and no bytes are
written. `project info` and `export` are read-only and do not take the lease; they
observe the last atomically published project file.

## Export admission

`export` uses render's finished-file path: output is staged and published only
after encoding, muxing and a final source revalidation. A required source that is
missing or whose content hash changed refuses the export with exit code 4 and
`unavailable` entries naming the asset ID, item ID, source tick range and reason
(`missing`, `changed`, `metadata`, `video` or `audio`). No partial final file is
promoted and an existing destination is preserved.

Degraded output is never a silent default. `--allow-degraded` is the explicit
consent that admits initially missing or changed visual sources as black/silence;
a source that fails during execution still refuses. Details of the admission and
publication fence are in the
[render contract](../../crates/render/README.md#export-and-publication).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Usage, malformed arguments or JSON, unknown command or invalid parameters |
| 3 | Project busy or changed on disk; nothing written |
| 4 | Required media source unavailable or changed; nothing published |
| 5 | Project or command rejected by validation |
| 6 | Media or render execution failed |
| 7 | I/O or background job failure |

With `--json`, an error is `{"status":"error","error":{"code":"busy","message":"…",
"exit_code":3,"unavailable":[…]}}`; `unavailable` appears only for source
admission failures. Messages never contain project text, media text or provider
payloads.

## Clip suggestions

The `suggest_clips` progress stage is recognized. Provider execution and clip-result
rendering are not yet CLI commands. The serialized engine result uses the
[AI ordered-segment contract](../../crates/ai/README.md#transcript-clip-suggestions);
future presentation must preserve its play order and source-order ranks.
