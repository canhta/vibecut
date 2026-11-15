# Native distribution

`cargo xtask dist` owns native app/CLI builds, packages, signing, universal assembly, source compliance, clean-host smoke and rolling nightly/draft assets. It stages real Cargo executables, dynamically linked FFmpeg, corresponding source archives, recipes, backports and license/asset notices. Version comes from the workspace package. No Python, model weights or extra codec binaries are used. Normal builds consume committed icons; `cargo xtask dist icons` regenerates them.

Linux ships as `vibecut-<version>-linux-<arch>.tar.gz`: `bin/` with `vibecut` and `vibecut-cli`, `lib/` with the bundled FFmpeg libraries (GPL build), `compliance/`, a desktop entry, icon and `install.sh` (default prefix `~/.local`). The binaries carry an `$ORIGIN/../lib` runpath, so extracting the tarball is enough to run it. No `.deb`, `.rpm` or AppImage is built.

## Local build and stage

Run Rust commands through the shared build wrapper in managed worktrees. Provision with `cargo xtask ffmpeg`, then read its prefix from `~/.cache/vibecut/ffmpeg/<os>-<arch>/current` (native Windows uses `USERPROFILE`). Keep the prefix and its sibling `sources/` directory.

```sh
cargo xtask build -- cargo xtask ffmpeg
PREFIX=$(cat "$HOME/.cache/vibecut/ffmpeg/macos-aarch64/current")
cargo xtask build -- cargo xtask dist build --ffmpeg "$PREFIX" --output target/package-inputs
cargo xtask build -- cargo xtask dist package --os macos --arch aarch64 \
  --binaries target/package-inputs --ffmpeg "$PREFIX" --output target/package-macos
```

`dist build` collects the two executable paths from Cargo JSON records, reuses the native media environment (including Windows bindgen guards), and records revision, profile and Cargo arguments. Release is the default; `--debug` is for local smoke. Output directories must be new. Failed output stays for inspection and must never be distributed. Packaging and auditing run on the target OS. `macos-app.sh` forwards to `dist package --os macos`.

The macOS `.pkg` contains `/Applications/VibeCut.app` and the CLI symlink under `/usr/local/bin`. Signed releases distribute a DMG with the app, Applications link and `install-cli.sh`, which adds `~/.local/bin/vibecut-cli`. Windows ships `vibecut-<version>-windows-<arch>.msi` and a portable `vibecut-<version>-windows-<arch>.zip` for `x86_64` and `aarch64`. The ZIP's `VibeCut/` folder runs in place and changes nothing outside itself. The MSI installs the same folder per machine under `Program Files\VibeCut`, adds a Start menu shortcut and appends the folder to the machine `PATH` so `vibecut-cli` resolves in new terminals; uninstall from Settings or `msiexec /x` removes all three. No platform's uninstall deletes projects or preferences.

### Windows installer choice

The installer is an MSI built by [WiX](https://github.com/wixtoolset/wix) 5.0.2 from `packaging/windows/vibecut.wxs`, chosen over MSIX for these reasons:

- **Library replacement.** LGPL FFmpeg must stay replaceable by the user. An MSI installs ordinary files that an administrator can swap beside the EXEs. MSIX installs into the read-only, package-sealed `WindowsApps` directory, where any replaced DLL breaks the package.
- **Unsigned installs.** Windows refuses an unsigned MSIX unless developer mode or the `-AllowUnsigned` path is set up, so without a certificate it could neither ship nor pass the clean-runner smoke. An unsigned MSI installs, with a SmartScreen prompt, and signing it later changes nothing else.
- **CLI and agents.** MSIX runs the app with a virtualised file system and registry and reaches the CLI only through an App Execution Alias. The MSI's real `PATH` entry and plain install folder give agents and scripts the same `vibecut-cli` the portable ZIP has.
- **Toolchain.** WiX 5.0.2 is MS-RL, harvests the audited payload with one `Files` glob and builds x64 and ARM64 packages on their native runners. Later WiX releases gate every build on accepting the Open Source Maintenance Fee EULA, an outward commitment for the maintainer to make, so the version is pinned. The MSIX tools would add Windows SDK packaging tools without these benefits.

Lost against MSIX: built-in auto-update, a clean per-user container and Store distribution. Updates reinstall a newer MSI, and `MajorUpgrade` replaces the older version in place.

On Windows use matching MSVC/Windows SDK developer tools and architecture-matched MSYS2 CLANG64 or CLANGARM64, as described in [provisioning](../docs/engineering/workflows.md#ffmpeg-provisioning). Use `--os windows --arch x86_64` or `aarch64`. Packaging needs `wix` 5.0.2 on `PATH` (`dotnet tool install --global wix --version 5.0.2`). The PE audit reads each image's machine, subsystem and import directory and rejects wrong architectures and any DLL that is neither bundled nor in System32; FFmpeg cannot be satisfied from System32. The recipe links zlib and winpthreads statically, so the FFmpeg DLLs import only system DLLs and each other. Shared libraries live beside both EXEs. A successful build does not establish Windows install, signing or runtime acceptance.

## Hosted builds and releases

`build.yml` runs on every main push and manual dispatch, cancelling older runs of the same ref. `distribution.yml` builds native macOS arm64 and x86_64 (`macos-15-intel`), merges both into universal, and builds native Windows x64 (`windows-latest`) and arm64 (`windows-11-arm`); every row is required. Each native row provisions FFmpeg through `setup-build` and its versioned cache. Distributions expire after seven days; intermediate native inputs after three.

Packages move to separate fresh VMs without Rust, media caches or media provisioning. These VMs run an architecture-matched standalone Rust xtask helper from the build VM, then install the downloaded package. The installed CLI runs `--version`, creates a 64×64 two-second synthetic overlay project through the registry, exports MP4, and checks its duration and video stream. The app runs `--headless-smoke NEW_DIRECTORY`, creates its workspace and editor and draws three egui frames without a window, graphics device or audio output. Universal packages run on both Apple Silicon and Intel. Each Windows VM smokes the extracted portable ZIP in place, then installs the MSI silently, smokes the installed copy, uninstalls it and requires the install folder, Start menu shortcut and machine `PATH` entry to be gone; `msiexec` logs land in the smoke artifact. This proves startup, frontend and export behaviour only; native GPU/device behaviour needs separate evidence.

The macOS deployment floor is 11.0. `dist build` sets `MACOSX_DEPLOYMENT_TARGET`, the bundle sets `LSMinimumSystemVersion`, FFmpeg's recipe targets 11.0, and packaging audits every Mach-O slice's exact architecture and deployment load command. Runtime on macOS 11 is a separate check; hosted VMs run newer OSes. `dist export` retains native executables, dylibs and verified source SDKs. Universal assembly refuses mismatched build records, source identities or dylib sets, relocates native inputs before `lipo`, and keeps both architecture build receipts. Every merged dylib, executable and bundle is signed inside out; `lipo -info` and `codesign --verify --deep --strict` are mandatory for every Mach-O.

A green main build replaces the rolling `nightly` pre-release's unsigned packages and `SHA256SUMS`. Notes list commits since the previous nightly with a full compare link. Publication refuses stale main heads, diverged history, and stable or immutable releases. Failed uploads do not move an existing nightly tag or delete stale assets. Asset replacement is not transactional: an interrupted upload can replace some assets, so reconcile or rerun the failed build before trusting the set. Branch dispatches only upload workflow artifacts.

The maintainer manually dispatches `release.yml` with the workspace version. `publish=false` (default) is the dry run: it signs and notarizes all three macOS DMGs, builds both Windows MSIs and ZIPs, and runs clean-host smokes without creating a release or version tag. `publish=true` requires main, creates `v<version>` and attaches the verified DMGs, MSIs, ZIPs and `SHA256SUMS` to one draft; every build row also uploads a `<package>.sha256` beside its package. Existing published or immutable releases, mismatched tags and duplicate signed assets are refused. Publishing the draft is the maintainer's action.

## Signing and verification

Hosted signing requires `MACOS_CERT_P12` (base64), `MACOS_CERT_PASSWORD`, `APPLE_API_KEY_P8` (raw P8), `APPLE_API_KEY_ID`, `APPLE_API_ISSUER` and `APPLE_TEAM_ID`. The early `dist signing validate` preflight imports and verifies the certificate before media provisioning or workspace builds, then removes its private material. `dist signing` imports one Developer ID Application identity into an ephemeral keychain and restores the original keychain search list on success or failure. The workflow calls `dist signing cleanup` after cancellation. Private files are owner-only; credentials, identities and tool output stay out of logs.

For a local package with identities already in the keychain, use `VIBECUT_MAC_SIGN_IDENTITY` and optional `VIBECUT_MAC_INSTALLER_IDENTITY`, plus `VIBECUT_NOTARY_PROFILE` or the key-file/key-ID/issuer variables. Unsigned staging uses ad-hoc signatures. Developer ID enables hardened runtime and timestamps, notarizes and staples the app before sealing the DMG, then signs, notarizes and staples the image. `Accepted`, code-signature verification, staples and Gatekeeper assessments are mandatory. An unsigned `.pkg` is only a local-test artifact.

Notarization failures fetch Apple's submission log before credential cleanup and report only numeric status codes and fixed diagnostic categories. Stapling and validation retry ticket-service exit 68 up to six times within three minutes per operation; other failures stop immediately. Retries never substitute for an Accepted submission or successful ticket validation.

Windows signing reads `VIBECUT_WINDOWS_CERT_PFX` (base64 PFX), `VIBECUT_WINDOWS_CERT_PASSWORD` and `VIBECUT_WINDOWS_TIMESTAMP_URL`; release runs map them from the `WINDOWS_CERT_PFX`, `WINDOWS_CERT_PASSWORD` and `WINDOWS_TIMESTAMP_URL` secrets and put the Windows SDK's SignTool on `PATH`. With a certificate, packaging writes the PFX to an owner-only runner-temporary file, signs and verifies (`signtool verify /pa`) every DLL, both EXEs and the MSI, then deletes the file. Without `VIBECUT_WINDOWS_CERT_PFX` it builds unsigned packages and prints a `Windows signing skipped` warning (a workflow annotation on hosted runners); a certificate without its password or timestamp URL fails. Nightly builds never receive these secrets. Certificate acquisition and Windows release policy stay with the maintainer.

```sh
cargo xtask build -- cargo xtask dist smoke --installed target/package-macos/root/Applications/VibeCut.app \
  --output target/package-smoke
```

The default smoke opens no window. `--signed` also checks the app signature, Gatekeeper and staple. `--observe` runs the native eight-second observation and screenshot path; request the maintainer's screen before using it locally. Its signed macOS mode records actual mapped FFmpeg paths with `lsof`. `--media PATH` probes an external synthetic fixture before the generated export. On clean Windows, `dist install-smoke` covers the ZIP and MSI as described above; to inspect module paths by hand, run the installed app and read PowerShell `Get-Process`/`Modules`. All loaded FFmpeg DLLs must stay beside the EXEs, including from unrelated working directories. MCP stdio installation belongs to [MCP control](../docs/product/agent-control/mcp.md); no passing MCP smoke is claimed. The receipt keeps `release_acceptance: false`: these checks grant neither maintainer release approval nor native device/hardware acceptance.

## Library replacement and recovery

FFmpeg is LGPL-2.1-or-later on macOS and Windows and GPL-3.0 with libx264 on Linux; reverse engineering to debug library changes is permitted. Included FFmpeg/Opus/libvpx/LAME/zlib archives match the verified recipe, receipt and patches. Extract `.archive` files with `tar -xf` and rebuild ABI-compatible shared libraries in an isolated prefix. No app rebuild is required. Keep the original libraries to undo replacement and never modify the only copy of an installed app.

On macOS, copy the app, replace architecture/ABI-compatible dylibs under `Contents/Frameworks`, preserve `@rpath/<filename>` IDs and `@loader_path`, and re-sign inside out with an ad-hoc identity. `cargo xtask dist replace --app-copy COPY.app --libraries PREFIX/lib` does the replacement, relocation and re-signing for original runtime filenames. The copy loses its notarized identity. Rerun the smoke and inspect loaded paths. On Windows close the app, back up and replace compatible DLLs beside both EXEs, then rerun smoke and module inspection; restore the backups to recover.

Failures refuse distribution and leave `receipt.json` with safe command records. Fix the input and rerun with a fresh output. Inspect notary rejections with `xcrun notarytool log`; retry stapling only after an Accepted submission. Never claim signing, notarization or clean-host acceptance from an unsigned build. Uninstall only verified app-owned CLI links and preserve project and preference data.

Contracts: [Apple signing](https://developer.apple.com/documentation/xcode/using-the-latest-code-signature-format), [notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), [Windows DLL search](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order), [SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).
