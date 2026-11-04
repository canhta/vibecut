# CLI

`vibecut-cli` is the headless adapter over the engine command registry. It opens
projects, runs editor commands, probes media and exports MP4 without a window,
UI toolkit or audio device. The same command IDs and parameters serve the
desktop app, MCP and plugins; see [control](../engineering/architecture.md#contracts).

Build it with `cargo build -p vibecut-cli`; the binary is `target/<profile>/vibecut-cli`.

## Commands

| Command | Does |
|---|---|
| `vibecut-cli probe <file>` | Probe a media file and report format, duration and streams |
| `vibecut-cli project new <path>` | Create an empty project, refusing to replace an existing file |
| `vibecut-cli project info <path>` | Read-only summary of a saved project |
| `vibecut-cli exec <project> <command_id> <json>` | Run one engine command and save the result |
| `vibecut-cli export <project> --out <mp4>` | Export the saved timeline to MP4 |
| `vibecut-cli subtitle <in> --to <lang>` | Transcribe, translate and export SRT/burned subtitles |
| `vibecut-cli providers` | List transcription/translation providers and capabilities offline |
| `vibecut-cli model list\|info\|download` | Inspect or install pinned local model files |
| `vibecut-cli commands` | Print the generated command registry |

Add `--json` to any command for a machine-readable result on stdout. Without it,
results are human-readable on stdout. Progress and diagnostics are always on
stderr, so stdout stays a single result. Errors use exit codes below and, with
`--json`, one JSON object on the last stderr line.

`export` options: `--aspect 9:16` renders 1080×1920 and `--aspect 16:9` renders
1920×1080; without it the project canvas is used. `--overwrite` replaces an
existing output. `--allow-degraded` is the explicit consent described below.

## Subtitles

`vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]
[--srt out.srt] [--burn out.mp4] [--aspect 9:16] [--provider <id>] [--model <id>]
[--translate-provider openai|anthropic] [--translate-model <id>] [--yes-upload]
[--overwrite]` runs the engine subtitle workflow: speech extraction, selected ASR,
checkpointed chunks, local alignment when the provider has no native timing, cue
segmentation, optional contextual translation, and the existing SRT/burn export
paths. The input media is probed once; `--aspect 9:16` selects 1080×1920 for burn
and `16:9` selects 1920×1080; without it the source geometry is used. At least one
of `--srt` and `--burn` is required.

Providers and models are explicit and are never switched automatically. `auto`
language detection requires the local `whisper.cpp` provider and resolves the
source before deciding whether a translator is needed; a same-language local run
therefore needs no cloud translation credentials. A different-language run still
requires the explicitly selected translation provider and its key. `vibecut-cli providers
--operation transcribe|translate --language <tag>` lists offline capability data;
`--model` must be one of the listed models. Local defaults are
`whisper-large-v3-turbo` (vi/en/zh) and `gipformer-68m-rnnt-int8` (vi); cloud
choices are OpenAI `gpt-transcribe` and ElevenLabs `scribe_v2`. Translation choices
are OpenAI `gpt-6.1-sol` and Anthropic `claude-sonnet-5-5`; `--to zh` selects
Simplified (`zh-Hans`), and `zh-Hant` is available explicitly.

Remote requests are refused before any upload unless `--yes-upload` is present.
The command prints the exact provider/model, languages, audio duration, upload
payload bytes, per-invocation cost ceiling and the provider's official
policy links for every request scope; the flag
authorizes only those displayed scopes, never a model download or a different
provider. Local providers need no upload consent.

ASR checkpoints are content-addressed complete windows in the default model cache.
A restarted command reuses valid windows and executes only missing ones; the
outcome reports the reused count. Checksum failure or an incomplete window is
recomputed; an alignment failure keeps the completed recognition checkpoint and
alignment is retried next run. The engine workflow also exposes an operation-wide
cooperative cancellation token connected to extraction, active jobs, translation
and export; the CLI has no signal-driven cancel gesture, so process interruption
remains crash semantics. Local weights are separate from checkpoints and are never downloaded
implicitly: `vibecut-cli model download <id> --yes-download` prints licence, source,
size, every file SHA-256 and requires the explicit flag. A missing model returns
an actionable `setup` error naming the download command.

`VIBECUT_ASR_THREADS` selects 1–32 local threads (default 4). `VIBECUT_ASR_GPU=0`
forces the CPU path. `VIBECUT_LOCAL_ASR` names the shipped Gipformer runner; without
it the Gipformer provider returns a typed setup error. Credentials come from
`OPENAI_API_KEY`, `ELEVENLABS_API_KEY` and `ANTHROPIC_API_KEY`; keys are never
printed. Credential absence is a `setup` refusal before media or network work.

## Commands and schemas

`vibecut-cli commands --json` is the authoritative generated schema: every command
ID, label and required parameter field with its wire type. It comes from the same
single declaration as typed and JSON dispatch, so do not maintain a second command
list. `exec` takes that command ID and its `params` object:

```sh
vibecut-cli project new demo.vibecut
vibecut-cli exec demo.vibecut add_track '{"name":"Video","kind":"video"}'
vibecut-cli exec demo.vibecut batch '{"commands":[
  {"command":"add_track","params":{"name":"Overlay","kind":"overlay"}},
  {"command":"add_bookmark","params":{"start":0,"duration":1,"note":"intro","color":[0,0,0,255]}}
]}'
vibecut-cli project info demo.vibecut --json
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
| 4 | Required media, model, runner or credential unavailable; nothing published |
| 5 | Project or command rejected by validation, or upload consent refused |
| 6 | Media or render execution failed |
| 7 | I/O or background job failure |

With `--json`, an error is `{"status":"error","error":{"code":"busy","message":"…",
"exit_code":3,"unavailable":[…]}}`; `unavailable` appears only for source
admission failures. Messages never contain project text, media text, provider
payloads or API keys. Subtitle errors add the stable codes `setup` (missing local
model/runner/credential), `ai` (provider execution) and `rejected` (refused upload
consent or unsupported language choice).

## Clip suggestions

The `suggest_clips` progress stage is recognized. Provider execution and clip-result
rendering are not yet CLI commands. The serialized engine result uses the
[AI ordered-segment contract](../../crates/ai/README.md#transcript-clip-suggestions);
future presentation must preserve its play order and source-order ranks.
