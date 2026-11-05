# Native distribution

`package.py` stages real desktop `vibecut` and headless `vibecut-cli` artifacts,
FFmpeg shared libraries, corresponding sources, build receipt/recipe, backports,
licences and asset notices. Version comes only from `[workspace.package]`.
`icons.sh` regenerates assets; it is not a distribution script. Both native hosts
need Python 3.11+ and their platform SDK tools. No models are bundled.

The macOS result is an installer `.pkg` containing `/Applications/VibeCut.app`
and `/usr/local/bin/vibecut-cli`. The Windows result is a ZIP containing a
per-user PowerShell installer and uninstaller. Installation adds the application
directory to the user's PATH; restart the terminal. Neither installer deletes
projects or preferences. The packaging output directory must not exist: failed
outputs remain for inspection and must never be distributed.

## Build and stage

From the repository root on the target OS/architecture, provision the media
backend with `cargo xtask ffmpeg` through the shared build wrapper. Read the
resolved prefix from `~/.cache/vibecut/ffmpeg/<os>-<arch>/current` (Windows uses
USERPROFILE when HOME is absent). Keep that prefix and its sibling `sources/`.
The following macOS example uses the wrapper required in managed worktrees:

```sh
cargo xtask build -- cargo xtask ffmpeg
PREFIX=$(cat "$HOME/.cache/vibecut/ffmpeg/macos-aarch64/current")
cargo xtask build -- python3 packaging/build.py --ffmpeg "$PREFIX" --output target/package-inputs
python3 packaging/package.py --os macos --arch aarch64 \
  --binaries target/package-inputs --ffmpeg "$PREFIX" --output target/package-macos
cargo xtask build -- cargo xtask ci --require-oracles
```

`build.py` collects executable paths from Cargo's JSON artifact records and names
outputs by package role. It never guesses a shared-cache path. `--debug` is only
for local smoke; release is the default. The binary source build record accompanies
the inputs; retain it with the final package receipt. Use `x86_64` on an Intel Mac.
Run `python3 -m unittest discover -s packaging -p 'test_*.py' -v` for source-integrity,
licence, dependency, runpath, architecture and credential-redaction boundary checks.
`macos-app.sh` forwards these staging arguments to `package.py --os macos`.

On native Windows, in the matching MSVC/Windows SDK developer shell, provision
FFmpeg using the documented MSYS2 toolchain in [workflows](../docs/engineering/workflows.md#ffmpeg-provisioning).
Native Windows runs Rust compilation directly; the shared build wrapper is Unix-only:

```powershell
$Prefix = Get-Content "$env:USERPROFILE\.cache\vibecut\ffmpeg\windows-x86_64\current" -Raw
$Prefix = $Prefix.Trim()
python packaging/build.py --ffmpeg $Prefix --output target/package-inputs
python packaging/package.py --os windows --arch x86_64 --binaries target/package-inputs --ffmpeg $Prefix --output target/package-windows
Expand-Archive target/package-windows/VibeCut-0.1.0-windows-x86_64.zip target/install-test
powershell -NoProfile -File target/install-test/VibeCut/install.ps1
```

Choose `aarch64` on Windows ARM64. PE machine/subsystem checks refuse mismatched
app, CLI or DLL inputs. `dumpbin /dependents` audits every executable and DLL;
missing FFmpeg cannot be satisfied from System32. Other dependencies must be
bundled, a Windows API set, or present in System32. This does not establish that
a clean machine has the same system components: clean-host runtime is mandatory.

## Sign and notarize

Unsigned macOS staging uses explicit ad-hoc signatures, required for modified
arm64 Mach-O files. For release, provide names of existing keychain identities:

```sh
export VIBECUT_MAC_SIGN_IDENTITY='Developer ID Application: <maintainer>'
export VIBECUT_MAC_INSTALLER_IDENTITY='Developer ID Installer: <maintainer>'
export VIBECUT_NOTARY_PROFILE='<existing notarytool keychain profile>'
# Rerun package.py with a fresh --output directory, then:
pkgutil --check-signature target/package-macos/VibeCut-0.1.0-macos-aarch64.pkg
spctl --assess --type install --verbose target/package-macos/VibeCut-0.1.0-macos-aarch64.pkg
```

The script signs dylibs, helper, app executable, app bundle, then package. Real
identities enable hardened runtime and secure timestamps. It verifies each code
signature, submits the signed package via `notarytool --keychain-profile`, requires
`Accepted`, then staples and validates the ticket. A profile never selects an
identity implicitly. Library validation remains enabled in release builds.

Windows signing reads `VIBECUT_WINDOWS_CERT_SHA1` (certificate in CurrentUser/My)
and `VIBECUT_WINDOWS_TIMESTAMP_URL` (RFC3161 for PE files; configure a service also
supporting PowerShell Authenticode timestamps). It signs DLLs, helper, app, then
uninstaller and installer scripts, verifying each. ZIP is an envelope, with its
SHA-256 in `receipt.json`; it has no Authenticode signature. Use signed installer
scripts under the machine's normal execution policy; do not globally weaken it.
When notarization is configured, macOS additionally produces a signed, notarized
and stapled `.dmg` containing the stapled app and `install-cli.sh`. Drag the app
to Applications, then run `sh /Volumes/VibeCut/install-cli.sh`; add `$HOME/.local/bin`
to PATH as instructed. This route needs only the Developer ID Application identity.
The `.pkg` requires a separate Developer ID Installer identity for a signed,
notarized installer; without that identity it remains a local unsigned test package.
Do not distribute that unsigned package as the signed release.

Instead of a keychain profile, a private build environment can provide
`VIBECUT_NOTARY_KEY_FILE`, `VIBECUT_NOTARY_KEY_ID` and `VIBECUT_NOTARY_ISSUER_ID`.
Never put those values into reports or source control. Signing/notary arguments are
redacted from command receipts. The bundle ID is `com.canhta.vibecut`; helper and
library signature IDs derive from it. No signing identity, timestamp URL or notary
profile is selected by default.

## Install and smoke

On a clean Apple Silicon Mac, mount the signed image using `hdiutil attach`,
copy VibeCut.app to Applications and run its accompanying `install-cli.sh`.
Assess the image with `spctl --assess --type open --context context:primary-signature
--verbose <dmg>` and the app with `spctl --assess --type execute --verbose <app>`;
validate both staples using `xcrun stapler validate <path>`. Run the smoke with
`--signed`: hardened runtime ignores dyld environment diagnostics, so this mode
checks signatures/Gatekeeper/staples and captures actual app dylib mappings using
`lsof`. CLI `probe` plus bundled-only static runpaths exercise the signed CLI;
its transient mapped paths are not captured in this mode.

For an unsigned local package test, install using Installer or:

```sh
sudo installer -pkg target/package-macos/VibeCut-0.1.0-macos-aarch64.pkg -target /
command -v vibecut-cli
ffmpeg -f lavfi -i testsrc2=size=64x64:rate=24 -t 1 -c:v mpeg4 target/package-probe.mp4
python3 packaging/smoke.py --installed /Applications/VibeCut.app \
  --media target/package-probe.mp4 --output target/installed-smoke
```

The oracle fixture generator is external and not distributed. Smoke runs real
`probe`, project creation, `exec add_track`, project info, and an eight-second
window launch from an unrelated directory with sanitized library environment and
system-only PATH. It requires valid JSON, success, completed app observation and
a captured window. On macOS dyld receipts must show bundled FFmpeg paths for every
process. Record `otool -L` and `otool -l` for installed executables and dylibs;
only bundled `@rpath` or system paths are admitted by packaging.

On clean Windows, install using `install.ps1`, then run:

```powershell
Get-Command vibecut-cli
python packaging/smoke.py --installed "$env:LOCALAPPDATA\Programs\VibeCut" --media target/package-probe.mp4 --output target/installed-smoke
Get-ChildItem "$env:LOCALAPPDATA\Programs\VibeCut\*.dll", "$env:LOCALAPPDATA\Programs\VibeCut\*.exe" | ForEach-Object { dumpbin /dependents $_.FullName }
$p = Start-Process "$env:LOCALAPPDATA\Programs\VibeCut\vibecut.exe" -PassThru
Start-Sleep -Seconds 3
$p.Refresh()
$p.Modules | Select-Object ModuleName,FileName | ConvertTo-Json | Set-Content target/loaded-modules.json
Stop-Process -Id $p.Id
powershell -NoProfile -File "$env:LOCALAPPDATA\Programs\VibeCut\uninstall.ps1"
```

Inspect the module receipt: every FFmpeg DLL must resolve beside the executable,
including when launched from a directory containing unrelated DLLs. Repeat with
replacement DLLs. Windows runtime and clean-machine installation remain unverified until these
commands run on provisioned hosts. Signing/notarization claims require the
corresponding successful receipt and external platform verification.
MCP stdio installation smoke is deferred to its owning ticket `p2-platform/02`;
no MCP executable or passing MCP smoke is claimed here.

## Library replacement and recovery

FFmpeg is LGPL-2.1-or-later; reverse engineering for debugging library changes is
permitted. The included archives are the corresponding FFmpeg, Opus and libvpx
sources, verified against the build receipt, plus applied backports and build
recipe. Extract the `.archive` files using `tar -xf`; their original URLs/formats
are in BUILD.txt. Rebuild compatible shared libraries using the recorded configure
flags and backports in an isolated prefix; do not overwrite the shipped sources.
Restore the original libraries from the recorded build prefix or a fresh extraction
of the same installer to undo a replacement. No application rebuild is required.

On macOS, make a copy of VibeCut.app, replace files in Contents/Frameworks with
ABI/architecture-compatible dylibs, retain their `@rpath/<filename>` install IDs
and `@loader_path` runpath, then re-sign each replacement, both executables and the
bundle inside out with `codesign --force --sign - <path>`. Ad-hoc re-signing the
whole app permits local replacement without the original developer's identity;
this copy loses the original notarized identity. Run the smoke again and inspect
dyld paths. For untouched originals from the build prefix, use `replace-macos.py`,
which fixes runpaths and re-signs in that order. Never modify your only app copy.

On Windows, close the app, back up the DLLs, copy compatible replacements beside
both executables, then rerun smoke and module inspection. Library replacement
does not require rebuilding the app or a certificate; enterprise execution policy
may require administrator coordination. Restore backups to recover.

Packaging fails closed on source mismatch, missing licences/receipt, unsupported
host/architecture, unbundled dependencies or signing/notarization errors. Inspect
stdout/stderr and the failed `receipt.json`; fix the input and rerun with a new
output directory. For notary rejection use `xcrun notarytool log <submission-id>
--keychain-profile <profile>`; for transient staple failure retry `xcrun stapler
staple <pkg>` and `validate <pkg>` after confirming an Accepted submission. A
failed/unsigned build must never be described as release accepted. To uninstall
the macOS CLI, remove only its installed symlink after checking `readlink` points
to this app; remove the app through Finder. Keep projects and preferences.

Source-documented platform contracts: [Apple signing order](https://developer.apple.com/documentation/xcode/using-the-latest-code-signature-format),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution),
[Windows DLL search](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order),
[SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).
