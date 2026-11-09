# Workflows

## Development loop

1. Read the ticket, [domain docs](../agents/domain.md) and the README of each crate you touch.
2. Look up API, crate, codec and provider facts (Context7, then official docs, then upstream source) and cite them in the report. Never guess a fact; ask the coordinator when one blocks you.
3. References are read-only in the main checkout's `.spikes/repos/`. UI follows [UI design](../product/ui/index.md).
4. Change code through its existing owner and update the owning doc in the same commit.
5. Run [verification](verification.md).

Commits: Conventional Commits subject, `git commit -s`. Branches: [orchestration](../agents/orchestration.md#placement-and-builds).

## Copy imports

Copy only the files you need. First commit: the mechanical import, renamed to our names, with unused features and source names removed. Later commits adapt it. Licensing is the maintainer's; never add notices.

## Upgrade policy

- Use the latest stable Rust, crates, Actions, APIs and models; check current releases (`cargo info`, crates.io) for every dependency you add or touch.
- One version of everything: change code in place, update every caller in the same commit and delete what it supersedes. Never add a second function, wrapper, alias, feature flag, `_v2`/`old`/`legacy` name, deprecation path or fallback to old behaviour.
- Greenfield: a persisted-format change bumps its version, with no migration and no second reader. Projects of another schema refuse with a typed error; preferences, caches and recovery journals of another version are dropped. There is no user data to preserve.
- Backport only an unreleased upstream crash or security fix ([ADR 0002](../adr/0002-desktop-media-and-time.md)).

## FFmpeg provisioning

`cargo xtask ffmpeg` builds the latest stable FFmpeg from verified source for the running host. Toolchains and platform details: [xtask](../../xtask/README.md#ffmpeg). Distribution: [open-source policy](open-source.md#ffmpeg-distribution).

## Hosted CI

`ci.yml` runs only on manual dispatch and is informational; local [checks](verification.md#checks) are the merge gate. Cache keys and job layout live in `.github/workflows/`.

## Distribution and release

Every main push builds unsigned packages (`build.yml`) and refreshes the `nightly` pre-release. Releases are manual through `release.yml`; publishing the draft belongs to the maintainer. Signing, notarization and packaging rules: [packaging](../../packaging/README.md#hosted-builds-and-releases).
