# mcp

## Purpose

Serves the MCP tool surface over stdio and the OS-permissioned attach transport for the CLI and the desktop app.

Engine owns the command registry, control admission, revisions, receipts, job policy and rendering; the desktop and CLI own when a server lives and how a session is saved. This crate owns the protocol adapter, grants, inbox and transport only. It has no UI, argument parsing, process lifetime or project persistence: `vibecut-cli mcp` and the desktop composition roots supply those. [MCP control](../../docs/product/agent-control/mcp.md) owns the tool surface, grants, bounds, lifecycle, cancellation/retry rules and client configuration.

## Module map

| Module | Responsibility | Contract |
|---|---|---|
| `error.rs` | States adapter refusals and the engine or transport failures around them. | This page |
| `grants.rs` | Holds the trusted command-line grants that bound every client request. | This page |
| `handler.rs` | Serves the MCP tool surface over the inbox queue. | This page |
| `inbox.rs` | Queues validated MCP requests for the engine owner to execute one per poll. | This page |
| `server.rs` | Listens for MCP clients inside the running desktop process. | This page |
| `socket.rs` | Authenticates local MCP peers with OS-owned access controls. | This page |
| `transport.rs` | Bounds and strictly frames newline-delimited JSON-RPC before `rmcp` parses it. | This page |

`inbox` opens the same waveform and thumbnail caches the CLI control session lends, so an allowed agent call to `media.generate_waveform` or `media.generate_clip_thumbnails` runs instead of refusing for lack of a store.

## Entry points

`Grants::{output, output_granted}` and `Grants::default` carry the human's output grant. `Inbox::new` builds the headless queue and cache, `Inbox::attach_media` and `Inbox::attach_text` lend the desktop's process caches and transcript adapters, and `Inbox::poll`, `poll_projects`, `execute_project`, `observe`, `retain_projects` and `register_project` serve the desktop routing envelope. `Inbox::drained` reports when every transport closed and the queue is empty. `Server::bind` and `bind_with_media` own a desktop listener and its runtime; `Handler::all_tools`, `Framed` and `proxy` let the CLI compose its stdio and attach modes. `Error` carries a rejection, an IO failure or an engine failure.

## Limits

Engine limits govern frames, responses, preview count, size, quality and receipts; the adapter's queue and connection bounds are owned by [MCP control](../../docs/product/agent-control/mcp.md#transport-and-file-permissions). A refused or expired request never reaches engine admission; accepted mutations stay in the session's receipt ledger after a disconnect.

## Tests

`cargo test -p mcp` runs the adapter contracts: grant decisions, catalog and pagination parity with the engine registry, hostile framing, flooded and slow clients, cancellation and deadline handling, desktop project routing, the consent gate, and the OS-owned Unix socket and Windows pipe permissions. The CLI's `tests/it/mcp.rs` covers the subcommand's stdio and attach composition end to end.
