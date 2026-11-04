# Verification

## Checks

`cargo xtask ffmpeg` once, then `cargo xtask ci` before every commit: fmt, Clippy
(warnings denied), tests, crate layers, asset sidecars and the name gate. Commit
only green. `cargo xtask --help` lists the rest.

The name gate reads local regexes from `.scratch/forbidden-names.txt` and rejects
matches in tracked files and branch commit messages.

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
