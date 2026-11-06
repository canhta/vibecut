# vibecut-cli

Headless `vibecut-cli` process adapter. Parses native command-line arguments, dispatches
edits through the engine registry, composes project writer leases and engine export
jobs, and maps typed results to stdout/stderr and stable exit codes. It owns no
timeline, codec, export-admission or provider logic and has no UI dependencies.

`lib.rs` wires the public `run`, `Output` and `CliError` entry points.
The module doors follow [structure](../../docs/engineering/structure.md#cli-appsvibecut-clisrc):


| Module | Responsibility | Contract |
|---|---|---|
| `args.rs` / `args/` | Parses and validates native arguments. | [Arguments](src/args/README.md) |
| `commands.rs` / `commands/` | Dispatches each command through one `run` adapter. | [Commands](src/commands/README.md) |
| `commands/ai/` | Adapts subtitle, provider, model and clip commands. | [AI commands](src/commands/ai/README.md) |
| `commands/catalog/` | Prints the generated command and schema catalogs. | [Catalog commands](src/commands/catalog/README.md) |
| `commands/libraries/` | Adapts font and sound library commands. | [Library commands](src/commands/libraries/README.md) |
| `commands/media/` | Adapts export, normalize, probe and scene commands. | [Media commands](src/commands/media/README.md) |
| `commands/media/batch/` | Runs one bounded export job file. | [Batch export](src/commands/media/batch/README.md) |
| `commands/project/` | Adapts control, exec, lifecycle and caption commands. | [Project commands](src/commands/project/README.md) |
| `mcp.rs` / `mcp/` | Serves the authenticated MCP tool surface. | [MCP](src/mcp/README.md) |
| `output.rs` / `output/` | Shapes results, progress and upload disclosures. | [Output](src/output/README.md) |
| `session.rs` / `session/` | Holds the leased control session and export admission. | [Session](src/session/README.md) |

The [CLI contract](../../docs/product/agent-control/cli.md) owns syntax, supported commands,
flags, output shapes and exit codes. `run(Vec<OsString>)` returns `Output` or
`CliError`; the binary removes the global JSON flag, writes the selected result
and maps `CliError::exit_code`. Paths retain native representation until an output
path must be represented in JSON, where unrepresentable paths return a typed
unsupported error.
The single integration binary mirrors native command groups under `tests/it/cli/`.
The test `fixtures` door exposes project/media setup, `process` owns subprocess
invocation, and `tests/it/media_fixture.rs` owns external media oracle fixtures.
The existing unit tests follow their argument, output, session or batch interface;
assertions and behaviour names are retained.

`cargo test -p vibecut-cli` covers command dispatch, Unicode/space paths, refused
parameters, subprocess lease contention, progress, export geometry, keyframed
opacity decoded from an export, rectangle masks applied by `exec` and decoded
from an encoded export, degraded consent and source loss before/during
export, offline provider/model catalogs,
subtitle argument refusals and credential refusal before any upload, plus ASS import counts, style persistence and extension refusal. It also
covers clip argument refusals, the offline clip provider catalog and the
suggestion-credential refusal before any media work. Mid-job source loss
uses a synchronous render hook after admission and the first decoded frame,
then checks the real job result and CLI exit code while preserving the prior output.
Progress is latest-only and throttled through engine; stderr stages are observational
and cannot synchronize tests. Oracle tests require external
FFmpeg/ffprobe: missing tools print a skip, required-oracle mode refuses a skip,
and found but failing tools fail. Run `cargo xtask ci --require-oracles` for the
full workspace check and tool/fixture receipts. No speed or memory measurement
is claimed; Windows runtime behavior remains unverified.
