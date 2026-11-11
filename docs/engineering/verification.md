# Verification

## Checks

Commit only green, with the smallest check that covers the change. Compile through `cargo xtask build -- <command>` ([builds](../agents/orchestration.md#placement-and-builds)); run `cargo xtask ffmpeg` once per machine first.

| Change | Check |
|---|---|
| Markdown, tickets, assets | `cargo xtask docs` |
| UI strings | `cargo xtask i18n` |
| `JsonSchema` metadata, including Rust doc comments on derived input types | `cargo xtask catalog`, then the existing `control::catalog::checked_in_registry_digest_and_command_reference_match_the_registry` test |
| Rust, while iterating | Focused named behaviour tests in the affected crate; use a narrow compile check only when needed to diagnose compilation |
| Rust, handoff | Once on final source, `cargo test -p <crate>` for each behaviour-edited crate, plus `cargo xtask checks`. Enumerated mechanical contract callers get the narrow compile check needed for atomic correctness; coordinator owns the final public-signature gate. No workspace build, Clippy sweep or `cargo xtask ci`, unless the maintainer asks for one |
| Media, render, hot path | The above plus the ticket benchmark against its baseline (speed and VMAF/SSIM, loudness, pixel diff or tick drift) |
| Fix round, merge conflict | Focused reproduction and affected edited-crate final suite for changed source; retain unaffected receipts |
| Rebase without conflict | Nothing is rerun; merge |
| Merge of a change to a public type or signature | `cargo xtask build -- cargo check --workspace --all-targets` once on the rebased branch, by the coordinator |
| Platform-specific code (OS `cfg`, FFI, file-system, build scripts) | A receipt from each affected OS with a build host; list others as unverified |
| `main`, every merge | Hosted CI runs the full workspace and reports there. Agents do not watch it; the maintainer asks for a fix when a run is red |

`cargo xtask ci [--require-oracles]` runs `checks`, `clippy` and workspace tests, and refuses inside a linked worktree unless `CI` is set (hosted runners set it), naming the local commands instead. Required oracle mode fails on a missing or pre-6.0 `ffmpeg`/`ffprobe` and on any skipped oracle test; skips never count as acceptance.

Reuse receipts for identical source and a compatible toolchain, configuration, environment and oracle set; cite their exact revision, command, named nonzero tests and result. Review is inspection of the changed diff and evidence, with reruns for an actual gap, contradiction or changed behaviour. A conflict-free rebase alone does not invalidate a receipt. A necessary prerequisite failure gets its narrow fix and failed-test rerun, not an unrelated suite sweep. Document checks need no test count.

Include behavioural consumers and test adapters in a contract-change census: a compile check cannot prove changed error, fallback or cancellation semantics. Run their affected named cases even when the caller types still compile.

## AI model integrations

- Provider adapters: test contracts, consent and failure paths offline, plus one bounded live smoke per provider (`VIBECUT_LIVE_AI=1`, keys, synthetic input, `#[ignore]`). Provider accuracy is not ours to gate.
- Our clip selection, boundaries, reframe and ranking: report every change against the labelled set with `cargo xtask clip-eval <set>` ([contract](../../crates/clip-eval/README.md)).
- Live LLM text calls use `gpt-6-luna`.

## When to write tests

Test only a plausible bug without coverage: changed public behaviour, hostile or boundary input, cancellation, failure. A bug's reproduction becomes a row in the existing behaviour's table. One behaviour per test, named after it (`trim_refuses_past_source_end`); prefer table rows to new tests; delete superseded cases.

No tests for compiler guarantees, getters, plumbing, Debug/Display, third-party behaviour, private helpers, refactors or docs. Redundant or implementation-coupled tests are review `changes`.

Each package has one integration binary at `tests/it/main.rs` with modules beneath it; run one with `cargo test -p <crate> --test it <module>::`.

## Test selection

| Change | Evidence |
|---|---|
| Parser, validation, pure function | Unit tables: valid, boundary, hostile, truncated |
| Time, mapping, edit invariants | Property tests with retained seeds |
| Decode, mux, export, A/V sync | ffmpeg/ffprobe oracle with stated tolerances |
| Text, compositor, subtitle overlay | Image goldens with Vietnamese combining marks |
| Engine, session, jobs | Command integration: refusal, atomicity, undo/redo, cancellation |
| Desktop interaction | egui_kittest through real commands and [automation IDs](../product/ui/index.md#principles); assert state, not pixels |
| Provider adapters | Offline fixtures plus a separate live smoke |

Inject codecs, providers and OS boundaries (file pickers, dropped files) at their seams; native dialogs and coordinate clicks are not test tools. Order concurrent tests with gates or channels.

## UI evidence

- Attach `cargo xtask ui-audit` scenes (1x and 2x) for each changed screen, and a side-by-side with the cited reference shot at the same logical size. Fix every difference or list it with its cause. Values come from measured shots or the reference's source, never invented.
- `cargo xtask ui-compare OURS.png REFERENCE.png [--region X,Y,W,H] [--out DIR]` scores edge-structure error (whole region default limit 0.02, worst 32 px tile 0.15). Inputs must share the logical size (references are 1727×1029; halve 2x captures first). Report the whole-region error before and after for each changed region; it must fall. The side-by-side, not the score, judges layout.
- Real-app checks attach in the background ([MCP control](../product/agent-control/mcp.md#start-and-attach)); one native dialog smoke per milestone, with the maintainer's OK.

Comparison validity and product acceptance are separate gates. Verify matching logical size, crop/origin, scale, state and compared region before interpreting a metric; invalid inputs are non-comparable, never a pass. Correct inputs and rerun only the comparison, retaining the original artifacts and result. Valid results still obey the thresholds and before/after criterion above; a design-peer decision or acceptance change must be explicit, not inferred from an invalid capture or an inspected side-by-side.

## Fixtures

Prepare the suite's executable, native-runtime and oracle prerequisites in the intended target/profile before running it. Reuse compatible verified artifacts; a missing prerequisite is failed setup, never a passing skip. Diagnose it before another suite run.

Generate synthetic media with `testkit::oracle::generate` into `target/fixtures/`. Goldens fix canvas, rate, fonts and CPU rendering. Re-bless only after looking at the image, with a reason.

## Evidence

Label each claim observed, source-documented, inferred or unverified, with command, revision and result. List skips. Benchmarks record hardware, OS, profile, revision, input, repetitions, cold/warm, time, throughput and peak RSS.

## Reports

Write reports beside the work and transfer handback reports with their linked evidence to the master's local archive under the [tracker's custody rules](../agents/issue-tracker.md). Remote paths alone do not establish durable custody. Open with `## TL;DR` (at most 10 lines: outcome, SHA or blocker, decisions, follow-ups).

Size the report to the decision and its risks. A fix receipt contains the correction and affected evidence with links to retained proof; an admission report contains only unresolved decisions and the proposed working boundary. Keep raw logs and inventories as linked artifacts. A passing review needs only its revision, scope, verdict and material limitations; actionable findings travel immediately under the [review process](../agents/orchestration.md#review-and-merge).

- Done: compact ticket/branch/exact-SHA receipt, named behaviour and failure evidence mapped to admitted Must/Verify items, and remaining gates. Link retained exact-source receipts instead of restating or rerunning them; include dependency lookups only when relevant. Attach detailed raw, benchmark or native evidence when the change needs it; preserve failures, skips and platform limitations.
- Review: ticket, SHA, findings with severity, `file:line`, reproduction and fix; verdict `approve` or `changes`. Correctness, security and data-loss findings block approval.
