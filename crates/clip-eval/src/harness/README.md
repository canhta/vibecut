# Entry points

`harness/suggestion` calls one provider and snaps the clips it proposes to source boundaries. `harness/report` owns the persisted evaluation report and its Markdown rendering.

```sh
cargo xtask clip-eval synthetic
cargo xtask clip-eval <set> --labels DIR --provider heuristic --k 5
cargo xtask clip-eval <set> --transcribe --max-cost-usd 1
cargo xtask clip-eval <set> --provider openai --model <id> --live --max-cost-usd 1
cargo xtask clip-eval <set> --write-baseline PATH --baseline-revision MAIN_SHA
cargo xtask clip-eval <set> --baseline PATH
```

`xtask` launches the `clip-eval` binary with the workspace FFmpeg environment. `<set>` resolves to `crates/clip-eval/fixtures/synthetic` for `synthetic`, an existing directory path, or `.privates/clip-eval/<set>` (falling back to the main checkout's `.privates` from a linked worktree). Label files are `*.json` in that directory; `*.transcript.json` and `baseline.json` are inputs, not labels.

Live LLM calls require `--live` and `OPENAI_API_KEY`, and stay within `--max-cost-usd` (default USD 1). One cumulative budget covers ASR and suggestion reservations across every video. Each remote invocation is reserved before its first upload and stays spent if it later fails. The report prints the per-video estimate and remaining budget before the call. The selected suggestion model comes from `--model` or the shipped default. The default provider is the free local heuristic, so the synthetic set runs offline.

`--transcribe` is the opt-in live ASR step for a label without a transcript. It uploads bounded 15-minute mono PCM chunks to the ElevenLabs Scribe v2 adapter, assembles absolute source coordinates and caches the transcript. It requires `ELEVENLABS_API_KEY` and the same cumulative budget. The selected ASR model comes from `--asr-model` or the shipped default.

Provider identity is `heuristic` or `openai`; `--model ID` selects the suggestion
model as bounded text, and `--asr-model ID` selects opt-in ElevenLabs ASR.
Defaults come from AI's table. `--accept-unknown-price` explicitly authorizes the
selected model for each requested job kind; without it unknown pricing refuses
before sending. The cumulative money budget reserves known quotes only. Report
schema 2 represents unknown money as null and renders it as unknown; it never
reports an unknown invocation as free. ASR cache identity includes a hash of the
selected model, so model IDs cannot become path components or share results.
