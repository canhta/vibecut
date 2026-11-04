# Open-source policy

## Licensing

Contributions are [MIT](../../LICENSE). Admitted dependency licences: MIT,
Apache-2.0, BSD, ISC, Zlib, Unicode, Unlicense, WTFPL, unmodified MPL-2.0, and
LGPL-2.1-or-later for shared FFmpeg under the rules below. Any other licence needs
an ADR.

## FFmpeg distribution

Build dynamic, LGPL-only FFmpeg: GPL, nonfree and automatic external-library
discovery off; no x264, x265 or FDK-AAC. Platform hardware codecs run through
FFmpeg. Admitted extras: libvpx, libopus and dav1d (BSD), libmp3lame for encoding
only (its GPL mpglib excluded). Any other component needs a licence review.

Every shipped bundle carries the resolved build recipe, source identity and hashes,
licence texts and notices, and the corresponding FFmpeg source (a link to a moving
branch is not enough). App docs name FFmpeg and its LGPL licence; our terms allow
reverse engineering for debugging and replacing the LGPL libraries.

- **macOS:** dylibs in `Contents/Frameworks` with `@rpath` install names and an
  `@executable_path/../Frameworks` runpath; sign nested libraries first, then
  notarize the bundle (Developer ID, hardened runtime). Document how a user replaces
  and re-signs a library where library validation blocks a modified one.
- **Windows:** architecture-matched DLLs beside the executable; never load FFmpeg
  from the working directory or a system install. Test a modified compatible DLL
  without rebuilding the app.

Release checks verify signing, notarization and library replacement; a development
build proves none of them. Sources:
[FFmpeg](https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md),
[LGPL 2.1 §6](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html#SEC6),
[library validation](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation),
[notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution),
[libvpx](https://github.com/webmproject/libvpx/blob/main/LICENSE),
[Opus](https://github.com/xiph/opus/blob/main/COPYING),
[dav1d](https://code.videolan.org/videolan/dav1d/-/blob/master/COPYING),
[LAME](https://lame.sourceforge.io/license.txt).

## Assets

Media, model weights and large binaries stay out of Git. Each admitted small asset
has a `<file>.attribution` sidecar with nonempty `asset`, `title`, `author`,
`source`, `license`, `license-file`, `added` (admitting ticket) and `notes`;
`cargo xtask assets` checks them. Fonts may be OFL-1.1 with the OFL text and, for a
subset, its regeneration procedure and hashes. Project brand and icon sources are
MIT; see [app icons](../../assets/app-icon/README.md).

## Model weights

Weights are never bundled: the user starts every download from the model manager.
Only MIT, Apache-2.0 or BSD weights are suggested as defaults. Other weights are
opt-in after showing their exact licence, source and restrictions. Downloads are
hash-verified.

## Privacy

Local-first by default. Remote AI runs only after explicit consent that shows what
is sent, estimated cost and the provider's retention and training policy. Keys,
transcripts and user text never reach logs. Keys and every other non-public project
file (private datasets, recordings, account notes) live in git-ignored `.privates/`
(keys in `.privates/.env`); their contents stay out of reports, logs and commits.
Report vulnerabilities [privately](../../SECURITY.md).
