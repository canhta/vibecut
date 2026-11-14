# Documentation

## Where each kind of fact lives

| Fact | Owner | Never in |
|---|---|---|
| Scope, priority, providers, languages | [Requirements](product/requirements.md) | tickets as the source of truth |
| UI principles, design language, reference evidence, geometry, copy rules, states, gaps | [UI design](product/ui/index.md) and its window files ([editor](product/ui/editor.md), [Home](product/ui/home.md), [shorts](product/ui/shorts.md)) | widget literals; ticket ids |
| Parity with the reference, one row per item: function and UI state | [Parity](product/parity/README.md) and its area files ([Home](product/parity/home.md), [shell](product/parity/shell.md), [assets](product/parity/assets.md), [captions](product/parity/captions.md), [catalogues](product/parity/catalogues.md), [video properties](product/parity/properties-video.md), [other properties](product/parity/properties-other.md), [timeline](product/parity/timeline.md), [player and export](product/parity/player-export.md), [AI and shorts](product/parity/ai-shorts.md), [states](product/parity/states.md)) | measured values or behaviour |
| Token contracts, Rust owners, measured sources | [UI tokens](product/ui/tokens.md); values in `kit/theme.rs` | - |
| Agent-facing surfaces | [CLI](product/agent-control/cli.md), [MCP](product/agent-control/mcp.md), [In-app agent](product/agent-control/agent.md), [Keyboard](product/agent-control/keyboard.md), [Commands](product/agent-control/commands.md) | crate READMEs |
| Rust safety, structural admission, errors, logging, time and text | [Standards](engineering/standards.md) | - |
| Ticket loop, research, copy imports, upgrades, commits, provisioning summary | [Workflows](engineering/workflows.md) | - |
| Checks, tests, fixtures, evidence, reports | [Verification](engineering/verification.md) | - |
| Licences, FFmpeg distribution, assets, model weights, privacy | [Open-source policy](engineering/open-source.md) | - |
| Crates and edges, the layer of each crate | [Architecture](engineering/architecture.md) | structure.md beyond its own rules |
| Inside a crate: folders, layers, naming | [Structure](engineering/structure.md), `xtask/modules.toml` | standards.md beyond its link |
| Contract of one module | its folder README, or the crate README for a single-file module | architecture.md beyond the index row |
| Why a decision was made | [ADRs](adr/README.md) | owner docs |
| Domain model, glossary and ADR conventions | [Domain docs](agents/domain.md) | - |
| Domain term | [Glossary](../GLOSSARY.md) | - |
| Agent roles, routing, dispatch, review, merge, cleanup | [Orchestration](agents/orchestration.md) | - |
| Ticket files, header, queries | [Issue tracker](agents/issue-tracker.md) | - |
| Build and provisioning detail | [xtask](../xtask/README.md) | workflows.md beyond its summary and link |
| README media provenance, regeneration and component licences | [README media](assets/readme/README.md) | root README slots beyond the two filled |
| Why the README demo uses Big Buck Bunny | [ADR 0007](adr/0007-big-buck-bunny-documentation.md) | asset README |
| Host, worker tooling, ticket state | `.scratch/` and the private workspace | any tracked file |

Owner docs state current contracts only, in short sentences. Change a fact at its owner and link to it elsewhere. No history, test inventories or restated code; rationale belongs in ADRs.

The root README shows the finished product to users: features with media, install, links. Status, plans and developer mechanics live in `.scratch/` and CONTRIBUTING.
