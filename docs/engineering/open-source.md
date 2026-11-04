# Open-source policy

## Licensing

Project contributions use [MIT](../../LICENSE). Admit dependencies licensed under
MIT, Apache-2.0, BSD, ISC, Zlib or Unicode, WTFPL or unmodified MPL-2.0; LGPL-2.1-or-later shared FFmpeg is admitted under the distribution rules below;
any other license requires an ADR. Review license metadata when changing dependencies.

## FFmpeg distribution

Use dynamic LGPL-only FFmpeg builds: disable GPL, nonfree and automatic discovery
of external libraries; exclude x264, x265 and FDK-AAC. Hardware backends use the
platform codecs through FFmpeg. Native AAC and MP3 decode are allowed. libvpx and
libopus use BSD licenses; dav1d uses BSD-2-Clause and may be admitted for AV1
decode. libmp3lame is LGPL and may be used for encoding only; its GPL mpglib
code is excluded. Additional components require a license/build review.

A shipping bundle includes the exact resolved build recipe, source identity and
measured/verified hashes, license texts, notices and corresponding FFmpeg/library
source. Keep the corresponding source available with the binary distribution;
a link to an evolving upstream branch alone is insufficient. Identify FFmpeg and
its LGPL license in the app/distribution documentation. Allow reverse engineering
for debugging changes to the libraries; impose no terms that prevent replacing
or relinking the LGPL components. These rules are release requirements, not
proof that an unsigned development build is ready to distribute.

On macOS put dylibs in `Contents/Frameworks`, use `@rpath` install names and an
`@executable_path/../Frameworks` runpath, sign nested libraries before the app,
and notarize the complete bundle with Developer ID/hardened runtime. Supply a
documented replacement/re-signing path and relinkable application materials
where library validation prevents a modified library from loading. Do not claim
LGPL replaceability from dynamic linking alone. Shipping signing/notarization
and modified-library replacement need explicit verification.

On Windows place architecture-matched DLLs alongside the executable in the
application directory; avoid searching arbitrary working directories. Include
the same notices, source/build materials and replacement instructions. Test a
modified compatible DLL without rebuilding the application. Never substitute a
GPL system installation for the admitted distribution.

Primary sources: [FFmpeg license](https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md),
[LGPL 2.1 section 6](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html#SEC6),
[Apple library validation](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation),
[notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution),
[libvpx license](https://github.com/webmproject/libvpx/blob/main/LICENSE),
[Opus license](https://github.com/xiph/opus/blob/main/COPYING),
[dav1d license](https://code.videolan.org/videolan/dav1d/-/blob/master/COPYING),
[LAME license](https://lame.sourceforge.io/license.txt).

## Assets

Keep media, model weights and large binaries out of Git. Admitted small assets
carry `<file>.attribution` sidecars with nonempty `asset`, `title`, `author`, `source`,
`license`, `license-file`, `added` and `notes` fields; READMEs and sidecars are exempt.
`added` identifies the admitting task, not a date. License paths are repository-relative.
Admitted font assets may use OFL-1.1; ship the OFL text, attribution sidecar and, for a modified
subset, the pinned regeneration procedure and hashes. Project-owned brand/icon sources and
committed exports are admitted small assets;
MIT applies. App/window icons use
[assets/app-icon](../../assets/app-icon/README.md); [brand vectors](../brand/logo.svg)
serve documentation. Regeneration and planned platform wiring live in the icon README.
[Verification](verification.md#checks) owns generated fixture placement.

## Model weights

Never bundle model weights in the app or installer; keep downloads small. The user
starts every download from the model manager. Suggest as defaults only weights under
MIT, Apache-2.0 or BSD licences. Other weights (CC-BY, custom or non-commercial) are
opt-in: show the exact licence, source and restrictions such as "non-commercial"
before download. Verify the pinned hash and never redistribute weights with the app.

## Privacy

Local-first is the default. Remote AI is opt-in: obtain explicit consent before
sending media or transcripts. Never log API keys or transcript text, and never
commit credentials or personal recordings. Review provider boundaries and error
paths for leaks. Use [private reporting](../../SECURITY.md) for vulnerabilities.
