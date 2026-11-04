# Source standards

## Safety

Treat media, project files, CLI parameters and AI responses as hostile: `get()`,
checked arithmetic, nonzero divisors, input-capped allocations, bounded recursion.
Return actionable `Result` errors; Clippy denies panic shortcuts outside tests.
Every crash fix adds the input that crashed as a regression test.

`unsafe` is forbidden except in `media` (the FFmpeg boundary) and the
[Whisper callback module](../adr/0003-whisper-callback-boundary.md). There it sits in
named FFI modules with `#[allow(unsafe_code)]`, a `// SAFETY:` comment per block and
a safe `Result` API on top; raw pointers and GPU plane slices stay private. Prefer
`ffmpeg-next` safe APIs and use direct FFI only for a concrete gap.

## Structural admission

Admit a module or crate only for a real responsibility with a consumer. Name it
after its concept (no `utils`, `common` or `manager`). Before writing logic, find
its existing owner and reuse it; when a second consumer appears, move the logic to
the concept's owner. Test helpers stay dev-only in their owner. New crates inherit
`[lints] workspace = true` and join the [crate map](architecture.md#crate-map).
Comments explain non-obvious reasons only.

## Minimal solutions

Take the first rung that works: not needed now, existing owner, std, platform
feature, existing dependency, minimum new code. Guidance, not a gate: keep the
ADR-chosen seams for languages, providers and codecs, and keep validation, data-loss
handling, security and accessibility on every rung. Mark a deliberate shortcut with
`// ponytail: <ceiling>, <upgrade path>`.

## Quality and speed

Prefer hardware with same-quality software fallback. Stream bounded buffers; avoid
needless frame, audio and raster copies. Parallel work runs through the engine job
runner and its content-keyed caches. Every hot-path change reports speed and an
output-quality metric (VMAF/SSIM, loudness/peaks, pixel diff or tick drift) per
[evidence](verification.md#evidence); quality never drops silently.

## Errors and concurrency

Each product crate has one typed error enum, with `Unsupported` and `Cancelled`
where they apply, sources kept via `#[source]`/`#[from]`, and safe IDs as context.
Public APIs never return `String`, boxed errors or provider bodies.

Background workers catch panics at entry and report a typed job failure. Recover a
poisoned lock only after validating state. Propagate cancellation, join workers,
hold no lock across IO or await. Audio callbacks never block, allocate unboundedly
or log. Tests cover cancellation and worker failure.

## Logging

`tracing` events; only apps install subscribers. Levels: error failed operation,
warn recoverable degradation, info lifecycle, debug bounded diagnostics, trace
opt-in. Fields carry only IDs, stages, counts, durations and error categories:
keys, headers, paths, transcripts, prompts, user text and provider bodies stay out.
Sensitive types implement redacting `Debug`; use `#[instrument(skip_all)]` with
explicit fields.

## Time and text

Edit, seek, mapping and A/V math use [integer ticks](architecture.md#media-and-time)
with source and output coordinates distinct in types. Floating seconds exist only at
display and provider boundaries: validate, convert once, name the rounding mode.

Text is NFC; edit, count and split by grapheme cluster. Test Vietnamese combining
forms. Paths stay `Path`/`OsStr` end to end, including Unicode and spaces;
unrepresentable paths are typed errors.

## Public APIs and crate README

Private by default; `pub` only for real consumers. Constructors validate invariants.
Public docs state units, bounds, failures and cancellation. Each crate README states
purpose, exclusions, entry points, supported inputs, limits, tests and measured
evidence.
