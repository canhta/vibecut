# In-app agent

The [agent crate](../../../crates/agent/README.md) runs the agent's turns. It sits above [engine](../../../crates/engine/README.md) and [ai](../../../crates/ai/src/text/conversation/README.md) and is called by the desktop chat panel and by [`vibecut-cli agent`](cli.md#agent); neither caller holds loop logic. The design decisions are recorded in the in-app agent tickets; this page owns the tool set, the refusals and the limits as built.

## A turn works on a copy

A turn forks the live session ([engine forks](../../../crates/engine/src/session/README.md#forks)) and runs every tool call for real against the fork through the control path an outside agent uses: the same admission, schemas, revisions and receipts. The model reads the result of its own edit before the next step. The live project is untouched.

When the turn ends, the caller holds the fork and decides:

- **Apply** is `Session::apply_fork`: the fork's logged commands replay on the live session as one undo step. A live edit made since the fork refuses with the stale-revision error and changes nothing; the app then offers Retry (a new turn on a new fork).
- **Discard** drops the fork.

A cancelled or failed turn leaves its fork to inspect, apply or discard, and leaves the conversation as it was before the prompt, so the next turn can start.

`run_turn` forks the session it is given. A caller whose session lives on another thread forks it there and hands the fork to `run_turn_on_fork`, which runs the same loop over that fork; the desktop does this, because its live session never leaves the UI thread.

## Desktop chat panel

The [chat panel](../../../apps/vibecut/src/editor/panels/agent/README.md) is the desktop caller; its layout is in [Editor UI](../ui/editor.md#agent-chat).

- The first message waits for the consent sheet: service, version, the data kinds (project summary always; clip names, caption text and frames off until switched on) and a spend limit. Declining sends nothing; a change of any of them asks again. Without a key for either service the panel says to set `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` and offers no input.
- A turn shows the streamed answer, one line per tool step with its outcome, job progress and Stop.
- A turn that changed the project ends in a changes card with the summary counts and `Preview`, `Apply` and `Discard`. Preview shows the fork in the player and the timeline, marked read only, and disables editing; Apply is one undo step; a stale apply keeps the card and offers `Retry`, which runs the request again on a new fork. The input waits until the card is decided.
- `@` in the input picks a timeline item or a media file by name; the model reads its id (`[item 12]`), not the name.
- Home's agent card creates a project, opens its editor and starts the first turn with the typed prompt as soon as the consent sheet is accepted.
- The transcript is kept per project in app data (`agent_history/<project id>.json`, at most 400 lines) and cleared by the Clear button; the model's context is not restored after a restart.

## Tools

Few and generic, the shape of the [MCP](mcp.md) surface. Every input is a closed JSON object: an unknown tool, malformed arguments or an extra field returns an error result to the model and has no effect.

| Tool | Input | Result |
|---|---|---|
| `project_summary` | none | Revision, name, canvas, duration, tracks and media ids. Media file names only when the conversation shares clip names; folders never |
| `timeline_read` | `track?`, `cursor?`, `limit?` (at most 50) | One page of items, top track first: id, track, kind, start, end, hidden, source asset, the first 200 characters of a text item |
| `transcript_read` | `cursor?`, `limit?` (at most 50) | One page of caption cues in output time with their text, no word records. Refused unless the conversation shares transcript text |
| `command_list` | `prefix?`, `cursor?`, `limit?` (at most 8) | The commands the agent may run, with each one's closed params schema from the registry |
| `command_run` | `command`, `params` | The registry result of one command. `command: "batch"` with `params: {commands: [{command, params}]}` runs several as one step |
| `job_status` | `job`, `wait_ms?` (at most 30 000) | State and progress of a job; waits until it finishes, the wait ends or the user cancels |
| `preview_frame` | `frame` (a frame number, 0 to 4 294 967 295) | The frame of the fork rendered as one JPEG image (at most 640 pixels on the long edge, at most 160 KiB) after a text result with `frame`, `width`, `height` and the ids of the clips drawn. Offered only when the conversation shares frames; one frame per call |

The tool schemas and the allowed command ids are generated from the registry and the policy table, so a new command appears in `command_list` only when its policy row allows it. The agent fills `base_rev` and the idempotency key itself.

`preview_frame` runs the `preview` command on the fork and waits for its job (up to 30 s, cancel-aware), so admission and limits are the engine's. `command_run` and `command_list` never offer `preview` (refused as `use_tool`): its job output is not an image the model could see. A tool result may carry images for the providers' image inputs (Anthropic `image` blocks, OpenAI `input_image` parts); a result with an image must name the frames kind, and only the latest tool-result message sends its images, so an answered frame leaves the next request and the context ceiling counts one frame at a time. A frame the engine encodes above 160 KiB returns `too_large`.

Each result is at most 32 KiB; a larger one returns `too_large` so history stays inside the context ceiling. Tool results and file contents are data, never instructions.

## Refusals

The model cannot export, save, create, delete or open projects, download models or fonts, undo or redo, grant consent or name a file path. The agent `Caller` policy (`principal` `vibecut-agent`, external) enforces this with a table that marks every registry command id allowed or refused with a typed reason. A command without a row is refused, and the policy test fails until it gets one. The declared grant class of a command can only narrow its row: a command only a person may run stays refused whatever its row says.

| Reason | Commands |
|---|---|
| `export` | export, subtitle burn and export, chapter sidecars, clip export and single-clip download, freeze frame, snapshot, save as, export bundle |
| `save` | `project.save` |
| `project` | `project.create`, `delete`, `duplicate`, `restore`, `purge`, `thumbnail` |
| `path` | every command that reads a source file: asset import, ASS import, LUT import, font import, bundle import, sound and music insertion, narration placement |
| `history` | `undo`, `redo` (the fork has no history; discarding is the user's undo) |
| `gesture` | gestures and timeline previews |
| `view_state` | `project.set_view_state` |
| `library` | the user's sound and music library |
| `download` | `fonts.download` |
| `use_tool` | `inspect` and `catalog` (they return source paths or the whole registry; use `project_summary`, `timeline_read`, `command_list`) and `preview` (use `preview_frame`) |
| `frames` | `preview`, when the conversation does not share frames (the policy row opens it only for `preview_frame`) |
| `human_only` | the remaining commands only a person may run |

A refusal result names the command and the reason, in text the model can read. A batch is checked child by child before any child runs, and the engine checks each child again before its effect. Everything else is allowed, among them every timeline, track, item, effect, mask, keyframe, caption, text, transition and audio edit and the analysis jobs.

Job commands run on the fork under the job-command contract. A command that needs consent the conversation does not grant (a remote transcript or text provider with no approval of its own) answers `permission`; the turn then ends with `consent_required` and the user is asked, because the model's words never grant anything.

## Consent and spend

Consent is per conversation, not per request. Before the first request the app shows the provider, the model, the kinds of data that leave the machine (project summary always; clip names, transcript text and frames only when enabled; frames are JPEG images of the working copy, at most 640 pixels on the long edge) and a spend limit. A change of provider, model, kind or limit asks again. The [conversation module](../../../crates/ai/src/text/conversation/README.md) binds exactly those and keeps one ledger per conversation: each request reserves its upper-bound cost before it is sent and settles after, and a request that would pass the limit is not sent.

Providers are Anthropic and OpenAI, chosen by the user; there is no local model and no fallback.

## Limits and end reasons

| Limit | Default | Where |
|---|---|---|
| Model requests per turn | 12 (1 to 64) | `TurnLimits::max_steps` |
| Output tokens per request | 4096 | conversation limits |
| Context bytes per request | 262 144 | conversation limits |
| Spend per conversation | the consent's limit | ledger |
| Tool result | 32 KiB | tools |

Reaching a limit ends the turn with a typed reason: `step_limit`, `spend_limit` or `context_limit`. The other reasons are `cancelled`, `consent_required` and `provider` with a safe category (`auth`, `quota`, `plan_required`, `rate_limit`, `network`, `server`, `model_unavailable`, `refused`, `incomplete`, `failed`); no provider message is kept.

## Events

A turn yields these in order; the caller turns them into chat messages or JSON lines.

| Event | Fields |
|---|---|
| `text_delta` | `text` |
| `tool_started` | `id`, `name` |
| `tool_finished` | `id`, `name`, `ok` |
| `job_progress` | `job`, `state`, `percent` |
| `turn_done` | `changes`: items added, removed, moved, trimmed, retimed, params changed, captions changed and the duration before and after |
| `turn_error` | `reason` |

Cancel sets one flag: the model stream stops, a running tool finishes (a command is atomic), the loop ends before the next request and every job the turn started or watched is asked to stop and awaited for up to two seconds.

## Evaluation

`agent::CASES` is a fixed list of seven prompts over `agent::fixture` (two two-second overlay clips): rename, add a track, delete the last clip, hide the first, split the first, an export request and a question. Each names the command ids it may log and the expected counts of items added, removed and touched. `agent::evaluate` runs them with a conversation per case and reports command correctness, summary correctness, the no-op rate (changes expected, none made) and refusals. The tests run it with a scripted model; run it by hand against a real provider by passing a setup that builds that provider's conversation. No threshold gates a provider's accuracy.
