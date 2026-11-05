# Workflows

## Development loop

1. Read the ticket, milestone map, [domain docs](../agents/domain.md) and each touched crate's README. Identify the owner of every piece to change.
2. Research API, crate, codec and provider facts: Context7 first, then official docs, then upstream source. Cite sources in the report; ask the coordinator when a missing fact blocks correctness.
3. Learn selectively from read-only references in the main checkout's `.spikes/repos/`: the core reference for structure/core, the ticket's editor references for UI/UX and features. UI decisions follow [UI design](../product/ui.md); imports follow the policy below.
4. Implement one ticket through existing owners, re-reading shared files before editing to retain concurrent changes. Update owning docs in the same change.
5. Run [verification](verification.md); finish when every Verify item has a command and result.

Commits use a Conventional Commits subject and `git commit -s`; ticket placement and branches follow [orchestration](../agents/orchestration.md#tickets-and-placement).

## Copy imports

Copy only needed reference files. First commit: mechanical import with our names, unused features and provenance comments removed. Later commits: adaptations. Both pass [checks](verification.md#checks), including the name gate. The maintainer settles licensing directly with owners; copied code carries no source names or notices.

## Upgrade policy

Use latest stable Rust, crates, Actions, APIs and models. Look up current releases of dependencies added or touched (`cargo info`, crates.io, official releases) and record the lookup; manifests/lockfiles own versions. Replace in place with one version of every path and superseded code removed. Persisted-format changes bump schema and refuse older files with a typed error, retaining user data.

Document accepted third-party limitations. Only an unreleased upstream crash/security fix permits a removable backport, per [ADR 0002](../adr/0002-desktop-media-and-time.md).

## FFmpeg provisioning

`cargo xtask ffmpeg` resolves/builds latest stable LGPL FFmpeg from verified source and logs the version. Distribution obligations live in [open-source policy](open-source.md#ffmpeg-distribution).

Windows needs architecture-matched MSYS2 CLANG64 (x64) or CLANGARM64 (arm64), Bash, Make, CMake, Ninja, pkg-config, NASM and Clang/LLVM; Rust uses MSVC. Native xtask caches under `USERPROFILE`, resolves Bash/Make/cygpath to absolute MSYS2 `usr/bin` paths on `PATH` and logs them; incomplete installations are refused rather than using Windows/WSL/Git Bash. Static Opus/libvpx builds request static pkg-config metadata, including private pthread links.

Windows excludes FFmpeg's out-of-contract `gfxcapture` screen filter through configuration: case-insensitive builds resolve libc++'s `<version>` to the archive's `VERSION`. For workspace checks, xtask merges general, underscore-target and hyphen-target `BINDGEN_EXTRA_CLANG_ARGS` in that order, then appends AMX include guards in the preferred hyphen-target variable. Exclude dependent AMX headers together until upstream bindings handle vector layouts; this avoids invalid packed/aligned tile bindings without changing native FFmpeg or public layouts.

## Hosted CI

`ci.yml` (`CI`) runs on main pushes and manual dispatch, with read-only permissions; ticket branches/PRs use [local checks](verification.md#checks). Require stable `CI gate` in branch protection. Ubuntu docs/lint run fmt and repository policy each time; Markdown/`docs/`-only main pushes stop there. Code/manual runs add one Apple Silicon `test (macos-arm64)` job, Clippy then workspace tests/doctests sharing native setup and target tree, without archive transfers or another macOS runner. `gate` fails closed for failed, cancelled or skipped selected jobs. Windows is advisory and supports manual dispatch. Superseded runs cancel per event/ref, including main bursts; main classifies the event diff, manual runs the full gate.

Cargo/native/fixture keys omit branches. Native jobs share `Swatinem/rust-cache` with an explicit key; only successful full-workspace Clippy/test producers save it, media-only jobs restore. Environment/manifest hashes cover compiler, lockfile, target environment/configuration and OS/architecture. Ticket branches restore main caches; main cannot restore sibling ticket caches.

Native LGPL cache keys use resolved FFmpeg/Opus/libvpx versions plus recipe/patch/toolchain hashes and platform, with exact matches. Provisioning resolves stable tags even on hits, saves successful native builds before later failures, and skips source-build-only macOS tools on hits. Synthetic fixture keys follow `crates/media/tests/`; rendered outputs are uncached. Log exact-hit receipts in logs/step summaries.

Release/media/render dispatches set `require-oracles`; ordinary manual CI also requires macOS oracles, with Windows following the input. Oracle admission and evidence follow [verification](verification.md#checks).

## Release

Releases are manual: the maintainer runs `release.yml` from Actions on `main` with the workspace version. It builds macOS Apple Silicon and creates `v<version>` on a draft release. Require `MACOS_CERT_P12` (base64), `MACOS_CERT_PASSWORD`, `APPLE_API_KEY_P8` (raw P8), `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, `APPLE_TEAM_ID` before build. Remove the temporary keychain on success/failure and unconditionally on cancellation; keep signing credentials and tool output out of logs, following [privacy](open-source.md#privacy). The [packaging owner](../../packaging/README.md) defines signing, notary, stapling, platform verification and recovery.

Only the signed DMG goes to a fresh hosted macOS VM for install/app/CLI smoke without build/media caches. Only after that may the workflow attach it to a draft release; existing published releases/assets are refused. Running the release workflow and publishing the draft belong to the maintainer. Every main merge also runs `build.yml`, an unsigned test build refreshing the rolling `nightly` pre-release.

For a local dry run, provision native dependencies, run `cargo xtask ci --require-oracles`, then `packaging/build.py` and `packaging/package.py` without signing/notary variables, with fresh output directories. It proves ad-hoc staging only; signing/notarization and clean-host acceptance require their release checks. Run `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v` and `actionlint` for workflow boundaries/syntax.
