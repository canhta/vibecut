# Documentation

## Where each kind of fact lives

| Fact | Owner | Never in |
|---|---|---|
| Scope, priority, providers, languages | [Requirements](product/requirements.md) | tickets as the source of truth |
| UI principles, design language, reference evidence, geometry, copy rules, states, gaps | [UI design](product/ui/index.md) and its window files ([editor](product/ui/editor.md), [Home](product/ui/home.md), [shorts](product/ui/shorts.md)) | widget literals; ticket ids |
| Token contracts, Rust owners, measured sources | [UI tokens](product/ui/tokens.md); values in `kit/theme.rs` | - |
| Agent-facing surfaces | [CLI](product/agent-control/cli.md), [MCP](product/agent-control/mcp.md), [Keyboard](product/agent-control/keyboard.md), [Commands](product/agent-control/commands.md) | crate READMEs |
| Rust safety, structural admission, errors, logging, time and text | [Standards](engineering/standards.md) | - |
| Ticket loop, research, copy imports, upgrades, commits, provisioning summary | [Workflows](engineering/workflows.md) | - |
| Checks, tests, fixtures, evidence, reports | [Verification](engineering/verification.md) | - |
| Licences, FFmpeg distribution, assets, model weights, privacy | [Open-source policy](engineering/open-source.md) | - |
| Crates and edges, the layer of each crate | [Architecture](engineering/architecture.md) | structure.md beyond its own rules |
| Inside a crate: folders, layers, naming | [Structure](engineering/structure.md), `xtask/modules.toml` | standards.md beyond its link |
| Contract of one module | its crate README, section of its folder | architecture.md beyond the index row |
| Why a decision was made | [ADRs](adr/README.md) | owner docs |
| Domain model, glossary and ADR conventions | [Domain docs](agents/domain.md) | - |
| Domain term | [Glossary](../GLOSSARY.md) | - |
| Agent roles, tiers, dispatch, review, merge, cleanup | [Orchestration](agents/orchestration.md) | - |
| Ticket files, header, queries | [Issue tracker](agents/issue-tracker.md) | - |
| Build and provisioning detail | [xtask](../xtask/README.md) | workflows.md beyond its summary and link |
| Hosts, Orca, relay, tiers' track record, ticket state | `.scratch/` and the private workspace | any tracked file |

Owner docs state current contracts in terse English: change a fact at its owner and
link to it elsewhere. Length is not a budget; duplication is. Rationale and history
belong in ADRs.

The root README shows the finished product to users: features with media, install, links. Status, plans and developer mechanics live in `.scratch/` and CONTRIBUTING.
