# Verification

## Checks

Run the smallest check that covers what changed; commit only green.

| Changed | Run |
|---|---|
| Markdown, tickets, assets only | `cargo xtask docs` (crate layers, asset sidecars, name gate; ~1 s) |
| Rust inside one crate, while iterating | `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets` |
| Any Rust, `Cargo.toml`, `xtask`, before handoff or merge | `cargo xtask ci` once (fmt, Clippy, all tests, layers, assets, name gate) |
| Media, render or other hot path | the above plus the benchmark its ticket names |

Run `cargo xtask ffmpeg` once per machine before the first Rust build. Merging after
a rebase reruns `cargo xtask ci` only when the commits added to `main` since your run
touched Rust, manifests or `xtask`. A reviewer runs CI once on the SHA it merges.

The name gate reads local regexes from `.scratch/forbidden-names.txt` and rejects
matches in tracked files and branch commit messages.

## When to write tests

- Every behaviour change: a test at the public interface that fails without the change.
- Every bug or crash fix: the reproduction first, as a regression test.
- Refactors that keep behaviour, docs and ticket edits: no new tests.

## Test selection

| Change | Evidence |
|---|---|
| Parser, validation, pure function | Unit tables: valid, boundary, hostile and truncated input, no panic |
| Time, mapping, edit invariants | Property tests with kept seeds: rational rates, boundaries, random commands, undo/redo |
| Decode, mux, export, A/V sync | ffmpeg/ffprobe oracle: stream parameters, frame and sample counts, stated tolerances |
| Text, compositor, subtitle overlay | Deterministic image goldens incl. Vietnamese combining marks and clipping |
| Engine, session, jobs | Command integration: refused states, atomicity, undo/redo, cancellation, failure |
| Desktop interaction | egui_kittest through real commands, plus screenshot and manual observation |
| Provider adapters | Offline fixtures for schema, errors, retry, consent; separate live test |

Tests exercise public interfaces with injected codec and provider adapters,
including cancellation and failure.

## Fixtures

Generate synthetic media with ffmpeg into `<repo>/target/fixtures/<area>/` and record
the command and tool version. Missing ffmpeg/ffprobe skips with a message;
`VIBECUT_REQUIRE_ORACLES=1` makes it a failure, and a skipped oracle never counts as
acceptance. Goldens use fixed canvas, rate, fonts and CPU rendering; actual and diff
images go under `target/`; re-blessing needs visual inspection and a reason.
Recordings, media and weights stay out of Git.

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
