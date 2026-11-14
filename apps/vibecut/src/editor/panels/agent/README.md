# Agent chat

Interface: `State`, `Action`, `show`, `apply`, `Backend`, `Environment`, `AgentView`.

The chat docked right of the timeline, in the row below the properties, or expanded into the full-height right column (`State::is_expanded`, kept for the window; layout and measured
values: [Editor UI](../../../../../../docs/product/ui/editor.md#agent-chat)). It composes the
[agent crate](../../../../../../crates/agent/README.md) and holds no loop logic.

| Module | Responsibility |
|---|---|
| `state` | `State`: consent, the conversation, the running turn, the pending changes, Apply, Discard, Retry and the history. |
| `view` | Paints the title strip, the transcript, the consent sheet, the changes card and the input with its `@` list. |
| `turn` | Runs one turn on a worker thread and returns its events over a channel. |
| `transcript` | Folds the event stream into the transcript lines the panel shows and the history keeps. |
| `history` | Reads and writes one project's transcript under the app data folder. |
| `mention` | Candidates for `@`, the search, and the replacement of a picked label by its id. |
| `backend` | `Backend`: opens the chat model and answers whether a key exists. |

## Flow

Before the first message the panel shows the consent sheet: the service, its version, the data
that leaves the machine (the project summary always; clip file names, caption text and frames
only when switched on, all off by default), and a spend limit. Accept builds the
`AgentChatConsent` for exactly those and a conversation under it; a change of any of them asks
again (`Data and spend` above the input). Without a key for either service the panel says where
to set it (the `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` environment variable) and offers no
input. Declining sends nothing.

Send forks the live session on the UI thread (`Session::fork`) and moves the fork to a worker
that calls `agent::run_turn_on_fork`; the live session is never lent to the thread. Events come
back over a channel: streamed text, one line per tool step with its outcome, job progress. Stop
sets the turn's cancel flag. A finished turn that changed something ends in the changes card with
the counts of the change summary and four buttons:

- `Preview` shows the fork in the player and the timeline, read only: the frame hands the areas
  the fork's session instead of the live one, disables the media, properties and timeline areas
  and the shortcuts, and marks both surfaces.
- `Apply` is `Session::apply_fork`: one undo step. A stale revision keeps the card with `Retry`,
  which runs the same request again on a new fork.
- `Discard` drops the fork.

The input stays off while a card waits, because the next turn forks the live project.

`@` in the input searches the timeline items and media by name; a picked label is replaced by
the id the model reads in `timeline_read` and `project_summary` (`[item 12]`), so a name is
never sent unless the creator shared clip names.

History is the transcript, kept per project as `<app data>/agent_history/<project id>.json`
(version 1, at most 400 lines, 512 KiB) and cleared with the Clear button. The model's own
context is not restored: a new conversation starts after a restart and asks for consent again.

Tests lend `Backend` a scripted model (`ai` with `test-chat`), so a turn runs without a network.
