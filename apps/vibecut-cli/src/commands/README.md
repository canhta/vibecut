# Commands

Native command dispatch through `run(context, args)` adapters.

Command siblings are grouped by project operations, media work, AI workflows,
asset libraries and generated catalogs. Each native command module exposes a single
`run(context, args)` entry point and keeps its helpers private. Commands use
only the `args`, `output`, `session` and MCP interfaces, never sibling commands.
The process context carries the existing 2 GiB memory setting; each job retains
its own budget with unchanged admission settings. Engine still owns actual
export admission and execution; `session` only composes the existing entry points.
