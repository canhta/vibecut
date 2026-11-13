# Edit loop details

## Request envelope

Every `command_run`, curated tool and `batch` call takes:

| Field | Meaning |
|---|---|
| `request_id` | Your label for the call, echoed back |
| `command` | Registry id such as `project.rename` (fixed by a curated tool) |
| `params` | The command's parameters |
| `base_rev` | Revision the edit builds on; required for effectful commands |
| `idempotency_key` | 1-128 bytes, unique per intended edit; required for effectful commands |

Read-only commands (`inspect`, `command_list`, `job_status`) need neither `base_rev` nor a key. `api_version`, `registry_hash` and `session_id` are filled in for you.

## Reading a response

`structuredContent` is the engine response. Success has `value.status` = `result`. `rev_after` is the revision to use next. A refusal sets `isError` and keeps the engine code in `value.value`: `stale_revision`, `idempotency_conflict`, `permission`, `refused`, `unsupported`, `session_gone`, `limit`, `receipt_capacity`. Adapter errors use `error.code` (`queue_full`, `timeout_unknown_outcome`, `cancelled_unknown_outcome`, `session_gone_unknown_outcome`).

| Code | Do |
|---|---|
| `stale_revision` | `inspect`, rebuild the edit on the new revision, new key |
| `idempotency_conflict` | You reused a key for different parameters; use a new key |
| `permission` | The person must grant it; do not retry |
| `*_unknown_outcome` | Retry the same request with the same key |
| `queue_full`, `limit` | Wait, then retry with the same key; shrink the request for `limit` |

## Jobs

`preview`, `export`, `media.*` analysis, `captions.generate`, `clips.suggest` and similar commands start a job and return `job` and `revision`. Poll `job_status {"job": <id>}` until `snapshot.state` is `completed`, `failed` or `cancelled`; stop one with `job_cancel`. Finished data is in `snapshot.output`. Accepted jobs survive an attach reconnect.

## Previews

At most 4 frames per call, JPEG, longest edge 640 px. Frame number and clip ids are burned into the picture. Ask for the frames where the edit lands (cut points, first and last frame of a clip), not an even sweep.

## Shell use

`vibecut-cli --json exec <project> <command_id> '<json>'` runs one command under the project writer lease; `vibecut-cli control <project>` takes newline-delimited JSON requests with the same envelope. The project allows one writer, so close its editor window or work on a copy.

| Exit | Meaning |
|---|---|
| 0 | Success |
| 2 | Bad arguments, JSON or parameters |
| 3 | Project busy or changed on disk; nothing written |
| 4 | Required media, model or credential unavailable |
| 5 | Rejected by validation, or consent refused |
| 6 | Media or render failure |
| 7 | I/O or job failure |

With `--json`, an error is one JSON object on the last stderr line.
