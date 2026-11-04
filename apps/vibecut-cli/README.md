# vibecut-cli

Headless `vibecut` process adapter. Parses native command-line arguments, dispatches
edits through the engine registry, composes project writer leases and engine export
jobs, and maps typed results to stdout/stderr and stable exit codes. It owns no
timeline, codec, export-admission or provider logic and has no UI dependencies.

The [CLI contract](../../docs/product/cli.md) owns syntax, supported commands,
flags, output shapes and exit codes. `run(Vec<OsString>)` returns `Output` or
`CliError`; the binary removes the global JSON flag, writes the selected result
and maps `CliError::exit_code`. Paths retain native representation until an output
path must be represented in JSON, where unrepresentable paths return a typed
unsupported error.

`exec` uses [engine commands](../../crates/engine/README.md#public-entry-points)
and holds the [project lease](../../crates/project/README.md#writer-lease-and-save-acknowledgement)
from load through save. Export uses the saved snapshot and
[render admission](../../crates/render/README.md#export-and-publication) through
engine jobs, with a fixed 2 GiB aggregate budget. It has no resume, provider
execution or persisted preferences. Process interruption follows crash semantics;
it has no graceful signal-to-job cancellation handler. A completed export reports
its output path, not render's detailed receipt.

`cargo test -p vibecut-cli` covers command dispatch, Unicode/space paths, refused
parameters, subprocess lease contention, progress, export geometry, degraded
consent and source loss before/during export. Oracle tests require external
FFmpeg/ffprobe: missing tools print a skip, required-oracle mode refuses a skip,
and found but failing tools fail. Run `cargo xtask ci --require-oracles` for the
full workspace check and tool/fixture receipts. No speed or memory measurement
is claimed; Windows runtime behavior remains unverified.
