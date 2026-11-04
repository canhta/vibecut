# VibeCut

Start with [documentation owners](docs/README.md), [terms](GLOSSARY.md) and [decisions](docs/adr/README.md).

## Read first

| Task | Read |
|---|---|
| Scope or acceptance | [Requirements](docs/product/requirements.md#scope) |
| Dependencies, contracts or headless work | [Architecture](docs/engineering/architecture.md#crate-map) |
| Source structure, safety, timing or reuse | [Standards](docs/engineering/standards.md#safety), [structural admission](docs/engineering/standards.md#structural-admission) and [minimal solutions](docs/engineering/standards.md#minimal-solutions) |
| Working in this repo, tasks or delivery | [Development loop](docs/engineering/workflows.md#development-loop), then the README of any crate you touch |
| Adding/upgrading dependencies or persisted data | [Upgrade policy](docs/engineering/workflows.md#upgrade-policy) |
| Multi-agent work, dispatch, review, merge | [Orchestration](docs/agents/orchestration.md) |
| Commands, tests or completion evidence | [Verification](docs/engineering/verification.md#checks) |
| Licensing, assets or remote AI | [Open-source policy](docs/engineering/open-source.md#licensing) and [privacy](docs/engineering/open-source.md#privacy) |

## Rules

- Product principles: quality and stability first, then light and fast; see [requirements](docs/product/requirements.md#principles).
- Research before deciding; follow the [research rule](docs/engineering/workflows.md#development-loop).
- Prefer the best current design; replace in place under the [upgrade policy](docs/engineering/workflows.md#upgrade-policy).
- Give each fact one authoritative owner; link elsewhere and reconcile its owner in the same change.
- Preserve unrelated work; freshly read shared files before editing them.
- Label evidence observed, source-documented, inferred, proposed or unverified. Plans and passing builds do not prove runtime behavior.
- Keep reports local and hand over paths. Never publish an Artifact.
- Coordinating other agents: follow [orchestration](docs/agents/orchestration.md) exactly: one-line dispatch specs, worker pull loops, workers self-merge, read report TL;DRs only, batch messages, ask the maintainer when unclear.

## Agent skills

### Issue tracker

Local markdown under `.scratch/` (git-ignored, never pushed). See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context: `GLOSSARY.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
