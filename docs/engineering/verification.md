# Verification

## Checks

Commit only green; choose the smallest covering check. Rust compilations follow the [build admission policy](../agents/orchestration.md#placement-and-builds); provision native dependencies with `cargo xtask ffmpeg` once per machine before its first local Rust build.

| Change/stage | Check |
|---|---|
| Markdown, tickets, assets only | `cargo xtask docs` (layers, assets/sidecars, i18n catalog audit, name gate, docs index, README module maps, tracker-free Markdown) |
| One Rust crate, iteration | `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets` |
| App UI strings | `cargo xtask i18n` (en/vi completeness, unused keys, direct egui literals, ticket ids and engineer words in shown text) |
| Code handoff, frozen SHA | `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets` for every touched crate and every crate that depends on a changed public interface, plus `cargo xtask checks`; touched media/render/export code runs its oracle tests in required mode |
| Media/render/hot path | Above plus the ticket benchmark; report speed and output quality (VMAF/SSIM, loudness/peaks, pixel diff or tick drift) against its baseline |
| Fix round after `changes` | Touched crates' tests and Clippy plus the reviewer's reproductions; no full CI |
| Batch integration on `main` | The coordinator runs `cargo xtask ci --require-oracles` once per batch of merges (at least daily while work lands) on `main`, then fixes or reverts what fails before the next batch; a red batch blocks new merges until fixed |
| Rust conflict resolution during a merge | The merger reruns the touched crates' tests and Clippy on the resolved tree |
| T1 merge | Smallest covering check on the exact rebased SHA before push |
| Clean rebase | Reuse the receipt; no full CI rerun |
| Platform scope of a receipt | A full-CI receipt from any supported OS counts; a change touching platform-specific code (`cfg` for an OS, native/FFI, file-system paths and locking, build or provisioning scripts) also needs a receipt from each affected OS that has a local build host; an affected OS without one is listed unverified in the report and left to hosted CI on `main` |

The build wrapper leaves a CI receipt and reuses it for the same tree. `cargo xtask ci [--require-oracles]` is the aggregate entry: `checks` (fmt/layers/assets/names/modules/docs), `clippy`, workspace tests/doctests. `cargo xtask modules` enforces the [structure](structure.md#enforcement) rules a script can check; `cargo xtask docs` also enforces that `docs/README.md` links every docs page, each crate and app README maps its folder modules, and tracked Markdown carries no tracker ids (only-shrinking `xtask/docs-baseline.txt`). Hosted execution/triggers belong to [workflows](workflows.md#hosted-ci).

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
| Desktop interaction | egui_kittest through real commands and stable [automation IDs](../product/ui/index.md#principles); assert engine and widget state, not pixels |
| Provider adapters | Offline schema/error/retry/consent fixtures plus separate live smoke |

Inject codec/provider adapters at public interfaces. Order concurrent tests with controlled gates/channels. Inject OS boundaries at their seam: picker results or `RawInput.dropped_files` enter the same app path; native dialogs/Finder/osascript/coordinate clicks are not test tools.

UI reports attach `cargo xtask ui-audit` offscreen named scenes at 1x/2x for changed screens; this separate design audit runs outside local/hosted CI. Each changed screen also gets a side-by-side image: our scene next to the cited reference shot at the same logical size, with every layout, spacing, type, colour and icon difference either fixed or listed with its reason. Values come from a measured reference shot or the implementation reference's source (its component classes and constants), never from defaults or invention. A reviewer returns `changes` for a UI change without the side-by-side, or with an unlisted difference. Real-app checks use the background attach route ([MCP control](../product/agent-control/mcp.md#start-and-attach)), never activation or a visible window. The maintainer grants the screen for one native dialog smoke per milestone.

`cargo xtask ui-compare OURS.png REFERENCE.png [--region X,Y,W,H] [--out DIR] [--max-error F]` scores structural agreement against a reference capture. It compares luminance after per-image contrast normalisation through Sobel edge maps (strength saturated with `tanh`, so a recolour that changes edge contrast but not edge position scores near zero) blurred with a 2 px box radius, then reports whole-region and 32 px per-tile mean absolute edge error. The two scales differ by an order of magnitude, so they have separate limits: `--max-error` (whole region, default 0.02) and `--max-tile-error` (worst tile, default 0.15). The command prints both values and whether each limit was exceeded, and exits non-zero when either one is; measured reference: a recolour scores 0.003 whole / 0.074 tile, a 6 px block move 0.001 / 0.21, a panel move 0.025 / 0.30. `--out` receives `side-by-side.png` (our render, our edge map, the reference, its edge map), `heatmap.png` (both edge maps beside the absolute-difference heatmap) and `tiles.csv` (every tile); tile coordinates are image coordinates, also under `--region`, where the grid starts at the region origin. Both inputs must already be the same logical size: reference captures are 1727×1029, a 2x capture is 3454×2058 and is downscaled to 1727×1029 before comparison (for example `ffmpeg -i NAME@2x.png -vf scale=iw/2:ih/2 NAME.png`); a size mismatch is an error naming both sizes and the tool never resizes. `ui-audit` renders each scene at its declared window (`WINDOW` is 1440×900; a scene may pass its own size) with `pixels_per_point` 1x/2x, so a capture-backed scene at the reference size declares `Harness::builder().with_size(egui::vec2(1727.0, 1029.0)).with_pixels_per_point(1.0)`; when the scene window differs, `--region` selects the common logical area.

A UI ticket's report states the whole-region error before and after for every changed region; the coordinator rejects a region whose error did not fall. The score is a regression signal, not a pass line, wherever our image differs from the capture by something approved: judge layout per component crop, at its own measured region, on the controls present in both (position, size and spacing must match exactly), and name every residual of a larger region by cause (native window border, traffic lights and shadow that the audit surface cannot draw, controls hidden because nothing works behind them, dynamic text, content the engine cannot yet produce, our palette and icons); where possible add a masked run that excludes those parts and report its score. The side-by-side images remain the judge of layout.

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
