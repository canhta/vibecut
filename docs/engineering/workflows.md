# Workflows

## Development loop

1. **Read** the ticket, its milestone `map.md`, the glossary, ADRs for the area and
   the README of every crate you touch. Done when you can name the owner of each
   piece you will change.
2. **Research** every API, crate, codec or provider fact you rely on: Context7 first,
   then official docs, then upstream source. Cite sources in your report; ask the
   coordinator when a missing fact blocks correctness.
3. **Learn from references** in `.spikes/repos/` (main checkout): structure and core
   from the core reference, UI/UX and features from the editor references the ticket
   names. Copy useful code under [copy imports](#copy-imports).
   UI work follows [UI design](../product/ui.md). Choose selectively: for each
   layout, interaction, shortcut and empty/loading/error state, compare how the
   references (code, and screenshots in `.spikes/ui-reference/`) solve it, take the
   best fit for our users and cite it with the reason. A design with no reference
   basis is marked new and needs coordinator approval before you build it.
4. **Implement** one ticket. Reuse the existing owner of any logic; re-read shared
   files right before editing them and keep other workers' changes intact. Update the
   owning docs in the same change.
5. **Verify** with the [smallest covering check](verification.md#checks), then the evidence
   [verification](verification.md) asks for. Done when every Verify item has a
   command and a result.

Commits: conventional subject, `git commit -s`.

## Copy imports

Copy only the files the ticket needs from a read-only reference. First commit:
mechanical import with our names, unused features removed and provenance comments
dropped. Later commits: adaptations. Both pass [checks](verification.md#checks), including the name
gate. Copied code carries no source names or notices; the maintainer settles
licensing with the owners directly.

## Upgrade policy

Use the latest stable Rust, crates, Actions, APIs and models. When adding or
touching a dependency, look up its current release (`cargo info`, crates.io,
official releases) and record the lookup in the report; versions live in manifests
and lockfiles.

Replace in place: one version of every path, superseded code deleted in the same
change. A persisted-format change bumps the schema version and refuses older files
with a typed error; user data is never deleted.

Accepted third-party limitations are documented, not patched. The one exception is
an upstream crash or security fix not yet released: backport it as a removable
patch per [ADR 0002](../adr/0002-desktop-media-and-time.md).

## FFmpeg provisioning

`cargo xtask ffmpeg` builds the latest stable LGPL FFmpeg from verified source and
logs the resolved version; keep the sources and receipt with every distributed
binary. Windows builds need MSYS2 CLANG64 (x64) or CLANGARM64 (arm64) with Bash,
Make, CMake, Ninja, pkg-config, NASM and matching Clang/LLVM, run on the matching
architecture; Rust keeps its MSVC toolchain. Native xtask uses `USERPROFILE`
for its Windows cache and resolves Bash, Make and cygpath to absolute paths in the
MSYS2 `usr/bin` on `PATH`, refusing an incomplete installation instead of using
Windows/WSL or Git Bash. Provisioning logs the selected tool paths. The Windows FFmpeg configure step
requests static pkg-config metadata for its static Opus/libvpx dependencies,
including libvpx’s private pthread link dependencies. The Windows recipe excludes
FFmpeg's `gfxcapture` screen-recording source filter (outside the editor's input
contract): its C++ build currently resolves libc++'s `<version>` include to the
source archive's `VERSION` file on case-insensitive filesystems. This upstream
limitation is excluded through configuration, not a source patch.
For Windows workspace checks, `xtask` also appends AMX intrinsic include guards to
`BINDGEN_EXTRA_CLANG_ARGS` for the native Windows target. It merges caller
arguments from the general, underscore-target and hyphen-target forms in that order,
then appends the guards in the preferred hyphen-target variable. This avoids bindgen's invalid packed/aligned tile representation
without changing the native FFmpeg build or its public layouts. The dependent AMX
headers are excluded together; remove this configuration when the upstream bindings
handle those vector layouts.

## Hosted CI

Pushes to `main`, `canhta/**` and `feat/**`, and pull requests run CI with read-only repository permissions.
Require the stable `CI gate` status in branch protection. `ci.yml` (`CI`) runs
`docs` on every change; Markdown/`docs/`-only diffs stop there. Rust changes also
run `lint` alongside a single Apple Silicon `macos-latest` test archive build.
Three `test (macos-arm64, N/3)` runners execute disjoint nextest count partitions
from that archive with required oracles; doctests run on the archive builder.
`gate` fails closed if any selected job or partition fails, cancels or skips.
Windows runs as an advisory check until its runtime is in scope.
Superseded PR and ticket-branch runs cancel per branch; main pushes finish.
Ticket pushes always run the full macOS required-oracle gate, including docs-only
pushes. Cargo, native media and synthetic media-fixture cache keys omit the branch
so ticket runs can restore default-branch caches. Fork PRs use `pull_request`, never
`pull_request_target`, and receive no release secrets. The PR gate requires a Signed-off-by trailer on every submitted commit,
as described in [contributing](../../CONTRIBUTING.md).

The shared native setup uses `Swatinem/rust-cache` with an explicit shared key
across build/lint/release jobs; its environment/manifest hash includes the Rust
compiler, lockfile, target environment and configuration, with OS/architecture
separation. GitHub permits ticket branches to restore main's caches, but main
cannot restore sibling ticket caches. Native LGPL FFmpeg is keyed by resolved
FFmpeg/Opus/libvpx versions plus recipe/patch/toolchain/setup hashes and platform,
with no date or stale-prefix fallback. Provisioning always resolves stable upstream
tags, including after a cache hit, and saves successful native builds before
later checks can fail. Media input fixtures are keyed by their owning generator
(`crates/media/tests/common/`); generated render outputs are not cached.
Nextest 0.9.151 is installed from checksum-verified upstream binaries through the
shared setup action. Archive runners restore only native/fixture caches; they do
not compile the test suite again. CI cache receipts report exact hits in logs and
step summaries. The manual media-dependencies workflow warms the same
cache and retains source/recipe archives.

Release and media/render dispatches set the `require-oracles` input: CI installs
external FFmpeg and runs the required mode from [verification](verification.md#checks).
The ordinary dispatch also requires macOS oracles; Windows follows the input.

A `v*` tag matching the workspace version in this repository starts the macOS
Apple Silicon release workflow. It requires `MACOS_CERT_P12` (base64),
`MACOS_CERT_PASSWORD`, `APPLE_API_KEY_P8` (raw P8), `APPLE_API_KEY_ID`,
`APPLE_API_ISSUER` and `APPLE_TEAM_ID`; absent secrets fail before building.
A temporary keychain is removed on success/failure, with an unconditional cleanup
step for cancellation. No signing credentials or tool output reach logs. The
[packaging owner](../../packaging/README.md) defines nested signing, notarization,
stapling, platform verification and failure recovery. Only the signed DMG moves
on to a separate fresh hosted macOS VM, which installs it and runs app/CLI smoke
without restoring build/media caches. Only then does the workflow attach the DMG
to a **draft** GitHub release; existing published releases and assets are refused.
Publishing the draft and pushing release tags require maintainer approval.

For an act-free local release dry run, provision the native dependencies, run
`cargo xtask ci --require-oracles`, build with `packaging/build.py`, and stage with
`packaging/package.py` without signing/notary environment variables, using a fresh
output directory each time. This stops at ad-hoc staging before Developer ID
signing; it proves neither notarization nor clean-host acceptance. Run
`python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v` and `actionlint`
for workflow boundary and syntax checks. Do not push a test tag.
