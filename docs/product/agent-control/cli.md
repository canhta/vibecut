# CLI

`vibecut-cli` opens projects, runs editor commands, probes media and exports video; adapter boundaries follow [architecture](../../engineering/architecture.md#contracts).

Build with `cargo build -p vibecut-cli` through the [build wrapper](../../../xtask/README.md#build-slots); output is `target/<profile>/vibecut-cli`.

## Commands

`vibecut-cli color lut <project> --import <file.cube|file.3dl> --item <id> [--strength <0..100>]` waits for the real Human import job, then registers/applies its durable descriptor as one atomic engine batch under the existing project writer lease. Failure leaves project bytes unchanged; a complete orphan resource may remain within the store quota. Existing `effects.update_param` accepts a number or `{"lut":"<canonical hash>"}` / `{"lut":null}` for already registered resources. Importing source paths and registering descriptors remain Human grants on MCP; an agent may apply existing references without gaining source-file access. Formats, limits and retained provenance belong to [render](../../../crates/render/src/scene/effects/README.md#imported-cubes) and [store](../../../crates/engine/src/store/README.md#imported-lut-bundles).

| Command | Does |
|---|---|
| `vibecut-cli scenes <file>` | Detect scene-change candidates without editing a project |
| `vibecut-cli media waveform <project> --asset <id>` | Generate and cache one asset's waveform peaks |
| `vibecut-cli media thumbnails <project> --item <id> --pixels-per-second <zoom> [--start <tick> --end <tick>]` | Generate and cache one clip's strip frames |
| `vibecut-cli media export-clips <project> --asset <id> [--asset <id>...] --dir <dir> [--overwrite]` | Export each asset's full source as its own file |
| `vibecut-cli probe <file>` | Probe a media file and report format, duration and streams |
| `vibecut-cli project new <path>` | Create an empty project, refusing to replace an existing file |
| `vibecut-cli project info <path>` | Summary plus optional view metadata under the project writer lease |
| `vibecut-cli project save-as <path> <new-path> [--replace]` | Publish a copy under a fresh project identity (plus its view metadata); an existing destination is refused unless `--replace` is given, and the project's own file always is |
| `vibecut-cli bundle export <project> <bundle-dir>` | Write a self-contained directory: project, manifest and hashed media copies; missing media is listed, not an error |
| `vibecut-cli bundle import <bundle-dir> <project-dir>` | Extract a bundle into a new project directory under a fresh identity; media absent from the bundle is listed for relinking |
| `vibecut-cli exec <project> <command_id> <json>` | Run one engine command under the project writer lease |
| `vibecut-cli normalize <project> [--clip] [--master] [--target-lufs -14] [--true-peak-db -1]` | Measure and store loudness gains for clip or master scopes |
| `vibecut-cli control <project>` | Run a revision-bound control session over newline-delimited JSON |
| `vibecut-cli mcp <project>` / `mcp --attach --socket <endpoint>` | Serve [MCP control](mcp.md) over stdio, headless or attached to the app |
| `vibecut-cli export <project> --out <mp4\|mov\|webm\|m4a>` | Export saved scene(s) to the container selected by the extension; `m4a` and audio-only output need `"audio_only": true` in `--settings` |
| `vibecut-cli batch <job-file>` | Render the validated export jobs in a JSON job file |
| `vibecut-cli subtitle <in> --to <lang>` | Transcribe, translate and export SRT/burned subtitles |
| `vibecut-cli captions import <project> <file.ass>` | Import ASS cues and styles as one caption item and print counts |
| `vibecut-cli cut <project> --remove-pauses [--min-pause-ms N] [--pad-ms N] [--crossfade-ms N] [--dry-run]` | List detected pauses, or remove them as one undoable edit |
| `vibecut-cli transcript <project> [--item <id> --cue <id> --words <first>-<last>]` | List cue/word output times, or delete a measured word range as one undoable edit |
| `vibecut-cli clips <in>` | Suggest ranked shorts from a long video and optionally export them |
| `vibecut-cli reframe <project> --item <id> --target 9:16\|1:1\|16:9 [--replace-manual] [--dry-run]` | Generate and apply the auto-reframe crop, printing per-shot reasons |
| `vibecut-cli chapters <project> --language <tag> [--max-chapters N] [--export <dir>]` | Generate chapters, title and description, and optionally export the sidecars |
| `vibecut-cli fillers <project> --language <tag> --llm [--include-markers]` | Classify fillers through the lent text provider and list candidates |
| `vibecut-cli providers` | List transcription, translation, clip, chapter and filler providers offline |
| `vibecut-cli model list\|info\|download` | Inspect or install pinned local model files |
| `vibecut-cli sounds search\|save\|unsave\|add` | Search Freesound with a user token or local saved sounds; save, unsave and add a cached sound |
| `vibecut-cli music search\|save\|unsave\|add` | Search the open music aggregator anonymously or read saved music; save, unsave and add a cached track |
| `vibecut-cli stickers search\|insert` | Search the bundled flags, or insert one as a sticker item |
| `vibecut-cli fonts list\|resolve\|download\|import` | Search bundled, system and catalog fonts, resolve a family, or download a catalog font |
| `vibecut-cli exec <project> keyframes.apply_preset '{"keys":[{"item":1,"path":"opacity","component":null,"time":0}],"preset":"ease_out"}'` | Ease the segment leaving each listed key with a named preset (`smooth`, `ease_out`, `ease_in`, `in_out`, `pop`, `linear`) as one undo step; `keyframes.set_handles` takes `"curve":[x1,y1,x2,y2]` instead |
| `vibecut-cli commands` | Print the generated command registry |
| `vibecut-cli schema` | Print the generated control request/response/event schema |

Add `--json` for a machine-readable result on stdout; without it stdout is human-readable. Progress and diagnostics go to stderr, so stdout stays a single result. Errors use the [exit codes](#exit-codes) and, with `--json`, one JSON object on the last stderr line.

`export` options: `--aspect 9:16` renders 1080×1920 and `--aspect 16:9` 1920×1080 (default: project canvas); `--overwrite` replaces an existing output; `--allow-degraded` is the consent described in [Export admission](#export-admission).

## Fonts

`fonts list [--query <text>]` lists families whose name contains the text, prefix matches first then alphabetical, from bundled, imported, OS-installed and curated downloadable families, one entry per name (bundled over verified custom import over cached download over system install over uncached download). Each entry has `origin` (`bundled`, `system`, `google`, `custom`), `state` (`available`, `not_downloaded`), `styles`, and for bundled and catalog families `license` id and `source` URL. `fonts resolve <family>` loads a family as a render would and prints `state` (`loaded`, `not_downloaded`, `cache_invalid`, `unknown`) and the `effective_family` layout uses; any state but `loaded` exits 4 and layout falls back to Inter. Neither uses the network.

`fonts download <family> --accept-license` is the only network action. It prints family, licence id, source URL and byte count to stderr and refuses with exit 5 without the flag. Files are fetched over HTTPS, verified against pinned size and SHA-256, stored with a licence file and per-file attribution under `<cache>/VibeCut/fonts`, and published as one directory; a failed or interrupted download leaves nothing. Font cache validation, load state and bundled/system fallback are owned by the [font catalog](../../../crates/libraries/src/font_catalog/README.md). `fonts import <file> --accept-license` copies a bounded, validated TTF, OTF or WOFF2 into the same store without network access; the flag acknowledges usage rights, and its absence refuses before reading. Imported source/hash/licence metadata and decoded bytes stay in the store, outside Project JSON. Exact hashes deduplicate; conflicting family/style bytes refuse. The commands are adapters over the registry entries `fonts.list`, `fonts.load`, `fonts.download` and `fonts.import`; a `control` session reaches the same operations with the font store lent by its composition root.

## Captions

`captions import <project> <file.ass|file.ssa> [--language <tag>] [--start <ticks>] [--track <id>] [--name <name>]` parses the script with `captions::parse_ass`, prints cue/style/skipped counts and line-numbered warnings, and applies `captions.import`. Asset, caption track and caption item are one undo step. `--track` targets an existing text track; `--name` labels a new one (default `Captions`). `--language` is the cue source language (default `en`); `--start` offsets the item in ticks. Input is capped at 16 MiB. Malformed lines warn and skip; an oversized file or a document with no surviving cue fails with a typed error and leaves the project unchanged.

## Media generation

`media waveform <project> --asset <id>` runs the engine's `GenerateWaveform` job for one project asset, waits for it, publishes the validated peak summary into the artifact cache and prints its sample rate, channel count, total samples and bucket count. A warm entry completes without opening the source, so a second run needs the media only while its cache entry survives. `media thumbnails <project> --item <id> --pixels-per-second <zoom> [--start <tick> --end <tick>]` warms the clip's visible output range (the whole clip when omitted) on the same 16:9 tile grid the display reads at that timeline zoom, then prints the cached source positions. Both commands read the project under the writer lease, never edit it and publish into the `waveform` and `thumbnails` directories under the default VibeCut cache root, shared with the control session. The same two jobs are the registry commands `media.generate_waveform` and `media.generate_clip_thumbnails` (see the generated [command reference](commands.md)); a control caller without attached media caches gets a typed refusal. `media export-clips <project> --asset <id> [--asset <id>...] --dir <dir> [--overwrite]` and the registry command `export.clips` export each selected video asset's whole source `[0, duration)` as `<source stem>.mp4` in the project's export settings, in request order; a repeated stem takes `<stem>-2.mp4` and onward. One engine job runs the files in order with overall progress. Cancelling stops the file in progress and removes its staged output; files already published stay complete. Used ranges, trims, effects and scenes play no part. Without `--overwrite`, any existing destination refuses the whole job before the first render. Through MCP the directory uses the granted output root, as for `clips.export`.

## Pause removal

`cut <project> --remove-pauses [--min-pause-ms 500] [--pad-ms 20] [--crossfade-ms 20] [--dry-run]` runs the engine's local speech-activity job over the active scene's audible items, prints the output-time candidate spans and, without `--dry-run`, applies them through `timeline.remove_pauses` and saves. One invocation is one undoable edit; `--dry-run` writes nothing. Defaults and their sources are in the [analysis contract](../../../crates/engine/src/analysis/README.md#pause-candidates). The job has no artifact cache, so a second run decodes the sources again. Candidates never cut inside a native caption word span, a span is padded by the silence it keeps and the join fades with the equal-power pair the reviewed spans imply.

## Filler removal

Local lexicon candidates and removal are the registry commands `timeline.filler_candidates` and `timeline.remove_fillers`; the generated [command reference](commands.md) owns their schemas, candidate shape and refusal rules.

`fillers <project> --language vi|en|zh|zh-Hans|zh-Hant --llm [--include-markers] [--provider openai|anthropic] [--model <id>] [--yes-upload] [--accept-unknown-price]` runs the engine's opt-in `ClassifyFillers` job over the active scene's visible measured words. The provider and model are the explicit user choice; defaults come from the AI defaults table, with no fallback; `--yes-upload` approves the exact provider, payload and cost printed on stderr, while without it the run refuses with exit 5 before any request. The model returns only exact word IDs from the transcript that was sent; an unknown or duplicate ID fails the job with the typed `InvalidSegments` reason instead of being dropped. Candidates use the same `id`, `range`, `kind` and `edited_cue` shape as `timeline.filler_candidates`, so the same `timeline.remove_fillers` applies them. `--include-markers` also allows discourse markers. `providers --operation fillers` lists the two providers offline; local lexicon detection remains the synchronous `timeline.filler_candidates` command and is never an automatic fallback.

## Chapters

`chapters <project> --language vi|en|zh|zh-Hans|zh-Hant [--max-chapters 10] [--provider openai|anthropic] [--model <id>] [--export <dir>] [--yes-upload] [--accept-unknown-price]` runs the engine's `GenerateChapters` job over the active scene's caption cues in output time. The provider and model are the user's explicit choice (OpenAI `gpt-6-luna` or Anthropic `claude-sonnet-5-5`; no fallback); `--yes-upload` approves the exact provider, payload and cost printed on stderr, while without it the run refuses with exit 5 before any request. Chapter starts snap to cue starts and increase inside the scene duration; an empty transcript, an unsupported language, cues without that language's text, overlapping or blank cues and a bound outside 1..=50 refuse with a typed reason and exit 5. `--export <dir>` publishes `<project>.txt` and `<project>.json` into an existing directory through the shared atomic publication owner; an existing file refuses and the project is never edited. The registry commands `chapters.generate` and `chapters.export_sidecars` and their schemas are in the generated [command reference](commands.md). `providers --operation chapters` lists the two providers offline.

## Transcript deletion

`transcript <project>` prints the active scene's caption cues and measured words with their output tick ranges, paired word text, and `protected` (explicitly edited cue text) and `locked` (track) flags; it writes nothing. `--item <id> --cue <id> --words <first>-<last> [--json]` deletes that cue's inclusive zero-based word range through `transcript.delete_range` and saves. Consecutive words form one cut span, words separated by an unselected word stay separate, and the edit closes vacated tails through the ripple path as one undoable edit. A word in another scene refuses with `SceneBoundary`, an edited cue with `Protected` and a locked track with `Locked`; each leaves the project unchanged and exits 5. A cut never rewrites cue text or timings. `transcript.merge_cues` and the other transcript registry commands are documented in the generated [command reference](commands.md).

## Subtitles

`subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual] [--srt out.srt] [--burn out.mp4] [--aspect 9:16] [--provider <id>] [--model <id>] [--translate-provider openai|anthropic] [--translate-model <id>] [--yes-upload] [--accept-unknown-price] [--overwrite]` runs the registry caption commands over an in-memory project holding `<in>`: `captions.generate` (speech extraction, ASR, checkpointed chunks, local alignment when the provider lacks native timing, cue segmentation), `captions.apply`, optional contextual `captions.translate` with `retranslate`, then `export_subtitles` and `burn_subtitles`. Pre-checks (bilingual languages, burn needs video, outputs differ and never replace the input) run before any job, so a refusal costs no upload. At least one of `--srt` and `--burn` is required. `--aspect` as for `export`; without it burn uses the source geometry.

Providers and models are explicit and never switched automatically. `auto` language detection requires the local `whisper.cpp` provider and resolves the source before deciding whether a translator is needed, so a same-language local run needs no cloud credentials; a different-language run needs the selected translation provider and its key. `providers --operation transcribe|translate --language <tag>` lists offline capabilities; `--model` accepts any bounded remote ID; downloaded local weights must be listed. Local defaults are `whisper-large-v3-turbo` (vi/en/zh) and `gipformer-68m-rnnt-int8` (vi); cloud transcription and translation defaults come from the AI defaults table. `--to zh` selects Simplified (`zh-Hans`); `zh-Hant` is explicit.

Remote requests are refused before any upload unless `--yes-upload` is present. The command prints the provider/model, languages, audio duration, upload bytes, per-invocation cost ceiling and the provider's policy links; the flag authorizes only those scopes, never a model download or another provider. Local providers need no consent.

ASR checkpoints are content-addressed complete windows in the model cache. A restart reuses valid windows, runs the missing ones and reports the reused count; a checksum failure or incomplete window is recomputed; an alignment failure keeps the recognition checkpoint and retries alignment. The CLI has no signal-driven cancel, so interruption has crash semantics. Local weights are never downloaded implicitly: `model download <id> --yes-download` prints licence, source, size and every file SHA-256 and requires the flag. A missing model returns a `setup` error naming that command.

`VIBECUT_ASR_THREADS` selects 1–32 local threads (default 4). `VIBECUT_ASR_GPU=0` forces CPU. `VIBECUT_LOCAL_ASR` names the shipped Gipformer runner; without it that provider returns a typed setup error. Credentials come from `OPENAI_API_KEY`, `ELEVENLABS_API_KEY` and `ANTHROPIC_API_KEY` and are never printed. A missing credential is a `setup` refusal before media or network work.

## Commands and schemas

`commands --json` is the authoritative generated catalog: `api_version`, `registry_hash`, the static control `limits`, `output_schema` (the JSON Schema 2020-12 shape of the result every command returns, carried once) and `entries`, each with command ID, label, effect, JSON Schema 2020-12 input shape and a fixture example. The `catalog` control command returns the same `output_schema` beside its `entries`. `schema --json` returns the same `api_version`/`registry_hash` with the request, response and event envelope schemas. There is no CLI-side command table ([architecture](../../engineering/architecture.md#contracts)). `exec` takes a command ID and its `params` object:

```sh
vibecut-cli exec demo.vibecut timeline.add_track '{"name":"Video","kind":"video"}'
```

The remaining `project.*`, `timeline.*`, `keyframes.*` and `audio.*` registry commands, including their schemas and examples, are documented in the generated [command reference](commands.md). Fields are required; unknown command, parameter and model fields are refused. Wire types are the Serde shapes of their crates; the [engine contract](../../../crates/engine/src/session/README.md#public-entry-points) covers the registry and JSON limits.

## Control session

`control <project> [--allow-overwrite] [--no-save]` is the CLI consumer of the [engine control contract](../../../crates/engine/src/control/README.md#control-admission-revisions-and-receipts). One process is one engine Session with an OS-random epoch. The writer lease is acquired before the first frame and held until the final save, so a desktop app holding it returns `busy`/3 before any request is read. Separate invocations share no revisions or receipts.

Requests are newline-delimited JSON objects on stdin: `request_id`, `command`, `params`, and optionally `idempotency_key`, `base_rev`, `api_version`, `registry_hash`, `session_id`. Omitted contract identity is filled from the running catalog; a wrong version or digest returns `schema_mismatch` and a wrong session `session_gone`. Read commands need no revision or key. Effectful commands need the current `base_rev` and a 1–128 byte `idempotency_key`. A stale revision returns `stale_revision` before any child of a batch; replaying an accepted key returns its original result; a different request under the same key returns `idempotency_conflict`. Receipts are never evicted: the next new effect after 4096 retained keys returns `receipt_capacity`. Unknown commands, unknown or extra fields, duplicate JSON keys, frames over 16 MiB and integers outside ±(2^53−1) are refused before effects; an oversized frame is drained and returned as an in-band `limit` error and the session continues. A session keeps 100 undo steps; `undo` and `redo` are ordinary frames. `project.create` stays with `project new` and is refused inside a session.

stdout is newline-delimited JSON (`--json` implied). Each line is the engine `Response` plus a stable `exit_code`. A frame that cannot become a request returns `{"status":"error","exit_code":N,"error":{"code":"…","message":"…","exit_code":N}}`. The process exits with the last frame's mapped code, so a one-frame session works like `exec`; earlier refusals stay in-band. Existing-code mapping: unknown/invalid/limit/schema 2, stale revision 3, permission/idempotency/receipt/transaction/empty/refusal 5, session/resync/missing-job/cancel/queue/failed 7; refined classes use the failure projections below. Replay returns the original effect revision even after a later save; use `inspect` for the current one.

`project.save` is the explicit save point. Admission, request-ID bounds, replay and receipt capacity are owned by `Session::control_lifecycle_save`; the CLI callback only publishes project and metadata files atomically. A refused or failed save keeps the session dirty and is reported in-band; success and typed execution errors are both receipted, so a retry that must execute again uses a fresh key. A changed project is otherwise saved once on clean EOF (a failed EOF save exits non-zero); `--no-save` suppresses that save. `--allow-overwrite` is the policy grant for `export`, `export_subtitles` and `burn_subtitles` with `overwrite: true`; without it the engine refuses with `permission`. External gesture commands are refused; agents use atomic `batch` plus `undo`.

The session ticks the engine job runner between requests. `preview` and `export` return job IDs; poll `job_status` (including retained inline preview JPEGs) and use `job_cancel` for graceful cancellation. `preview.snapshot {time, path}` is synchronous: it renders the frame at canvas size through the production compositor and publishes a PNG atomically without changing project bytes; the active project and metadata paths are protected, so an aliasing destination is refused. EOF cancels and joins outstanding jobs through the publication fence. There is no signal handler: interruption has crash semantics, the OS releases the lease and unsaved edits are lost.

## Batch job files

`batch <job-file> [--allow-degraded] [--allow-overwrite]` renders an ordered set of saved projects through the `export` job path. It never edits or saves a project, loads each distinct project once as an immutable snapshot before any job is submitted, and does no network, provider, download or subtitle work.

The file is a JSON object with exactly `version` (`1`) and `jobs` (1–1000 entries), at most 1 MiB, with distinct project snapshots capped at 64 MiB aggregate. Each entry has exactly `project` and `output`, plus optional `aspect` (`project`, `9:16`, `16:9`; default `project`), `allow_degraded` and `overwrite` (default `false`). Relative paths resolve from the job file's directory. Paths are non-empty UTF-8 of at most 4096 bytes naming a file, and `output` parents must exist. Outputs must be distinct and must not alias a project, project source or the job file. Hard-linked outputs are refused; on Windows and macOS case-only collisions are refused even on case-sensitive directories, and macOS also equates composed and decomposed Unicode spellings. An existing output is refused unless its entry sets `overwrite` and the command has `--allow-overwrite`. An entry setting `allow_degraded` requires `--allow-degraded`. Neither grant can come from the file alone.

```json
{
  "version": 1,
  "jobs": [
    {"project": "a.vibecut", "output": "out/a.mp4"},
    {"project": "b.vibecut", "output": "out/b-vertical.mp4", "aspect": "9:16"}
  ]
}
```

Malformed or truncated JSON, duplicate keys, unknown fields, an unsupported version, wrong types, empty or oversized `jobs`, invalid paths and unsafe destinations are typed refusals before any job or output. A non-regular job file is refused; a missing or unreadable one is an I/O failure.

Jobs run in file order on one export worker. The result has `status` (`completed`, `failed`) and a `jobs` array in file order; each entry has `index`, `project`, `output`, `status` (`completed`, `failed`, `skipped`) and, on failure, a typed `error`. The first failed or cancelled job stops the batch: later entries are `skipped`, completed files stay, and the process exits with that job's code. A failed or cancelled job never promotes a partial file and preserves an existing destination. Interruption has crash semantics; there is no signal-to-cancel handler.

## Project files and concurrency

A project is the `.vibecut` file owned by the project crate: [project contract](../../../crates/project/src/persistence/README.md#envelope-and-limits). Lease admission follows the [CLI owner contract](../../../apps/vibecut-cli/README.md).

Saved caption looks use the ordinary edit and export paths:

```sh
vibecut-cli exec project.vibecut captions.apply_style '{"item":4,"style":{"kind":"preset","preset":"box"}}'
vibecut-cli export project.vibecut --out captions.mp4
```

The item ID comes from inspection. `custom` takes a complete `CueStyle` in `style`; see the
[generated command reference](commands.md). SRT/VTT text stays
unchanged; there is no subtitle preset flag or preset field in export settings.

Text animations and saved styles use the ordinary edit path:

```sh
vibecut-cli exec project.vibecut text.apply_animation '{"item":3,"preset":"slide_in"}'
vibecut-cli exec project.vibecut text.save_preset '{"item":3,"name":"Style 1"}'
vibecut-cli exec project.vibecut text.apply_preset '{"items":[3],"preset":0}'
vibecut-cli export project.vibecut --out title.mp4
```

Use inspected Text item IDs; animation choices are `fade_in`, `fade_out`, `fade_in_out`,
`slide_in`, `slide_out`, `slide_in_out`, `scale_in`, `scale_out` and `scale_in_out`.
`text.save_preset` reports the new preset ID in `created`; `text.apply_preset` takes one
or more Text item IDs. The [edit owner](../../../crates/engine/src/session/edit/README.md#text-animation)
defines curves, duration, bases and conflict rules. CLI control and MCP use these same
commands.

## Export admission

The output extension selects MP4/H.264, MOV/H.264 or WebM/VP9 through the shared render owner; with `audio_only: true` it selects M4A/AAC (`m4a` or MP4) or WebM/Opus and no video field is allowed. `--settings <JSON>` is `render::OutputOptions`: `version: 1`, nullable `resolution: [width,height]`, `fps: {num,den}`, `bit_rate`, `codec: automatic|h264|vp9`, `profile: automatic|main|high`, `audio_only: bool`, `range: {kind: full}` or `{kind: in_out|selected, start, end}` in ticks, and `scene: {kind: active|all}` or `{kind: selected, scene: <id>}`. All fields are required; other versions and unknown fields refuse. Defaults and bounds: [render contract](../../../crates/render/README.md). `--aspect` overrides output geometry; an explicit settings resolution wins. All scenes use distinct `<stem>-scene-<id>.<ext>` destinations. JSON success adds `paths` with every completed file; `path` stays the requested stem.

Output is staged and published only after encoding, muxing and a final source revalidation. A required source that is missing or whose content hash changed refuses the export with exit 4 and `unavailable` entries naming asset ID, item ID, source tick range and reason (`missing`, `changed`, `metadata`, `video`, `audio`); no partial file is promoted and an existing destination is preserved.

Degraded output is never a default. `--allow-degraded` admits initially missing or changed visual sources as black/silence; a source that fails during execution still refuses. Fence details: [render contract](../../../crates/render/src/export/README.md#export-and-publication).

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

With `--json`, an error is `{"status":"error","error":{"code":"busy","message":"…", "exit_code":3,"unavailable":[…]}}`; `unavailable` appears only for source admission failures. Messages never contain project text, media text, provider payloads or API keys. Subtitle errors add the codes `setup` (missing local model, runner or credential), `ai` (provider execution) and `rejected` (refused upload consent or unsupported language). A `control` session uses its [protocol code projection](#control-session). `batch` writes its result to stdout and exits with the first failed job's code, or 0; refusals of the job file use the stderr envelope.

Typed engine diagnostics project the owner's `Failure` code to the documented label and exit.
Stdout control frames separately project that code to protocol exits. Direct errors retain
those exits: `budget` uses 2, `stale_result` uses 3, `worker_failure` and `io` use 7, and the
remaining refined classes use 5. `RuntimeError::Failed` delegates its inner wire code, so a wrapped
limit, stale result or IO failure intentionally becomes control exit 2, 3 or 7 respectively.
Recovery persistence failures also delegate the stored cause's code: disk IO uses `io`/7,
worker unwind uses `job`/7, limits use `usage`/2 and budget refusal uses `job`/7 in typed CLI
diagnostics. Stdout control keeps its separate projection: IO and unwind use exit 7;
limits and budgets use 2.
These failures retain the safe message `session persistence failed` and remain non-transient;
code delegation does not retry a sticky write or expose its cause's diagnostic payload.
The current API version and catalog digest are published by `commands --json`.

## Sounds

`sounds search --library <dir> [--page N] [--page-size N] [--all-licences] [query]` runs the registry command `sounds.search`, calling Freesound APIv2 when `FREESOUND_API_KEY` is set and otherwise searching saved local sounds. Results are commercial-only by default, sorted by downloads, each with author, licence, attribution, source page and preview URL. The CLI opens `--library` and reads the credential in its composition root; neither is a command parameter, and the token is never printed, stored or put in a URL. Provider failures use the codes `auth`, `quota`, `rate_limit`, `network`, `setup`; a missing credential degrades to local search.

`sounds save <library> <sound-json>` and `sounds unsave <library> <id>` run `sounds.save` and `sounds.unsave` against the same lent library. `sounds add <project> <library> <id> [--start <ticks>]` requires a cached preview and commits one undoable audio item with `sourceType=library`, attribution and licence metadata. Cached previews are keyed by the exact provider URL.

The three library commands are read-effect registry entries (they never enter project undo) and declare `grant = Human`, so an agent transport cannot search, save or unsave the human's library.

## Music

`music search --library <dir> [--tag <tag>] [--saved] [--page N] [--page-size N] [query]` runs the registry command `music.search`. Without `--saved` it queries the open music aggregator anonymously, or with the user's own `OPENVERSE_API_KEY` when the environment sets one; `--saved` reads the local saved collection and never resolves a credential or touches the network. Every result carries title, author, licence name, licence deed URL, attribution, source page, audio URL and duration; only public-domain-dedication and attribution-only licences are admitted, and any other row is dropped even if the source ignores the filter. `--tag` and a free-text query are mutually exclusive. The CLI opens `--library` and reads the optional key in its composition root; neither is a command parameter, and the key is never printed, stored or put in a URL.

`music save <library> <track-json>` and `music unsave <library> <id>` run `music.save` and `music.unsave` against the same lent library. `music add <project> <library> <id> [--start <ticks>]` requires a cached track and commits one undoable audio item with its title, author, licence, deed, attribution and source page. Cached audio is keyed by the exact provider URL and copied into project-owned media so library cleanup never removes bytes a project references.

The three library commands are read-effect registry entries and declare `grant = Human`; `music.add_to_timeline` is one edit-effect entry with grouped undo. An agent transport is lent no music access, so MCP and generic control refuse these commands before any callback, network or filesystem work.

## Clip suggestions

`clips <in> [--from vi|en|zh|auto] [--count N] [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s] [--overlap R] [--keywords a,b] [--provider heuristic|openai|anthropic] [--model <id>] [--asr-provider <id>] [--asr-model <id>] [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual] [--aspect 9:16] [--translate-provider openai|anthropic] [--overwrite] [--allow-degraded]] [--yes-upload] [--accept-unknown-price]` runs the registry commands `clips.suggest` (ASR, ranking, edge snapping) and, with `--export`, `captions.translate` + `retranslate` and `clips.export`, and prints candidates as JSON. `heuristic` is the local default and needs no upload. Remote suggestion uploads transcript segments only, never audio, and needs `--yes-upload` after the command prints provider, endpoint, byte count, cost ceiling and the provider's retention/training policy link. A failed provider is a typed failure; the command never falls back to another provider or the heuristic.

`--select` takes 1-based candidate indices (default: all). `--export` requires `--subs` and starts one export job per selected short at 1080×1920 (`clips.export`), burning the requested language; when `--subs` differs from the detected source, or with `--bilingual`, subtitle translation runs first through the caption commands. A failed short cancels and drains the others; shorts already published stay complete. Outputs are `<title>.mp4` with no sidecars. Existing files are preserved unless `--overwrite`.

Candidates carry `pieces` (the provider's play-order pieces with roles), `segments` (snapped source ranges) and `adjustments` (signed original/adjusted delta per edge). The `word_fade` kind marks frame rounding at a sub-frame word gap; [shorts snapping](../../../crates/shorts/src/boundaries/README.md) owns its overlap and export-audio policy. Rejected snaps and dropped suggestions are reported with typed reasons. `providers --operation clips` lists the offline suggestion catalog with policy links.

## Reframe

`reframe <project> --item <id> --target 9:16|1:1|16:9 [--replace-manual] [--dry-run]` runs the registry job `reframe.generate {item,target}` on one video item, prints every shot's source start, analysed key count and fallback reasons, then applies the finished job with `reframe.apply {job,replace_manual}` and saves once. Shots whose `crop.auto` key is false were marked manual with `reframe.set_manual {item,at}` and keep their position keys unless `--replace-manual` is passed; `reframe.clear {item}` removes all three crop channels. `--dry-run` prints the report without editing. A generate job never edits the project, and apply refuses a stale revision, an evicted or unfinished job or an item changed since the job started. Face following is not speaker awareness.

Generation needs the pinned local `yunet-2026may` face model installed through `model download yunet-2026may --yes-download`; nothing downloads implicitly. The detector loads only that graph and its fingerprint enters the analysis cache key. The command family is the same registry entries the control session and MCP expose.

## Scene detection

`scenes <file> [--threshold 0.28] [--fade-threshold 0.20] [--min-scene-ticks <ticks>] [--json]` imports a local known-duration video into a throwaway in-memory project and runs the registry command `media.detect_scenes {asset, settings}` (the engine [scene detection job](../../../crates/engine/src/analysis/README.md#scene-detection)) against the process media cache; control sessions run the same command for assets already in the project. Human output prints source ticks, change kind and confidence; JSON adds the content key, source identity, requested range, ordered `candidates` and `decoded_frames`. Confidence is measured change, not a probability. Explicit settings enter the content cache key. The command never edits a project and has no signal-driven cancel; engine callers use the job cancellation token.

Keyword emphasis uses the same registry: `captions.select_emphasis {item,cue_ids,mode}` (`mode` `lexicon`, the default, is local and deterministic) and `captions.set_emphasis {item,cue_id,word_ids}` (an empty list clears). A cue without verified word timing refuses with a typed error and changes nothing. `captions.suggest_emphasis {item,cue_ids?}` is the optional LLM path through the lent text provider: it needs the same upload approval as chapters and fillers, returns suggestions as job output for `captions.set_emphasis`, and never falls back to the lexicon on refusal or provider failure.

Named caption values use the ordinary `exec`/control registry: `captions.save_preset {item,name}`, `captions.rename_preset {preset,name}`, `captions.delete_preset {preset}` and `captions.apply_style {item,style:{kind:"saved",preset,overrides}}`. Save returns a created caption preset ID (human output: `created caption preset N`); `inspect` exposes the project bank. A saved apply selects the bank value live with an optional sparse override object; built-in or custom values materialize and clear that selection, and deleting a selected preset refuses. Export consumes the effective item style without another preset flag.

For an unknown-price remote model, `--yes-upload` also requires
`--accept-unknown-price`. This explicit acknowledgement applies to the exact
selected provider/model and job kind disclosed by that command; no known money
ceiling is fabricated. Existing token and byte bounds still apply. Job commands
that receive resolved provider access (`chapters.generate`, `fillers.classify`)
use that composition's model and consent, with no client override fields.

Stickers: `stickers search [--category all|flags] [--limit N] [query]` runs `stickers.search` over the bundled catalog (254 country flags and 17 regional or organisation flags with region and continent keywords; stickers are flags only, so `all` and `flags` list the same rows) and prints each `provider:value` id with its intrinsic size. `stickers insert <project> <id> [--start <ticks>] [--duration <ticks>]` runs `stickers.insert`, which resolves the id, stores it with the intrinsic size (flags 640x480) and inserts a `sticker` item on a sticker track (duration defaults to 5 seconds). A `control` session reaches both commands with the catalog lent by the CLI; `exec` cannot run `stickers.insert` because it lends no catalog, and an MCP session lends none yet. A sticker takes transform, blend and effects but no masks. Render rasterises flag SVGs at the destination size; a missing asset, or a retired `shapes:` id, paints a placeholder and the renderer reports a typed `StickerError`. Shapes are graphic items inserted by the `graphic.insert.*` commands, not stickers.

Rectangle, ellipse, polygon and star items use `timeline.insert` with `content.kind=graphic`, a closed `definitionId` and explicit appearance. Edit fallbacks with `timeline.set_item_param` graphic patches, or ordinary `keyframes.*` paths for authoritative appearance. The long-lived `control` session accepts `preview.snapshot {time,path}` to publish PNG; one-shot `exec` refuses it. `export` encodes video through the same compositor. These commands are also available through the framed MCP adapter.

## Clipboard

A long-lived `control` session supports `clipboard.copy`, `clipboard.copy_keyframes`,
`clipboard.read`, `clipboard.paste` and `clipboard.paste_keyframes`. Read returns
`{kind:"clipboard", payload:...}` for the keyframe slot; a client may send that
version-one payload explicitly with `source:{kind:"payload",payload:...}`. Item paste
also accepts an explicit version-one item payload. Use `source:{kind:"stored"}` within
the same Session. Effectful calls use ordinary revision/idempotency admission; Copy
advances revision without dirtying the project. One-shot `exec` can paste explicit
serialized payloads but a second process inherits no store, and `exec` does not execute
reads. Exact shape: [commands](commands.md).
Transfer and lifetime: [clipboard](../../../crates/engine/src/session/README.md#clipboard).
