# Agent chat stream fixtures

Offline only; never replayed against the network.

- `anthropic/*.sse` follow the Messages streaming wire shape (`message_start`, `content_block_*`, `input_json_delta`, `message_delta`, `message_stop`, `ping`) and `openai/*.sse` the Responses streaming shape (`response.output_item.*`, `response.output_text.delta`, `response.function_call_arguments.*`, `response.completed`), both as documented on 2026-11-12. They are authored from the docs, not recorded: no request id, header or credential, and the clip text is invented.
- `text` is a two-delta reply, `tool` a reply with a thinking or reasoning item, text and one tool call (`get_clip`, `{"clip":"c1"}`), `parallel` two tool calls and `malformed` a tool call whose arguments stop mid-object. Usage is 100 to 120 input tokens and 10 to 50 output tokens, so the ledger rows read exact micro-USD at the $2 / $10 per million-token rate.
- Error statuses and broken streams are synthesized inline in the tests.
- `frame.jpg` is the agent evaluation fixture's frame as `preview_frame` shows it: solid red (two red overlay clips) at 640x360, encoded by FFmpeg (`color=c=0xFF0000:s=640x360`, `-q:v 4`). Only the ignored live rows send it.

Refresh from a consented live run when either stream schema changes.
