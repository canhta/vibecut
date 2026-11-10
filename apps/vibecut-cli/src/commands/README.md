# Commands

Native command dispatch through `run(context, args)` adapters.

Siblings are grouped by project operations, media work, AI workflows, asset libraries and generated catalogs. Each leaf command module exposes a single `run(context, args)` entry point and keeps its helpers private; the group modules re-export them, and the media group also routes `waveform` and `thumbnails`. Commands use only the `args`, `output`, `session` and MCP interfaces, never sibling commands. The process context carries a 2 GiB memory setting and each job keeps its own budget. Engine owns export admission and execution; `session` only composes its entry points.
