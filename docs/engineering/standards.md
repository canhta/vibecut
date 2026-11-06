# Source standards

## Safety

Treat media, project files, CLI parameters and AI responses as hostile: `get()`, checked arithmetic, nonzero divisors, input-capped allocations, bounded recursion. Return actionable `Result` errors; Clippy denies panic shortcuts outside tests. Tests follow [verification](verification.md#when-to-write-tests).

`unsafe` is forbidden except in `media` (the FFmpeg boundary) and the [Whisper callback module](../adr/0003-whisper-callback-boundary.md). There it sits in named FFI modules with `#[allow(unsafe_code)]`, a `// SAFETY:` comment per block and a safe `Result` API on top; raw pointers and GPU plane slices stay private. Prefer `ffmpeg-next` safe APIs and use direct FFI only for a concrete gap.

## Structural admission

Admit a module or crate only for a real responsibility with a consumer; [structure](structure.md#rules) owns where it lives, how folders group and which way imports point. New crates inherit `[lints] workspace = true` and join the [crate map](architecture.md#crate-map).

## Minimal solutions

Take the first rung that works: not needed now, existing owner, std, platform feature, existing dependency, minimum new code. Guidance, not a gate: keep the ADR-chosen seams for languages, providers and codecs, and keep validation, data-loss handling, security and accessibility on every rung. Mark a deliberate shortcut with `// ponytail: <ceiling>, <upgrade path>`.

## Quality and speed

Codec admission follows [media](../../crates/media/README.md#input-and-quality-boundaries). Stream bounded buffers; avoid needless frame, audio and raster copies. Execution owners follow [architecture](architecture.md#contracts); measure hot paths through [verification](verification.md#checks).

## Errors and concurrency

Each product crate has one typed error enum, with `Unsupported` and `Cancelled` where they apply, sources kept via `#[source]`/`#[from]`, and safe IDs as context. Public APIs never return `String`, boxed errors or provider bodies.

Background workers catch panics at entry and report a typed job failure. Recover a poisoned lock only after validating state. Propagate cancellation, join workers, hold no lock across IO or await. Audio callbacks never block, allocate unboundedly or log.

## Logging

`tracing` events; only apps install subscribers. Levels: error failed operation, warn recoverable degradation, info lifecycle, debug bounded diagnostics, trace opt-in. Fields carry only IDs, stages, counts, durations and error categories: keys, headers, paths, transcripts, prompts, user text and provider bodies stay out. Sensitive types implement redacting `Debug`; use `#[instrument(skip_all)]` with explicit fields.

## Time and text

Edit, seek, mapping and A/V math use [integer ticks](architecture.md#media-and-time) with source and output coordinates distinct in types. Floating seconds exist only at display and provider boundaries: validate, convert once, name the rounding mode.

Text is NFC; edit, count and split by grapheme cluster. Test Vietnamese combining forms. Paths stay `Path`/`OsStr` end to end, including Unicode and spaces; unrepresentable paths are typed errors.

## Public APIs and crate README

Expose `pub` APIs only to real consumers; constructors validate invariants. Public docs state units, bounds, failures and cancellation. Each crate README states purpose and exclusions, gives the public interface in a few lines and maps its top-level modules to their READMEs; each module README states that module's contract, supported inputs, limits, tests and measured evidence.
