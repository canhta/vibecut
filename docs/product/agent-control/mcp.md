# MCP control

The CLI owns the MCP adapter over the [engine control contract](../../../crates/engine/README.md#control-admission-revisions-and-receipts).
It uses the official `rmcp` SDK, targets MCP 2026-07-28 and negotiates older client
versions through the SDK. MCP Tasks are not advertised. Preview/export calls start
engine jobs; poll `job_status` and use `job_cancel` for graceful job cancellation.

## Start and attach

```sh
vibecut-cli mcp /absolute/project.json
vibecut-cli mcp /absolute/project.json --output-dir /absolute/exports
vibecut /absolute/project.json --control-socket /private/control/editor.sock
vibecut-cli mcp --attach --socket /private/control/editor.sock
```

On Windows, the endpoint is a local named-pipe name, for example
`vibecut-editor`, rather than a filesystem socket path. Attach always exposes
stdio to the client. The app must start with `--control-socket`; its optional
`--control-output-dir` is the trusted output grant. Client-side attach arguments
cannot broaden the app's grants. Headless mode holds the project writer lease
for one process Session, saves changed content on EOF and joins/cancels jobs on
shutdown. `--no-save` suppresses the EOF save; explicit `project.save` still uses
engine admission, publication and receipts. Desktop persistence stays with
Workspace/Autosave: MCP `project.save` returns a typed permission refusal there.

`--control-background <project> <endpoint> <recovery-directory> <seconds>` runs
the real app Workspace with no window or audio-device creation, for at most one
hour. It never creates a native window or activates the app; macOS uses accessory
activation policy. Background attach tests use this route. Normal launches retain
the visible editor.

Keep the background attach regression in the default suite: it checks the shipped
process entry point, app-owned Workspace and authenticated IPC without native UI
interaction. An in-process kittest cannot cover process launch or foreground
ownership. Widget interactions remain offscreen kittests; background startup
ordering is also covered deterministically in the in-process Workspace tests.

## Tools and results

The default surface is `command_list`, `command_run`, `inspect`, `preview`,
`undo`, `redo`, `export`, `job_status`, `job_cancel` and `batch`. Headless
`--all-tools` additionally exposes every registry command, replacing dots in tool
names with underscores. The catalog and parameter schemas come from the engine
registry. Both curated tools and `command_run` accept the engine request envelope;
`command_list` accepts optional numeric `cursor`/`limit` (1–16) and returns
`next_cursor`; MCP `tools/list` also paginates at 16 tools per page.
the curated tool fixes `command` to its registry ID. `api_version`, `registry_hash`
and `session_id` may be omitted for a fresh call and are filled from the running
contract. Send `inspect` with `request_id`, `command: "inspect"` and `params: {}`
first to obtain the Session identity and current revision.

Visible desktop requests may include `project_id` as the project's decimal
string to select an open Editor window; omission selects the focused editor.
Each selected project retains its Session identity, revisions, receipts and jobs.
Unknown or closed projects return `session_gone`; a temporarily saving editor
returns a typed busy refusal. Closing one window cancels only its control jobs.
The headless CLI retains its single-project envelope and tool schemas.

Mutations require `base_rev` and `idempotency_key`. Keep the original Session,
revision and key on a retry. The engine keeps receipts for the running Session;
reconnecting attach processes share that ledger. It never automatically retries
a mutation. A new Session epoch refuses the old request, and capacity exhaustion
refuses before effects. Changing parameters under an accepted key returns
`idempotency_conflict`. Generic calls and nested batches use the same admission
and grants as curated tools. External gestures and imported source paths are
refused. MCP cannot create, duplicate, delete or thumbnail projects; those lifecycle
operations remain with their existing owners.

`structuredContent` contains the engine response unchanged, except that preview
JPEG arrays become MCP image blocks, retaining the frame/clip/revision metadata.
Engine limits govern preview count, size, quality and burned-in labels. Export
results carry file paths. Engine refusals set `isError` and retain the engine code.
Adapter errors use `error.code`, including `queue_full`, `limit`,
`timeout_unknown_outcome`, `cancelled_unknown_outcome` and
`session_gone_unknown_outcome`. After an uncertain outcome, retry only the original
key/request; a fresh key is a new operation. Cancellation discards queued work;
an already committed atomic edit remains committed and receipted. Accepted jobs
survive an attach disconnect and can be inspected or cancelled after reconnect.

## Transport and file permissions

Unix sockets require a real, owner-owned directory with no group/other access;
new endpoint directories are created with mode 0700 and the socket with 0600.
On macOS, socket permissions are set after bind inside the verified private
directory because the OS rejects pre-bind `fchmod`; the process umask is never
changed. Both ends check the OS peer effective user ID. Broad/foreign/symlink endpoint
directories are refused rather than repaired; existing socket names are never
overwritten. Windows pipes reject remote connections and have a protected DACL
granting their owner access. The attach client compares the connected pipe's
owner with the OS-assigned owner of a new anonymous pipe, using open kernel
handles rather than a PID-to-user lookup. App and client must have matching
ownership/elevation context. No bearer token or authentication environment
variable exists in client config.

The grant policy explicitly admits individual commands and refuses every other
command with typed `permission`, including commands added to an existing effect
class. Source-bearing sound/caption imports and font downloads are refused.
MCP cannot admit new source paths. Media already in a human-opened project is
trusted for rendering. File publication requires an explicit pre-existing output
directory grant, an absolute immediate-child destination and `overwrite: false`.
The parent must name the selected directory or its canonical path exactly, checked
before filesystem resolution; client-selected aliases and other roots are refused.
Canonical parent revalidation refuses directory/symlink escapes; render's existing
admission and publication fence remain authoritative. The client cannot create
directories, overwrite files, upload media or grant itself provider consent.

Limits are 8 MiB per incoming/outgoing JSON-RPC frame, 64 incoming messages per
second per connection, eight attached connections and 16 queued engine calls
shared across clients. Accept errors log a bounded category and retry after
100 ms until server shutdown. Each connection permits four outstanding protocol requests
and four queued replies, with one task owning output IO. Duplicate or oversized
protocol IDs close the connection. A partial frame has five seconds to finish; initialization,
connect and writes also have five-second deadlines. Calls have a 30-second
deadline. Oversize, duplicate-key, truncated, excessively deep or rate-exceeding
frames close that transport; accepted app mutations remain in the ledger.
Tool output is bounded including text/structured-content duplication and image
base64. No parameters, paths, peer text or cancellation reason are logged.

## Client configuration

Replace the executable and project paths below with installed absolute paths.
For attach, replace `args` with `["mcp", "--attach", "--socket", "vibecut-editor"]`
on Windows, or use the private socket path on Unix. No token is needed.

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

Configuration sources: [Claude Code](https://code.claude.com/docs/en/mcp),
[Codex](https://learn.chatgpt.com/docs/extend/mcp?surface=cli),
[OpenCode v2 source](https://github.com/anomalyco/opencode/blob/dev/services/www/src/docs/content/mcp-servers.mdx)
and [current configuration schema](https://opencode.ai/config.json).
Protocol and adapter sources: [MCP stdio](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports),
[rmcp](https://docs.rs/rmcp/3.8.0/rmcp/),
[local sockets](https://docs.rs/interprocess/2.4.7/interprocess/local_socket/),
[Windows security capture](https://docs.rs/win-authorization-sys/0.1.0/win_authorization_sys/).

Run `cargo test -p vibecut-cli -p vibecut mcp` through the build wrapper for framing,
rate, deadline/cancellation, engine admission/receipt parity, OS permissions and
real-app background attach coverage. Windows additionally asserts that the
foreground-window handle is unchanged. Live client/version evidence and platform
verification status belong in the ticket report.
