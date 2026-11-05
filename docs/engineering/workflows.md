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

Windows needs architecture-matched MSYS2 CLANG64 (x64) or CLANGARM64 (arm64), Bash, Make, Perl (libvpx configure), CMake, Ninja, pkg-config, NASM and Clang/LLVM; Rust uses MSVC. Native xtask caches under `USERPROFILE`, resolves Bash/Make/cygpath to absolute MSYS2 `usr/bin` paths on `PATH` and logs them; incomplete installations are refused rather than using Windows/WSL/Git Bash. Static Opus/libvpx builds request static pkg-config metadata, including private pthread links.

Windows excludes FFmpeg's out-of-contract `gfxcapture` screen filter through configuration: case-insensitive builds resolve libc++'s `<version>` to the archive's `VERSION`. For workspace checks, xtask merges general, underscore-target and hyphen-target `BINDGEN_EXTRA_CLANG_ARGS` in that order, then appends AMX include guards and an explicit native MSVC Clang target in the preferred hyphen-target variable. The target applies to FFmpeg and Whisper: native bindgen otherwise trusts MSYS2 libclang’s GNU default, whose unsigned enums disagree with whisper-rs’s MSVC representation. Workspace Cargo children receive MSVC/SDK header paths in `INCLUDE`, discovered through `cc`'s Windows registry support or inherited from an already-configured developer shell. Forward only `INCLUDE` from that compiler environment to preserve the FFmpeg DLL search path; missing MSVC tools or SDK paths fail with setup guidance. Exclude dependent AMX headers together until upstream bindings handle vector layouts; this avoids invalid packed/aligned tile bindings without changing native FFmpeg or public layouts.

Windows workspace Cargo children also receive the managed static `/MD` Sherpa
library directory through `SHERPA_ONNX_LIB_DIR`, preserving Rust's default dynamic
MSVC CRT. Native AI archive selection, cache integrity and explicit overrides
belong to [ai's native build contract](../../crates/ai/README.md#local-gipformer).

## Hosted CI

`ci.yml` (`CI`) runs on main pushes and manual dispatch, with read-only permissions; ticket branches/PRs use [local checks](verification.md#checks). Require stable `CI gate` in branch protection. Ubuntu docs/lint run fmt and repository policy each time; Markdown/`docs/`-only main pushes stop there. Code/manual runs add one Apple Silicon `test (macos-arm64)` job, Clippy then workspace tests/doctests sharing native setup and target tree, without archive transfers or another macOS runner. `gate` fails closed for failed, cancelled or skipped selected jobs. Windows is advisory and supports manual dispatch. Superseded runs cancel per event/ref, including main bursts; main classifies the event diff, manual runs the full gate.

Cargo/native/fixture keys omit branches. Native jobs share `Swatinem/rust-cache` with an explicit key; only successful full-workspace Clippy/test producers save it, media-only jobs restore. Environment/manifest hashes cover compiler, lockfile, target environment/configuration and OS/architecture. Ticket branches restore main caches; main cannot restore sibling ticket caches.

Sherpa's native prebuilt directory under `target/` has a separate OS/architecture,
lockfile and toolchain cache, saved before Rust cache cleanup. That cleanup removes
non-Cargo files even when named as an extra cache directory. On a missing native
cache, remove its generated directory (old Rust caches retain empty library
directories) and clean `sherpa-onnx-sys` so neither the upstream directory-existence
check nor a restored build fingerprint can retain missing libraries. Its next
build provisions the native archive.

Native LGPL cache keys use resolved FFmpeg/Opus/libvpx versions plus recipe/patch/toolchain hashes and platform, with exact matches. Provisioning resolves stable tags even on hits, saves successful native builds before later failures, and skips source-build-only macOS tools on hits. Synthetic fixture keys follow `crates/media/tests/`; rendered outputs are uncached. Log exact-hit receipts in logs/step summaries.

Release/media/render dispatches set `require-oracles`; ordinary manual CI also requires macOS oracles, with Windows following the input. Oracle admission and evidence follow [verification](verification.md#checks).

## Distribution and release

Every main push and manual `build.yml` dispatch builds unsigned macOS arm64, x86_64 and universal packages plus advisory Windows x64/arm64 packages. Older runs of that ref cancel. A green main run refreshes the rolling `nightly` pre-release, tag, SHA-256 sums and commit-list notes; branch dispatches only upload short-lived workflow artifacts. Each native row reuses `setup-build`; smoke runs on separate VMs without restoring media/Rust caches. Windows remains advisory until p0-core/44.

Releases are manual: dispatch `release.yml` with the workspace version. Its default `publish=false` signs/notarizes all three macOS DMGs and verifies them on fresh runners (universal on both architectures), without creating a release or tag. `publish=true` requires main and attaches the verified DMGs and sums to one draft, creating `v<version>`; published/immutable releases, mismatched tags and duplicate signed assets are refused. Publishing that draft belongs to the maintainer.

Require `MACOS_CERT_P12` (base64), `MACOS_CERT_PASSWORD`, `APPLE_API_KEY_P8` (raw P8), `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, `APPLE_TEAM_ID` before building signed packages. Remove temporary signing material on success/failure and unconditionally on cancellation; credentials and tool output stay out of logs, following [privacy](open-source.md#privacy). [Packaging](../../packaging/README.md#hosted-builds-and-releases) owns minimum OS, universal assembly, signing, clean-host checks and recovery.

For a local dry run, provision native dependencies, run `cargo xtask ci --require-oracles`, then `cargo xtask dist build` and `cargo xtask dist package` without signing/notary variables, with fresh output directories. It proves ad-hoc staging only; signing/notarization and clean-host acceptance require their release checks. Run `cargo test -p xtask` and `actionlint` for workflow boundaries/syntax.
