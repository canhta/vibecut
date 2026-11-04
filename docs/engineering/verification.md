# Verification

## Checks

Run `cargo xtask ci` from the root before every commit; commit only green states.
It runs workspace formatting checks, Clippy with warnings denied, and tests.
Use `cargo fmt --all` to apply formatting. Rust is pinned in `rust-toolchain.toml`.

Choose focused tests for changed behavior and credible failures. Add regressions
for reproduced defects; avoid tests that copy implementation or assert cosmetic
text. Generate fixtures into `target/`; never commit media or model weights.
Hosted CI supplements local checks under the [delivery policy](workflows.md#delivery).

## Evidence

Report exact commands, exit results, revision and relevant limits. Label claims
observed, source-documented, inferred, proposed or unverified. A passing build
proves neither runtime editing behavior, AI quality nor export correctness.
Retain reports locally and hand over paths; publication requires explicit direction.
