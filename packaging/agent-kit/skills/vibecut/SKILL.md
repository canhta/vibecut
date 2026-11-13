---
name: vibecut
description: Edit a VibeCut video project through its MCP server or vibecut-cli. Use when asked to cut, trim, caption, reframe, arrange or export video in VibeCut, or to check a preview frame of a project.
---

# VibeCut editing loop

VibeCut exposes one command registry over MCP (`vibecut-cli mcp`) and over the shell (`vibecut-cli exec`, `control`). Never guess a command name or parameter: read it from `command_list` or [references/commands.md](references/commands.md).

## Loop

1. **Open.** The MCP server is configured with one project (headless) or attached to the running app. Call `inspect` with `{"request_id":"r1","command":"inspect","params":{}}`. Keep `rev_after` as the current revision.
2. **Look up.** `command_list` pages the registry (`limit` 1-16, then `next_cursor`). Take parameters from the schema and example it returns.
3. **Edit.** Call `command_run` (or a curated tool) with `command`, `params`, `base_rev` = the current revision and a fresh `idempotency_key`. Replace the revision with the response's `rev_after`. Group related edits in one `batch`. Undo a wrong edit with `undo`.
4. **Preview.** Call `preview` with `{"frames":[<frame numbers>]}`. It starts a job: poll `job_status` with the returned job id until it completes. Frames arrive as inline images with frame and clip ids burned in. Look at them before continuing.
5. **Verify.** Call `inspect` again and compare with what you intended. A preview shows the picture; `inspect` shows structure, so use both.
6. **Export.** Call `export` with an absolute `path` inside the human-granted output directory and `overwrite: false`. Poll `job_status`; the result carries the file path. Call `project.save` in headless mode only.

Details, envelope fields and error handling: [references/edit-loop.md](references/edit-loop.md). Permissions and consent: [references/safety.md](references/safety.md).

## Rules

- Every edit needs the current `base_rev` and a unique `idempotency_key`. A `stale_revision` refusal means: run `inspect`, then redo the edit against the new state.
- After a timeout or lost reply, retry only the original request with the original key. A fresh key is a new edit.
- A job never edits the project itself. Apply its result with a command, using the job's `revision` as `base_rev`.
- You cannot read or write files outside the project. Source paths, new media, the project library and provider consent belong to the person; ask them instead of working around a `permission` refusal.
- Shell-only agents use `vibecut-cli --json exec <project> <command_id> '<json>'` and the exit codes in [references/edit-loop.md](references/edit-loop.md#shell-use).
