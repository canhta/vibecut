# Source standards

## Safety

Return actionable `Result` errors; background jobs report failures instead of
silently stopping. Workspace Clippy denies panic shortcuts outside tests;
`clippy.toml` permits test assertions. Treat media/project files, CLI parameters
and AI responses as hostile: use `get()`, checked arithmetic, nonzero divisors
and capped input-sized allocations. Review these boundaries with focused tests.

Workspace Rust lints forbid unsafe code except in the single OS-codec FFI crate:
it overrides the workspace policy with `unsafe_code = "deny"`, permits unsafe only
in specific FFI modules with `#[allow(unsafe_code)]`, documents each unsafe block
with `// SAFETY:`, and exposes a safe `Result` API. No other exception is admitted.
[Architecture](architecture.md#media-and-time) owns exact edit/timing representation.

## Structural admission

Admit modules and crates only for actual responsibilities, consumers or dependency
boundaries. Avoid `utils`, `common` and `manager` buckets and empty future crates.
Use Rust naming conventions and the narrowest visibility consumers need. Comments
explain only non-obvious reasons; omit attribution/provenance, origin notes and
filler comments. Record required asset and license notices under the
[open-source policy](open-source.md). New crates inherit `[lints] workspace = true`;
review dependencies against [architecture](architecture.md).
