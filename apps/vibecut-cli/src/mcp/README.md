# MCP

MCP surface: `grants` (trusted bounds), `inbox` (engine-side queue), `handler` (tool surface), `server` (desktop listener), `command` (the `mcp` CLI command), `socket`/`transport` (authenticated framing).

`mcp` owns the stdio protocol adapter and OS-permissioned attach transport, composed by the desktop app through this library. [MCP control](../../../../docs/product/agent-control/mcp.md) owns the tool surface, grants, bounds, lifecycle, cancellation/retry rules and client configuration. Engine owns schema, revision, receipt, job and rendering.
