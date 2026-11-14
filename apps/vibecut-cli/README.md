# vibecut-cli

## Purpose

Headless `vibecut-cli` process adapter. Parses native command-line arguments, dispatches edits through the engine registry, composes project writer leases and engine export jobs, and maps typed results to stdout/stderr and stable exit codes. It owns no timeline, codec, export-admission, provider or MCP protocol logic and has no UI dependencies. The MCP adapter itself lives in the [MCP crate](../../crates/mcp/README.md); this CLI only composes its `mcp` subcommand.

## Module map

`lib.rs` wires the public `run`, `Output` and `CliError` entry points. The module doors follow [structure](../../docs/engineering/structure.md#cli-appsvibecut-clisrc):

| Module | Responsibility | Contract |
|---|---|---|
| `args.rs` / `args/` | Parses and validates native arguments. | [Arguments](src/args/README.md) |
| `commands.rs` / `commands/` | Dispatches each command through one `run` adapter. | [Commands](src/commands/README.md) |
| `commands/ai/` | Adapts subtitle, provider, model, clip, chapter, filler and agent commands. | [AI commands](src/commands/ai/README.md) |
| `commands/catalog/` | Prints the generated command and schema catalogs. | [Catalog commands](src/commands/catalog/README.md) |
| `commands/libraries/` | Adapts font and sound library commands. | [Library commands](src/commands/libraries/README.md) |
| `commands/media/` | Adapts export, normalize, probe, scene, waveform and thumbnail commands. | [Media commands](src/commands/media/README.md) |
| `commands/media/batch/` | Runs one bounded export job file. | [Batch export](src/commands/media/batch/README.md) |
| `commands/project/` | Adapts control, exec, lifecycle, caption, cut, reframe and transcript commands. | [Project commands](src/commands/project/README.md) |
| `mcp.rs` / `mcp/` | Composes the `mcp` subcommand over the MCP crate. | [MCP command](src/mcp/README.md) |
| `output.rs` / `output/` | Shapes results, progress and upload disclosures. | [Output](src/output/README.md) |
| `session.rs` / `session/` | Holds the leased control session and export admission. | [Session](src/session/README.md) |

Every in-process project edit builds a typed `engine::Command`. Workflow and asset-library adapters send their registry request through the control entry too, so the wire boundary remains the only string-id path: `exec` (id and JSON come from the caller), `control` frames, MCP tool calls and the one-shot registry helper.

## Entry points

The [CLI contract](../../docs/product/agent-control/cli.md) owns syntax, supported commands, flags, output shapes and exit codes. `run(Vec<OsString>)` returns `Output` or `CliError`; the binary removes the global JSON flag, writes the selected result and maps `CliError::exit_code`. Paths keep their native representation until JSON output needs them; an unrepresentable path returns a typed unsupported error. The `session` module exposes the one-shot writer-lease runner, the control stream, shared export admission, the engine request envelope filler and the runtime settings each command borrows.

## Limits

Each process opens the [host-sized budget](../../crates/engine/src/runtime/README.md#host-sized-default-budget) through `Context::budget`; every job keeps its own engine budget. Bounded batch job files, control frames and MCP frames use the engine limits. The MCP subcommand's queue, connection and deadline bounds belong to the [MCP crate](../../crates/mcp/README.md#limits).

## Tests

The single integration binary mirrors native command groups under `tests/it/cli/`. The `fixtures` door exposes project/media setup, `process` owns subprocess invocation, and `tests/it/media_fixture.rs` owns external media oracle fixtures. `tests/it/mcp.rs` covers the `mcp` subcommand through its public headless adapter. Progress is latest-only and throttled through engine; stderr stages are observational and cannot synchronize tests. Oracle tests need external FFmpeg/ffprobe: a missing tool prints a skip, required-oracle mode refuses a skip, and a found but failing tool fails. Run `cargo xtask ci --require-oracles` for the full check.
