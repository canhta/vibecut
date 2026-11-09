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

The [CLI contract](../../docs/product/agent-control/cli.md) owns syntax, supported commands, flags, output shapes and exit codes. `run(Vec<OsString>)` returns `Output` or `CliError`; the binary removes the global JSON flag, writes the selected result and maps `CliError::exit_code`. Paths keep their native representation until JSON output needs them; an unrepresentable path returns a typed unsupported error.

The single integration binary mirrors native command groups under `tests/it/cli/`. The `fixtures` door exposes project/media setup, `process` owns subprocess invocation, and `tests/it/media_fixture.rs` owns external media oracle fixtures. Progress is latest-only and throttled through engine; stderr stages are observational and cannot synchronize tests. Oracle tests need external FFmpeg/ffprobe: a missing tool prints a skip, required-oracle mode refuses a skip, and a found but failing tool fails. Run `cargo xtask ci --require-oracles` for the full check.
