# Documentation

| Owner | Facts |
|---|---|
| [Requirements](product/requirements.md) | Principles, scope, providers, languages |
| [UI design](product/ui.md) | UI principles, design language, reference evidence; per-window geometry in [editor](product/ui-editor.md), [Home](product/ui-home.md), [shorts](product/ui-shorts.md) |
| [UI tokens](product/tokens.md) | Token contracts, Rust owners, measured sources |
| [Keyboard](product/keyboard.md) | Generated action table, binding persistence and platform modifiers |
| [CLI](product/cli.md) | Headless commands, JSON output, exit codes, project lease and export consent |
| [MCP](product/mcp.md) | Agent control transport, tools, attach, grants and failure codes |
| [Commands](product/commands.md) | Generated command reference: schemas, effects and examples |
| [Architecture](engineering/architecture.md) | Crates, dependency edges, contracts, media and time |
| [Structure](engineering/structure.md) | Layers, folders, module naming and enforcement |
| [Standards](engineering/standards.md) | Rust safety, structural admission, errors, logging, time and text |
| [Workflows](engineering/workflows.md) | Ticket loop, research, copy imports, upgrades, commits, provisioning summary |
| [Verification](engineering/verification.md) | Checks, tests, fixtures, evidence, reports |
| [Open-source policy](engineering/open-source.md) | Licences, FFmpeg distribution, assets, model weights, privacy |
| [Orchestration](agents/orchestration.md) | Agent roles, tiers, dispatch, review, merge, cleanup |
| [Issue tracker](agents/issue-tracker.md) | Ticket files, header, queries |
| [Domain docs](agents/domain.md) | Domain model, glossary and ADR conventions |
| [Glossary](../GLOSSARY.md), [ADRs](adr/README.md) | Domain terms; decision rationale |

## Where each kind of fact lives

| Fact | Owner | Never in |
|---|---|---|
| Scope, priority, providers, languages | [Requirements](product/requirements.md) | tickets as the source of truth |
| UI geometry, copy rules, states, gaps | [UI design](product/ui.md) and its window files; values in `kit/theme.rs` via [tokens](product/tokens.md) | widget literals; ticket ids |
| Agent-facing surfaces | [CLI](product/cli.md), [MCP](product/mcp.md), [Keyboard](product/keyboard.md), [Commands](product/commands.md) | crate READMEs |
| Crates and edges, the layer of each crate | [Architecture](engineering/architecture.md) | structure.md beyond its own rules |
| Inside a crate: folders, layers, naming | [Structure](engineering/structure.md), `xtask/modules.toml` | standards.md beyond its link |
| Contract of one module | its crate README, section of its folder | architecture.md beyond the index row |
| Why a decision was made | [ADRs](adr/README.md) | owner docs |
| Domain term | [Glossary](../GLOSSARY.md) | - |
| Build and provisioning detail | [xtask](../xtask/README.md) | workflows.md beyond its summary and link |
| Hosts, Orca, relay, tiers' track record, ticket state | `.scratch/` and the private workspace | any tracked file |

Owner docs state current contracts in terse English: change a fact at its owner and
link to it elsewhere. Length is not a budget; duplication is. Rationale and history
belong in ADRs.

The root README shows the finished product to users: features with media, install, links. Status, plans and developer mechanics live in `.scratch/` and CONTRIBUTING.
