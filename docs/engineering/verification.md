# Verification

## Checks

Run `cargo xtask ffmpeg` to provision the current LGPL libraries, then
`cargo xtask ci` from the root before every commit; commit only green states.
It checks formatting, Clippy with warnings denied, workspace tests, architecture
layers and asset sidecars. Use `cargo fmt --all` to format. Rust is pinned in
`rust-toolchain.toml`. `cargo xtask layers` reads the
[crate table](architecture.md#crate-map) using Cargo metadata; all dependency kinds,
optional and target-specific edges count. External UI/platform dependencies still
need source review. `cargo xtask new-crate <name>` scaffolds only admitted core
packages; fill its README contract before delivery, add behavior, then run CI.

If `.scratch/forbidden-names.txt` exists, CI reads its case-insensitive extended
regexes (one nonempty pattern per line) and rejects matches in tracked working-tree
files and branch commit messages since merge-base with `main`. On `main` it checks
HEAD's message. Absent patterns skip silently. The file remains local and ignored.
License admission is a [dependency review](open-source.md#licensing); no automatic
license or online freshness gate is installed. Record release lookup evidence.

## Test selection

| Change | Required evidence |
|---|---|
| Pure function, parser, validation | Unit tables: valid, boundary, hostile/truncated inputs; errors without panic |
| Time, mapping, edit invariants | Property tests: rational rates, boundaries, random commands, undo/redo and reproducible seeds |
| Demux/mux/decode, export, A/V sync | External ffmpeg/ffprobe oracle, stream parameters, frame/sample counts and stated tolerances |
| Text, compositor, subtitle overlay | Deterministic image goldens plus Vietnamese combining marks and clipping cases |
| Engine/session/jobs | Command integration: enabled/refused states, atomicity, undo/redo, cancellation and failures |
| Desktop interaction | egui_kittest flows through real commands, screenshot and manual app observation for native behavior |
| Provider adapters | Offline response fixtures for schema/errors/retry/consent; separately gated live-provider test |

Tests establish changed behavior, not cosmetic wording or a mirror of implementation.
Keep property regression seeds. A test that skips a required oracle is not oracle
acceptance. FFmpeg is the production backend; subprocess ffmpeg/ffprobe tests independently
exercise its CLI and stream metadata, and do not establish independence of codec
algorithms. Native codec evidence is platform-specific; other platforms stay unverified.

## Fixtures

Generate synthetic media with an independently installed ffmpeg into `<repo>/target/fixtures/<area>/`,
independent of `CARGO_TARGET_DIR`. Record the generation command/tool version and
assert subprocess success. Use ffprobe to check streams, durations, offsets and
export outputs; compare decoded pixels/audio with tolerances justified by codec loss.
Fixture helpers skip with an explicit message when ffmpeg/ffprobe is absent;
`VIBECUT_REQUIRE_ORACLES=1` turns missing tools into failure. A found tool that fails
is a failure, not a skip. Never commit recordings, media or model weights.

Goldens use generated inputs or [licensed small assets](open-source.md#assets),
fixed canvas/rate/font/settings and deterministic CPU rendering. Keep fixture
generation/probe helpers dev-only with media, and raster comparison helpers
dev-only with their rendering owner; reuse them when a second test needs them.
Tests exercise public execution interfaces with injected codec/provider adapters,
including cancellation and failure, rather than bypassing them for helper tests. Pin test fonts and
avoid machine-dependent font fallback. Keep small expected PNGs with attribution;
write actual/diff images under `target/`. Changes require visual inspection and an
explained re-bless, never automatic acceptance. Record dimensions and pixel tolerance.

Live-provider tests require both `VIBECUT_LIVE_AI=1` and credentials; also mark them
ignored so ordinary tests stay offline. Never run them in CI. Use synthetic inputs,
explicit upload consent and bounded cost/time; do not log secrets or user text.

## Evidence

Label every material claim observed, source-documented, inferred, proposed or
unverified. Include exact command, revision, exit result and limits. Builds do not
prove editing behavior, AI quality, export correctness or native playback. Record
skipped/unrun checks and their reason, with remaining acceptance explicitly unverified.
Benchmark evidence states hardware/OS, build profile, revision, input generation,
resolution/rate/duration, exact command, repetitions and observed elapsed time,
throughput and peak RSS where relevant; distinguish cold/warm runs. No guessed numbers.
Keep reports local and hand over paths; never publish an Artifact.

## Reports

Every report starts with `## TL;DR`: at most 10 lines (outcome, merged SHA or
blocker, decisions needed, follow-ups). Details follow below it.

Done report fields (omit none; use an explicit reason when not applicable):

- Ticket, branch, commit SHAs and diff stat.
- Verify item -> exact command -> exit result -> evidence path; CI tail.
- Latest stable dependency/toolchain/Action/API lookup -> official source -> result.
- Observed behavior; source-documented facts; inferred/proposed choices; unverified
  acceptance and skips; follow-up tickets proposed, not silently created.
- Remaining work and review status of the exact revision.

Review report fields:

- Ticket, reviewed SHA and scope; inspected consumers and checks run/unrun.
- Each finding: severity (`critical`, `high`, `medium`, `low`), `file:line`, concrete
  evidence/reproduction, impact and proposed fix. Separate facts from inference.
- Verdict `approve` or `changes`; correctness, security and data-loss findings block
  approval. List remaining limitations even with no findings.
