# Verification

## Checks

Run the smallest check that covers what changed; commit only green.

| Changed | Run |
|---|---|
| Markdown, tickets, assets only | `cargo xtask docs` (crate layers, asset sidecars, name gate; ~1 s) |
| Rust inside one crate, while iterating | `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets` |
| Before handoff | `.scratch/bin/build cargo xtask ci --require-oracles` once on the frozen SHA (fmt, Clippy, all tests, layers, assets, name gate, required oracles) |
| Media, render or other hot path | The above plus the benchmark its ticket names |

Run `cargo xtask ffmpeg` once per machine before the first local Rust build.
While iterating, use crate-scoped tests and Clippy.

CI runs once per change, by the implementer, on the frozen SHA:
`.scratch/bin/build cargo xtask ci --require-oracles` leaves a receipt for that tree,
and asking again for the same tree returns the receipt instead of rebuilding. A
reviewer reads the receipt and runs only targeted reproductions; a reviewer's own fix
commit gets one CI run. A clean rebase merges without another run. Hosted CI runs on
`main` only for the newest commit of a burst of merges (older runs cancel), and on
demand for Windows via `.scratch/bin/hosted-ci`; a red `main` is fixed or reverted at
once by whoever merged last.

## AI model integrations

- **Third-party models** (ASR, translation, TTS, LLM providers): prove the
  integration works: contract, consent, failure paths and one bounded live smoke per
  provider showing real output. Their accuracy belongs to the model the user picks
  and changes with every release: no eval corpora, WER/MOS scores or quality gates,
  and no review finding asks for them.
- **Our own AI logic** (the agentic clip pipeline: selection, boundaries, reframe,
  and any prompt, heuristic or ranking we own): measured, because we improve it. Its
  eval names a fixed labelled set, a metric and a baseline, and every change to that
  logic reports the metric against the baseline. The clip pipeline harness is
  `cargo xtask clip-eval <set>`; see the
  [clip-eval contract](../../crates/clip-eval/README.md).
- **Live runs** (smokes, evals): prefer OpenAI; pure LLM calls (text in, text out) use
  only `gpt-6-luna`; other services (ASR, TTS, vision) may use any admitted provider.

## When to write tests

A test earns its place only if a plausible bug would make it fail and no other test
would. Coverage is not a goal; tests are code that slows every build and change.

Write one when:
- A behaviour changes: one test at the public interface that fails without the change.
- A bug or crash is fixed: the reproduction becomes a case in the existing test for
  that behaviour (a table row), not a new test or a `review_*`/`regressions` file.
- A risk has no cover yet: boundaries, hostile input, cancellation, failure paths.

Do not write one for:
- What the compiler or type system already guarantees, getters, constructors,
  plumbing, `Debug`/`Display`, or a third-party crate's behaviour.
- The same behaviour again at another layer: test it where it is decided; callers
  test only their own decisions.
- Private helpers: test through the public interface.
- Refactors that keep behaviour, docs and ticket edits.

Shape:
- One behaviour per test, its inputs as a table; name tests and files after the
  behaviour (`trim_refuses_past_source_end`), never after where a case came from.
- Prefer extending an existing test's table to adding a test; delete tests a change
  makes redundant in the same commit.
- A reviewer flags a redundant, duplicate or implementation-coupled test as a
  `changes` finding, the same as a missing one.

## Test selection

| Change | Evidence |
|---|---|
| Parser, validation, pure function | Unit tables: valid, boundary, hostile and truncated input, no panic |
| Time, mapping, edit invariants | Property tests with kept seeds: rational rates, boundaries, random commands, undo/redo |
| Decode, mux, export, A/V sync | ffmpeg/ffprobe oracle: stream parameters, frame and sample counts, stated tolerances |
| Text, compositor, subtitle overlay | Deterministic image goldens incl. Vietnamese combining marks and clipping |
| Engine, session, jobs | Command integration: refused states, atomicity, undo/redo, cancellation, failure |
| Desktop interaction | egui_kittest through real commands via automation ids, asserting engine and widget state, never pixels. Screenshots are a separate audit: `cargo xtask ui-audit` renders named scenes offscreen at 1x and 2x for the ticket report and design review; it never runs in `cargo xtask ci` or hosted CI. OS boundaries are injected at their seam, never driven: a file picker or drop delivers paths into the same app path, so tests push `RawInput.dropped_files` or the picker result directly; Finder, OS dialogs, `osascript` and coordinate clicks are not test tools. A ticket never opens a native window on the maintainer's screen; real-app runs drive the app through its control socket (p2-platform/02) by command, launched in the background without activation or a visible window (never taking keyboard focus), and the OS dialog itself is covered by one native smoke pass per milestone, run only when the maintainer grants the screen |
| Provider adapters | Offline fixtures for schema, errors, retry, consent; separate live test |

Tests exercise public interfaces with injected codec and provider adapters,
including cancellation and failure. Concurrency tests order events with gates or
channels the test controls, never with sleeps or timing margins.

## Fixtures

Generate synthetic media with ffmpeg into `<repo>/target/fixtures/<area>/` and record
the command and tool version. Missing ffmpeg/ffprobe skips with a message;
`VIBECUT_REQUIRE_ORACLES=1` makes it a failure, and a skipped oracle never counts as
acceptance. Goldens use fixed canvas, rate, fonts and CPU rendering; actual and diff
images go under `target/`; re-blessing needs visual inspection and a reason.

Live provider tests need `VIBECUT_LIVE_AI=1` plus keys, are `#[ignore]`d, never run
in CI, and use synthetic input with bounded cost.

## Evidence

Each claim carries a label (observed, source-documented, inferred, proposed,
unverified), the exact command, revision and result. Skipped checks are listed with
their reason. Benchmarks state hardware/OS, profile, revision, input, repetitions,
cold or warm, elapsed time, throughput and peak RSS; numbers are measured, never
estimated.

## Reports

Every report opens with `## TL;DR`: at most 10 lines (outcome, SHA or blocker,
decisions needed, follow-ups).

Done report: ticket, branch, SHAs, diff stat; each Verify item → command → result;
dependency lookups; observed vs unverified behaviour; proposed follow-ups; remaining
work.

Review report: ticket, SHA, scope, checks run; each finding with severity
(critical/high/medium/low), `file:line`, reproduction, impact and fix; verdict
`approve` or `changes`. Correctness, security and data-loss findings block approval.
