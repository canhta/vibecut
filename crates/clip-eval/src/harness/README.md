# Entry points

`harness/suggestion` calls one provider and snaps the clips it proposes to source boundaries. `harness/report` owns the persisted evaluation report and its Markdown rendering.

```sh
cargo xtask clip-eval synthetic
cargo xtask clip-eval <set> --labels DIR --provider heuristic --k 5
cargo xtask clip-eval <set> --transcribe --max-cost-usd 1
cargo xtask clip-eval <set> --provider gpt-6-luna --live --max-cost-usd 1
cargo xtask clip-eval <set> --write-baseline PATH --baseline-revision MAIN_SHA
cargo xtask clip-eval <set> --baseline PATH
```

`xtask` launches the `clip-eval` binary with the workspace FFmpeg environment.
`<set>` resolves to `crates/clip-eval/fixtures/synthetic` for `synthetic`, an
existing directory path, or `.privates/clip-eval/<set>` (falling back to the main
checkout's `.privates` from a linked worktree). Label files are `*.json` in that
directory; `*.transcript.json` and `baseline.json` are inputs, not labels.

Live LLM calls require `--live`, `OPENAI_API_KEY`, and stay within
`--max-cost-usd` (default USD 1). One cumulative budget covers ASR and suggestion
reservations across every video; each remote invocation is reserved before its
first upload and the reservation stays spent if it later fails. The report prints
the per-video estimate and remaining budget before the call. Pure LLM calls use
only `gpt-6-luna`. The default provider is the free local heuristic, so the
synthetic set runs offline.

`--transcribe` is the opt-in live ASR step for a label without a transcript. It
uploads bounded 15-minute mono PCM chunks to the admitted ElevenLabs Scribe v2
adapter, assembles absolute source coordinates, and caches the resulting
transcript. It requires `ELEVENLABS_API_KEY` and the same cumulative budget. ASR
is not an LLM call, so the Luna-only rule does not apply.

