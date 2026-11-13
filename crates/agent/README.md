# agent

## Purpose

Runs the in-app agent's turns: it forks a session, drives a streamed tool-use conversation whose tool calls run on the fork, and ends with the fork and a change summary for the caller to apply or discard.

Engine owns forks, the command registry, control admission and jobs; `ai` owns the provider adapters, the conversation consent and the spend ledger. This crate owns the turn loop, the tool set generated from the registry, the agent command policy, the event stream and the evaluation set. It has no UI, argument parsing, project persistence or process lifetime: the desktop chat panel and `vibecut-cli agent` compose it. [Agent](../../docs/product/agent-control/agent.md) owns the tools, refusals, limits and events as the user and the model see them.

## Module map

| Module | Responsibility | Contract |
|---|---|---|
| `cancel.rs` | Carries the user's cancel request into a running turn. | This page |
| `error.rs` | States why a turn could not start. | This page |
| `eval.rs` | Scores the agent against a fixed list of prompts with the changes each should make. | [Evaluation](../../docs/product/agent-control/agent.md#evaluation) |
| `event.rs` | Defines the event stream a turn yields, with the typed reasons it ends early. | [Events](../../docs/product/agent-control/agent.md#events) |
| `policy.rs` | Decides which registry commands the agent caller may run. | [Refusals](../../docs/product/agent-control/agent.md#refusals) |
| `provider.rs` | Opens the chat adapter of the provider the user chose. | This page |
| `reads.rs` | Reads bounded views of the fork's project for the model. | [Tools](../../docs/product/agent-control/agent.md#tools) |
| `tools.rs` | Generates the model's tools from the registry and runs each call against the fork. | [Tools](../../docs/product/agent-control/agent.md#tools) |
| `turn.rs` | Runs one turn of the tool loop over a fork of the live session. | [A turn works on a copy](../../docs/product/agent-control/agent.md#a-turn-works-on-a-copy) |

## Entry points

`new_conversation(model, consent, limits)` builds the `ai` conversation with the agent's system text and `tool_specs()`; it refuses without a consent for exactly that provider and model. `run_turn(TurnRequest, events)` forks the live session, sends the prompt, runs tool calls until the model answers without any, a limit is reached or `CancelFlag::cancel` is called, and returns a `Turn` holding the fork, its `TurnEnd` and the refusal count. Every `AgentEvent` goes to the `events` closure in order and the last is `TurnDone` or `TurnError`. The caller applies with `Session::apply_fork(turn.fork)` or drops the fork.

`AgentPolicy` is the `Caller` policy: `TABLE` marks every registry command id `Allow`, `Frames` or `Refuse(Reason)`, and `verdict` reads a row. `Provider` and `open_model` build the Anthropic or OpenAI adapter from a validated key. `CASES`, `fixture` and `evaluate` are the evaluation set. `Error` states a refusal before the first request.

## Limits

`TurnLimits::max_steps` bounds the model requests of a turn (1 to 64, default 12). Output tokens, context bytes and spend belong to the conversation; a tool result is at most 32 KiB; a page of `timeline_read` or `transcript_read` is at most 50 entries and a `command_list` page at most 8. Ending a turn waits at most two seconds for cancelled jobs to settle.

## Tests

`cargo test -p agent` runs the contracts over the public interface with `ai::text::conversation::ScriptedChat` (the `ai` `test-chat` feature, enabled only as a dev-dependency), a deterministic model that plays a fixed script and can build a reply from the previous tool results: the policy table against the registry (a command without a row fails), a multi-step edit that reads its own result, refused commands and batch children, malformed and extra-field calls, a job on the fork, step and spend limits, cancel before a request, during a tool and during a job, a stale apply, withheld data kinds and the evaluation set scored with a correct and an idle model.
