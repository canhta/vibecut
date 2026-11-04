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

Before writing logic, search for its existing owner and reuse it. When a second
consumer appears, move the logic to the module that owns the concept instead of
copying it. Keep reusable fixture/golden helpers dev-only in their concept owner;
admit a separate test package only when actual consumers require it.

## Minimal solutions

Prefer the first sufficient option: needed now, existing owner, Rust std, native
platform feature, existing dependency, then minimum new code. This is guidance,
not an admission gate: preserve ADR-chosen seams and planned language, provider
and codec extensibility. Consider deletion before addition, fixed values before
configuration, and concrete code before a hypothetical single-adapter interface.
For a deliberate simplification with a known ceiling, write
`// ponytail: <ceiling>, <upgrade path>`. Preserve hostile-input validation,
data-loss handling, security and accessibility at every option.

## Errors and concurrency

Product crates expose one typed error enum each (derive with `thiserror` when useful),
with variants for invalid input, unsupported input/capability (`Unsupported`) and cancellation
(`Cancelled`) where applicable. Keep source errors with `#[source]`/`#[from]` and
safe operation/asset-ID context. Public APIs return typed errors, never `String`,
opaque boxed errors or provider response bodies. The xtask executable may use
string diagnostics because it exposes no library API.

Catch unwinding panics at background worker entry points and report a typed job
failure; do not format arbitrary panic payloads into logs. `catch_unwind` cannot
recover aborts. Handle poisoned locks explicitly: recover only after validating
state, otherwise fail the job. Own worker shutdown, propagate cancellation and
join workers; do not hold locks over IO or awaits. Audio callbacks neither block,
allocate unboundedly nor log. Tests cover cancellation and worker failure.

## Logging

Use `tracing`: structured events and job spans correlate concurrent work (see
[upstream](https://docs.rs/tracing/latest/tracing/)). Libraries emit events; only
apps install subscribers. Error = failed operation, warn = recoverable degradation,
info = lifecycle/stage, debug = bounded diagnostic metadata, trace = opt-in detail.
Default to info for ours and warn for dependencies; rate-limit progress events.

Allow only IDs, stages, counts, durations and safe error categories. Never record
keys, authorization headers, paths, transcripts, prompts, user text or provider
bodies. Sensitive wrappers implement redacting `Debug` and safe `Display`; audit
nested errors too. Use `#[instrument(skip_all)]` with explicit safe fields:
[instrument records arguments by default](https://docs.rs/tracing/latest/tracing/attr.instrument.html).

## Time and text

Use [integer ticks](architecture.md#media-and-time) for edit, seek, mapping and
A/V math; distinguish source and output coordinates in types/arguments. Rational
rates and checked integer arithmetic govern rescaling. Floating seconds occur
only at display/provider boundaries: validate finite/range, convert once and name
the rounding mode (floor, ceil or nearest with ties specified). Never accumulate
floating durations or apply an audio offset twice.

Normalize ingested text to NFC; edit, count and split by Unicode grapheme clusters,
not bytes or scalar count. Preserve Vietnamese diacritics; test combining forms.
Keep paths as `Path`/`PathBuf`/`OsStr`, accept spaces and Unicode, and pass arguments
directly to subprocesses. Never use lossy conversion for file identity, persistence
or subprocess arguments; reject an unrepresentable serialized path with a typed error.

## Public APIs and crate README

Default private, then `pub(crate)`; expose `pub` only for actual external consumers.
Validate invariants in constructors; hide fields that could bypass them. Document
units, supported inputs, bounds and failure/cancellation behavior on public APIs.
Every product crate README states purpose/exclusions, public entry points, supported
inputs, limits, tests/oracles and any observed accuracy/performance evidence.
