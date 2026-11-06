# Workflows

## Development loop

1. Read the ticket, milestone map, [domain docs](../agents/domain.md) and each touched crate's README. Identify the owner of every piece to change.
2. Research API, crate, codec and provider facts: Context7 first, then official docs, then upstream source. Cite sources in the report; ask the coordinator when a missing fact blocks correctness.
3. Learn selectively from read-only references in the main checkout's `.spikes/repos/`: the core reference for structure/core, the ticket's editor references for UI/UX and features. UI decisions follow [UI design](../product/ui/index.md); imports follow the policy below.
4. Implement one ticket through existing owners, re-reading shared files before editing to retain concurrent changes. Update owning docs in the same change.
5. Run [verification](verification.md); finish when every Verify item has a command and result.

Commits use a Conventional Commits subject and `git commit -s`; ticket placement and branches follow [orchestration](../agents/orchestration.md#placement-and-builds).

## Copy imports

Copy only needed reference files. First commit: mechanical import with our names, unused features and provenance comments removed. Later commits: adaptations. Both pass [checks](verification.md#checks), including the name gate. The maintainer settles licensing directly with owners; copied code carries no source names or notices.

## Upgrade policy

Use latest stable Rust, crates, Actions, APIs and models. Look up current releases of dependencies added or touched (`cargo info`, crates.io, official releases) and record the lookup; manifests/lockfiles own versions. Replace in place with one version of every path and superseded code removed. Persisted-format changes bump schema and refuse older files with a typed error, retaining user data.

Document accepted third-party limitations. Only an unreleased upstream crash/security fix permits a removable backport, per [ADR 0002](../adr/0002-desktop-media-and-time.md).

## FFmpeg provisioning

`cargo xtask ffmpeg` resolves and builds latest stable FFmpeg from verified source, logs the version and admits only targets matching the running host; distribution obligations live in [open-source policy](open-source.md#ffmpeg-distribution).
Platform toolchains, the static zlib/PNG build, Opus/libvpx, Windows MSYS2, bindgen arguments, Sherpa and `runtime-dlls` projection belong to [xtask's FFmpeg section](../../xtask/README.md#ffmpeg).
External `ffmpeg`/`ffprobe` remain separately installed tools for oracle admission ([verification](verification.md#checks)).

## Hosted CI

`ci.yml` (`CI`) runs only on manual dispatch, with read-only permissions; it is informational: local [checks](verification.md#checks) are the merge gate. Ubuntu docs/lint run fmt and repository policy; a `test (linux-x64)` job provisions FFmpeg and runs Clippy then workspace tests/doctests; one Apple Silicon `test (macos-arm64)` job runs the same checks sharing native setup and target tree. Windows is advisory. `gate` fails closed for failed, cancelled or skipped jobs. Superseded runs cancel per ref.

Cargo/native/fixture keys omit branches. Native jobs share `Swatinem/rust-cache` with an explicit key; only successful full-workspace Clippy/test producers save it, media-only jobs restore. Environment/manifest hashes cover compiler, lockfile, target environment/configuration and OS/architecture. Ticket branches restore main caches; main cannot restore sibling ticket caches.

Sherpa's native prebuilt directory under `target/` has a separate OS/architecture,
lockfile and toolchain cache, saved before Rust cache cleanup. That cleanup removes
non-Cargo files even when named as an extra cache directory. On a missing native
cache, remove its generated directory (old Rust caches retain empty library
directories) and clean `sherpa-onnx-sys` so neither the upstream directory-existence
check nor a restored build fingerprint can retain missing libraries. Its next
build provisions the native archive.

Native media cache keys use resolved FFmpeg/Opus/libvpx/zlib/x264 versions plus recipe/patch/toolchain hashes and platform, with exact matches. Provisioning resolves stable tags even on hits, saves successful native builds before later failures, and skips source-build-only macOS tools on hits. Synthetic fixture keys follow `crates/media/tests/`; rendered outputs are uncached. Log exact-hit receipts in logs/step summaries.

Release/media/render dispatches set `require-oracles`; ordinary manual CI also requires oracles, which the macOS job installs with Homebrew and the Linux job with the distribution `ffmpeg` package, with Windows following the input. Oracle admission and evidence follow [verification](verification.md#checks).

## Distribution and release

Every main push and manual `build.yml` dispatch builds unsigned macOS arm64, x86_64 and universal packages plus advisory Windows x64/arm64 packages. Linux builds the same native binaries and packages `VibeCut-<version>-linux-<arch>.tar.gz` with the bundled LGPL libraries, desktop entry and `install.sh`; `.deb`, `.rpm` and AppImage packaging remain open. Older runs of that ref cancel. A green main run refreshes the rolling `nightly` pre-release, tag, SHA-256 sums and commit-list notes; branch dispatches only upload short-lived workflow artifacts. Each native row reuses `setup-build`; smoke runs on separate VMs without restoring media/Rust caches. Windows packaging remains advisory until its release checks pass ([packaging](../../packaging/README.md#hosted-builds-and-releases)).

Releases are manual: dispatch `release.yml` with the workspace version. Its default `publish=false` signs/notarizes all three macOS DMGs and verifies them on fresh runners (universal on both architectures), without creating a release or tag. `publish=true` requires main and attaches the verified DMGs and sums to one draft, creating `v<version>`; published/immutable releases, mismatched tags and duplicate signed assets are refused. Publishing that draft belongs to the maintainer.

Require `MACOS_CERT_P12` (base64), `MACOS_CERT_PASSWORD`, `APPLE_API_KEY_P8` (raw P8), `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, `APPLE_TEAM_ID` before building signed packages. Remove temporary signing material on success/failure and unconditionally on cancellation; credentials and tool output stay out of logs, following [privacy](open-source.md#privacy). [Packaging](../../packaging/README.md#hosted-builds-and-releases) owns minimum OS, universal assembly, signing, clean-host checks and recovery.

For a local dry run, provision native dependencies, run `cargo xtask ci --require-oracles`, then `cargo xtask dist build` and `cargo xtask dist package` without signing/notary variables, with fresh output directories. It proves ad-hoc staging only; signing/notarization and clean-host acceptance require their release checks. Run `cargo test -p xtask` and `actionlint` for workflow boundaries/syntax.
