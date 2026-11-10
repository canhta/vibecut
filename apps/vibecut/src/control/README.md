# Control

Interface: `poll_workspace`, `background`.

The optional control listener and background attach compose the CLI-owned adapter
over the live `Workspace`; requests run against the same `Workspace` and `Session` as
the visible app, and an omitted project target means the focused window. Syntax and
boundaries: [MCP control](../../../../docs/product/agent-control/mcp.md).
