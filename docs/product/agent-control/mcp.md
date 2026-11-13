# MCP control

The [mcp crate](../../../crates/mcp/README.md) owns the MCP adapter over the [engine control contract](../../../crates/engine/src/control/README.md#control-admission-revisions-and-receipts); the CLI's `mcp` subcommand and the desktop app compose it. It uses the official `rmcp` SDK and targets MCP 2026-07-28. MCP Tasks are not advertised. Preview and export start engine jobs; poll `job_status` and stop them with `job_cancel`.

## Start and attach

```sh
vibecut-cli mcp /absolute/project.json
vibecut-cli mcp /absolute/project.json --output-dir /absolute/exports
vibecut /absolute/project.json --control-socket /private/control/editor.sock
vibecut-cli mcp --attach --socket /private/control/editor.sock
```

On Windows the endpoint is a local named-pipe name such as `vibecut-editor`, not a socket path. Attach always exposes stdio to the client. The app must start with `--control-socket`; its optional `--control-output-dir` is the trusted output grant, and client attach arguments cannot broaden it.

Headless mode holds the project writer lease for one process Session, saves changed content on EOF and joins/cancels jobs on shutdown. `--no-save` suppresses the EOF save; explicit `project.save` still uses engine admission, publication and receipts. On desktop, persistence stays with Workspace/Autosave and MCP `project.save` returns a typed permission refusal.

`--control-background <project> <endpoint> <recovery-directory> <seconds>` runs the real app Workspace with no window or audio device, for at most one hour. It never creates a native window or activates the app (macOS uses accessory activation policy). Normal launches show the editor.

## Tools and results

The default surface is `command_list`, `command_run`, `inspect`, `preview`, `undo`, `redo`, `export`, `job_status`, `job_cancel` and `batch`. Headless `--all-tools` also exposes every registry command, with dots in tool names replaced by underscores. Catalog and parameter schemas come from the engine registry.

Curated tools and `command_run` accept the engine request envelope; a curated tool fixes `command` to its registry ID. `api_version`, `registry_hash` and `session_id` may be omitted on a fresh call and are filled from the running contract. Send `inspect` with `request_id`, `command: "inspect"` and `params: {}` first to get the Session identity and current revision. `command_list` takes optional numeric `cursor` and `limit` (1–16) and returns `next_cursor`; each page carries `output_schema` once, the schema of the result every command returns, beside its `entries`, which have no output schema of their own. `tools/list` paginates at 16 tools per page and declares no per-tool `outputSchema`: the field is optional in MCP, every tool returns that one shared result shape, and repeating it on each tool would add 111 KB per tool to the listing.

A visible desktop request may carry `project_id` (the project's decimal string) to select an open Editor window; omission selects the focused editor. Each project keeps its own Session identity, revisions, receipts and jobs. Unknown or closed projects return `session_gone`; an editor that is saving returns a typed busy refusal. Closing a window cancels only its control jobs. The headless CLI keeps its single-project envelope and tool schemas.

Mutations require `base_rev` and `idempotency_key`. A retry keeps the original Session, revision and key. The engine keeps receipts for the running Session and reconnecting attach processes share the ledger; it never retries a mutation itself. A new Session epoch refuses the old request, capacity exhaustion refuses before effects, and changed parameters under an accepted key return `idempotency_conflict`. Generic calls and nested batches use the same admission and grants as curated tools. External gestures and imported source paths are refused. MCP cannot create, duplicate, move to Trash, restore, purge or thumbnail projects: the project library belongs to the person, and each of those is a Human-granted command.

`structuredContent` carries the engine response unchanged, except preview JPEG arrays become MCP image blocks that keep the frame/clip/revision metadata. `media.generate_waveform`, `media.generate_clip_thumbnails` and `media.detect_scenes` run against the same process caches the CLI control session lends, opened at the default cache roots; they admit no new source path. `audio.measure_loudness {}` measures the project's loudness targets without a cache. A job-start command returns `job` and `revision`; `job_status` carries the finished data in one tagged `output` (`preview`, `pauses`, `chapters`, `fillers`, `loudness`, `scenes`, `captions`, `translations`, `reframe`), and the agent applies the result with `base_rev` = that `revision`. A job never edits the project by itself. `reframe.generate {item,target}` needs the local vision detector lent by the composition root; without it the command refuses `unsupported`. `reframe.apply {job,replace_manual}` reads the retained output of that finished job, so it refuses an evicted job or a project changed after the job started. Engine limits govern preview count, size, quality and burned-in labels. Export results carry file paths. Engine refusals set `isError` and keep the engine code. Adapter errors use `error.code`, including `queue_full`, `limit`, `timeout_unknown_outcome`, `cancelled_unknown_outcome` and `session_gone_unknown_outcome`. After an uncertain outcome, retry only the original key and request; a fresh key is a new operation. Cancellation discards queued work; an already committed atomic edit stays committed and receipted. Accepted jobs survive an attach disconnect and can be inspected or cancelled after reconnect.

## Transport and file permissions

Unix sockets require a real, owner-owned directory with no group/other access; new endpoint directories get mode 0700 and the socket 0600. On macOS socket permissions are set after bind inside the verified directory, because the OS rejects pre-bind `fchmod`; the process umask is never changed. Both ends check the OS peer effective user ID. Broad, foreign or symlink endpoint directories are refused rather than repaired, and existing socket names are never overwritten.

Windows pipes reject remote connections and have a protected DACL granting the owner access. The attach client compares the connected pipe's owner with the OS-assigned owner of a new anonymous pipe, using open kernel handles. App and client must share ownership and elevation context. No bearer token or authentication environment variable exists.

The grant policy admits individual commands and refuses every other command with typed `permission`, including commands added to an existing effect class. Source-bearing sound/caption imports, the sound-library commands (`sounds.search`, `sounds.save`, `sounds.unsave`) and font downloads are refused; MCP cannot admit new source paths. Media already in a human-opened project is trusted for rendering. File publication requires a pre-existing output directory grant, an absolute immediate-child destination and `overwrite: false`. The parent must name the granted directory or its canonical path exactly, checked before filesystem resolution; aliases and other roots are refused, and canonical parent revalidation refuses directory/symlink escapes. Render's admission and publication fence remain authoritative. The client cannot create directories, overwrite files, upload media or grant itself provider consent. `chapters.generate` and `fillers.classify` are admitted only through the `TextAccess` the composition root lends to the inbox: without a stored human approval they refuse `permission` before building the transcript input or touching the network, and a granted run uses the approved provider's exact payload and cost. The command determines whether its destination is a file or directory; a directory is refused for a file destination. `chapters.export_sidecars {dir, document}` accepts only the exact human-selected directory spelling or its stored canonical root, checked before any client-path filesystem query; other directory aliases are refused. It then revalidates the canonical directory and follows the output grant above: a pre-existing granted directory, an immediate-child `<project>.txt`/`<project>.json` pair and no overwrite.

Limits: 16 MiB per JSON-RPC frame in each direction, 64 incoming messages per second per connection, eight attached connections, 16 queued engine calls shared across clients, four outstanding protocol requests and four queued replies per connection (one task owns output IO), 30 s per call. A partial frame, initialization, connect and writes each get five seconds. Accept errors log a bounded category and retry after 100 ms until shutdown. Duplicate or oversized protocol IDs and oversize, duplicate-key, truncated, excessively deep or rate-exceeding frames close that transport; accepted app mutations stay in the ledger. Tool output is bounded including text/structured-content duplication and image base64. No parameters, paths, peer text or cancellation reason are logged.

## Client configuration

Use installed absolute paths. For attach, set `args` to `["mcp", "--attach", "--socket", "vibecut-editor"]` on Windows, or the private socket path on Unix.

Claude Code `.mcp.json`:

```json
{"mcpServers":{"vibecut":{"type":"stdio","command":"/absolute/vibecut-cli","args":["mcp","/absolute/project.json"]}}}
```

Codex `config.toml`:

```toml
[mcp_servers.vibecut]
command = "/absolute/vibecut-cli"
args = ["mcp", "/absolute/project.json"]
```

OpenCode v2 `opencode.json`:

```json
{"mcp":{"servers":{"vibecut":{"type":"local","command":["/absolute/vibecut-cli","mcp","/absolute/project.json"]}}}}
```

Sources: [Claude Code](https://code.claude.com/docs/en/mcp), [Codex](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), [OpenCode v2](https://github.com/anomalyco/opencode/blob/dev/services/www/src/docs/content/mcp-servers.mdx) and its [config schema](https://opencode.ai/config.json), [MCP stdio](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [rmcp](https://docs.rs/rmcp/3.8.0/rmcp/), [local sockets](https://docs.rs/interprocess/2.4.7/interprocess/local_socket/), [Windows security capture](https://docs.rs/win-authorization-sys/0.1.0/win_authorization_sys/).

`fonts.import` is a human-granted source-file operation: accepting font usage rights
in request JSON never grants an agent permission to read arbitrary files. Existing
font listing and loading grants are unchanged; font store custody and format bounds
are owned by the [font catalog](../../../crates/libraries/src/font_catalog/README.md#custom-import).

Caption generation and translation use the [engine caption workflows](../../../crates/engine/src/subtitles/README.md#caption-workflow-commands).
An attached composition lends `CaptionAccess` through `Inbox::attach_captions`; its prompt owns
human upload approval. Headless MCP lends local speech resources and `DenyUploads`: missing
optional ASR setup refuses caption work without preventing ordinary inspection or edits.
`captions.generate` names an existing asset, `captions.apply` names its retained completed job,
and `captions.translate` names an existing caption item. Apply with the accepted revision, then
publish with `export_subtitles` under the existing output grant. A remote run without approved
consent fails before that window's upload; a client cannot add consent or source paths.

`clips.suggest {asset, ...}` starts one suggestion job over an existing asset through the same
`CaptionAccess`, which also lends the clip workflow (`CaptionAccess::with_clips`). Its output
(`clips`) carries ranked candidates and the source-timed cues. `heuristic` (the default) uploads
nothing; a remote provider uploads transcript segments only, after the lent prompt shows
provider, endpoint, byte count, cost ceiling and the provider's retention/training link, and a
client cannot grant that approval. A missing credential or refused consent is a typed failure,
never a fallback to another provider. `clips.export {job, select, directory, ...}` names the
retained completed suggestion job and starts one export job per selected short under the
existing output directory grant (never overwriting); an optional `item` names the caption item
(source cues plus any `captions.translate` result) whose languages are burned.

The six noncurated Agent commands `clipboard.copy`, `clipboard.copy_keyframes`, `clipboard.cut`,
`clipboard.read`, `clipboard.paste` and `clipboard.paste_keyframes` run through
`command_run` (and their generated all-tools names). Read returns the bounded keyframe
payload in structuredContent. Attached UI and MCP share the selected Editor's Session
store; other editors and headless processes start empty. No item or keyframe payload is
written to the system clipboard. Transfer and lifetime:
[clipboard](../../../crates/engine/src/session/README.md#clipboard).
