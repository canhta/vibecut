# Verification

## Checks

Commit only green; choose the smallest covering check. Rust compilations follow the [build admission policy](../agents/orchestration.md#tickets-and-placement); provision native dependencies with `cargo xtask ffmpeg` once per machine before its first local Rust build.

| Change/stage | Check |
|---|---|
| Markdown, tickets, assets only | `cargo xtask docs` (layers, assets/sidecars, i18n catalog audit, name gate) |
| One Rust crate, iteration | `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets` |
| App UI strings | `cargo xtask i18n` (en/vi completeness, unused keys, direct egui literals) |
| Code handoff, frozen SHA | `cargo xtask ci --require-oracles` once (fmt, Clippy, all tests/doctests, layers, assets, names, required oracles) |
| Media/render/hot path | Above plus the ticket benchmark; report speed and output quality (VMAF/SSIM, loudness/peaks, pixel diff or tick drift) against its baseline |
| Fix round after `changes` | Touched crates' tests and Clippy plus the reviewer's reproductions; no full CI |
| Merge of a tree without a full-CI receipt (fix round, reviewer fix, Rust conflict resolution) | The merger runs one full local CI on the final tree before push |
| T1 merge | Smallest covering check on the exact rebased SHA before push |
| Clean rebase | Reuse the receipt; no full CI rerun |

The build wrapper leaves a CI receipt and reuses it for the same tree. `cargo xtask ci [--require-oracles]` is the aggregate entry: `checks` (fmt/layers/assets/names), `clippy`, workspace tests/doctests. Hosted execution/triggers belong to [workflows](workflows.md#hosted-ci).

Required mode rejects missing, unusable or pre-6.0 external ffmpeg/ffprobe, sets `VIBECUT_REQUIRE_ORACLES=1` so skips fail, and prints tool paths/versions plus fixture size/SHA-256 receipts. Fast/offline CI labels skipped oracles; skips never count as acceptance. The name gate rejects local `.scratch/forbidden-names.txt` regexes in tracked files and branch commit messages.

## AI model integrations

- Third-party ASR/translation/TTS/LLM adapters: verify contracts, consent, failure paths and one bounded live smoke per provider with real output. Provider accuracy changes with user-selected models; eval corpora, WER/MOS and quality gates apply only to our logic below.
- Our clip selection/boundary/reframe prompts, heuristics and ranking: use a fixed labelled set, metric and baseline; report every change against it through `cargo xtask clip-eval <set>` ([contract](../../crates/clip-eval/README.md)).
- Live smokes/evals prefer OpenAI; pure text-in/text-out LLM calls use only `gpt-6-luna`. Other services may use any admitted provider. Live provider tests use synthetic input, bounded cost, `VIBECUT_LIVE_AI=1` plus keys, and `#[ignore]` outside CI.

## When to write tests

Add a test only for a plausible bug without existing coverage: changed behaviour at its public interface, or uncovered hostile/boundary inputs, cancellation and failure. For a bug/crash, add its reproduction to the existing behaviour's table. Keep one behaviour per test, prefer new table rows over new tests, and delete superseded cases. Name tests and files after behaviour, e.g. `trim_refuses_past_source_end`.

Compiler/type guarantees, getters/constructors, plumbing, Debug/Display, third-party behaviour, private helpers, repeated coverage at another layer, unchanged refactors and docs/tickets need no test. Test decisions where owned and only caller-specific decisions in callers. Redundant, duplicate or implementation-coupled tests are review `changes`, like missing coverage.

Integration tests use one binary per package, rooted at `tests/it/main.rs`; behaviour
files and shared helpers are modules beneath `tests/it/`. `cargo xtask checks`
rejects other integration-test entry points, including explicit Cargo targets.
Run a module with `cargo test -p <crate> --test it <module>::`.

## Test selection

| Change | Evidence |
|---|---|
| Parser/validation/pure function | Unit tables: valid, boundary, hostile, truncated; no panic |
| Time/mapping/edit invariants | Properties with retained seeds: rational rates, boundaries, random commands, undo/redo |
| Decode/mux/export/A/V sync | ffmpeg/ffprobe oracle: stream parameters, frame/sample counts, stated tolerances |
| Text/compositor/subtitle overlay | Deterministic image goldens with Vietnamese combining marks and clipping |
| Engine/session/jobs | Command integration: refused states, atomicity, undo/redo, cancellation, failure |
| Desktop interaction | egui_kittest through real commands and stable [automation IDs](../product/ui.md#principles); assert engine and widget state, not pixels |
| Provider adapters | Offline schema/error/retry/consent fixtures plus separate live smoke |

Inject codec/provider adapters at public interfaces. Order concurrent tests with controlled gates/channels. Inject OS boundaries at their seam: picker results or `RawInput.dropped_files` enter the same app path; native dialogs/Finder/osascript/coordinate clicks are not test tools.

UI reports attach `cargo xtask ui-audit` offscreen named scenes at 1x/2x for changed screens; this separate design audit runs outside local/hosted CI. Real-app checks use the control socket (p2-platform ticket 02), background launch without activation or a visible window. The maintainer grants the screen for one native dialog smoke per milestone.

## Fixtures

Generate synthetic media with ffmpeg into `target/fixtures/<area>/`; record generator command/tool version and apply oracle admission above. Goldens fix canvas, rate, fonts and CPU rendering; put actual/diff images under `target/`. Re-bless only after visual inspection with a reason.

## Evidence

Label each claim observed, source-documented, inferred, proposed or unverified, with command, revision and result; a green build proves only the build. List skips and reasons. Benchmarks record hardware/OS, profile, revision, input, repetitions, cold/warm, measured elapsed time, throughput and peak RSS.

## Reports

Keep reports local and hand over paths. Open with `## TL;DR`, at most 10 lines covering outcome, SHA/blocker, decisions and follow-ups.

| Report | Required contents |
|---|---|
| Done | Ticket, branch, SHAs, diff stat; every Must and Verify item → the test or command that exercises it → result; dependency lookups; observed/unverified behaviour; follow-ups and remaining work |
| Review | Ticket, SHA, scope, checks; findings with critical/high/medium/low severity, `file:line`, reproduction, impact, fix; verdict `approve` or `changes` |

Correctness, security and data-loss findings block approval.
