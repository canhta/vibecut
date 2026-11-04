# VibeCut

Open-source CapCut alternative: a Rust desktop video editor for macOS and Windows.

## Read when

| When you | Read |
|---|---|
| judge scope, priority or acceptance | [requirements](docs/product/requirements.md) |
| add a crate, dependency edge or contract | [architecture](docs/engineering/architecture.md) |
| write Rust | [standards](docs/engineering/standards.md), then the README of each crate you touch |
| start, research, copy, upgrade or commit | [workflows](docs/engineering/workflows.md) |
| test, benchmark or report | [verification](docs/engineering/verification.md) |
| add a dependency, asset, model weight or remote AI call | [open-source policy](docs/engineering/open-source.md) |
| dispatch, review or merge agent work | [orchestration](docs/agents/orchestration.md) |
| name a domain concept or revisit a decision | [glossary](GLOSSARY.md), [ADRs](docs/adr/README.md) |

## Rules

- Quality and stability first, then light and fast.
- Research with Context7 first, then official docs; mark anything unchecked unverified.
- Use the latest stable versions. Replace in place: one version of everything, superseded code deleted in the same change.
- One owner per fact; link to it elsewhere.
- Label claims observed, source-documented, inferred, proposed or unverified. A green build proves only the build.
- Keep reports local and hand over paths.
- Secrets and private files live in git-ignored `.privates/`; their contents stay out of output, logs and commits.

## Agent skills

### Issue tracker

Local markdown in `.scratch/`. See [issue tracker](docs/agents/issue-tracker.md).

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/`. See [domain docs](docs/agents/domain.md).
