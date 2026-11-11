# Open-source policy

## Licensing

Contributions are [GPL-3.0-only](../../LICENSE). Admitted dependency licences: MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, Unlicense, WTFPL, CDLA-Permissive-2.0, unmodified MPL-2.0, LGPL-2.1-or-later for shared FFmpeg, and GPL-3.0 for FFmpeg with libx264. Any other licence needs an ADR.

## FFmpeg distribution

Build dynamic FFmpeg with nonfree and automatic external-library discovery off. Linux enables the GPL build and libx264 so software H.264 export works without a GPU; macOS and Windows keep their platform encoders. Admitted extras: libvpx, libopus and dav1d (BSD), zlib (Zlib, statically linked so PNG packets decode), libx264 (GPL-3.0), libmp3lame for encoding only (its GPL mpglib excluded). Any other component needs a licence review.

Every shipped bundle carries the resolved build recipe, source identity and hashes, licence texts and notices, and the corresponding FFmpeg source (a link to a moving branch is not enough). App docs name FFmpeg and its licence (LGPL where used, GPL-3.0 on Linux); our terms allow reverse engineering for debugging and replacing the libraries.

- **macOS:** dylibs in `Contents/Frameworks` with `@rpath` install names and an `@executable_path/../Frameworks` runpath; sign nested libraries first, then notarize the bundle (Developer ID, hardened runtime). Document how a user replaces and re-signs a library where library validation blocks a modified one.
- **Windows:** architecture-matched DLLs beside the executable; never load FFmpeg from the working directory or a system install. Test a modified compatible DLL without rebuilding the app.
- **Linux:** shared `libav*`/`libvpx`/`libopus` libraries under the managed prefix's `lib/`, with an `$ORIGIN/../lib` runpath on the managed tools and `LD_LIBRARY_PATH` for Cargo children. Linux ships the tarball described in [packaging](../../packaging/README.md); signing and notarization checks remain macOS-only.

Release checks verify signing, notarization and library replacement; a development build proves none of them. Sources: [FFmpeg](https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md), [LGPL 2.1 §6](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html#SEC6), [GPL-3.0](https://www.gnu.org/licenses/gpl-3.0.html), [x264 COPYING](https://code.videolan.org/videolan/x264/-/blob/stable/COPYING), [library validation](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation), [notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), [libvpx](https://github.com/webmproject/libvpx/blob/main/LICENSE), [Opus](https://github.com/xiph/opus/blob/main/COPYING), [zlib](https://zlib.net/zlib_license.html), [dav1d](https://code.videolan.org/videolan/dav1d/-/blob/master/COPYING), [LAME](https://lame.sourceforge.io/license.txt).

## Assets

Media, model weights and large binaries stay out of Git. Bundled content (stickers, icons, looks) comes only from sets under a licence admitted above, CC0 or OFL-1.1. [ADR 0007](../adr/0007-big-buck-bunny-documentation.md) admits one documented exception: a credited 12-second Big Buck Bunny excerpt (CC-BY-3.0) for README media and explicit local media fixtures; the film and excerpts stay out of Git, and this does not admit other CC licences or built-in content. Audio the user fetches item by item may also be under an attribution-only licence: each saved item keeps author, source, licence and licence link, and export writes them. Non-commercial, share-alike and no-derivatives items are never offered. The app holds no credential for a content source; a source that needs a key uses the user's own. Each admitted small asset has a `<file>.attribution` sidecar with nonempty `asset`, `title`, `author`, `source`, `license`, `license-file`, `added` (admitting ticket) and `notes`; `cargo xtask assets` checks them. Fonts may be OFL-1.1 with the OFL text and, for a subset, its regeneration procedure and hashes. The curated downloadable font catalog is [libraries-owned](../../crates/libraries/src/font_catalog/README.md#font-catalog): fonts reach a user's cache only on explicit action, pinned by size and SHA-256, and each cached file keeps the same `.attribution` record (source URL, licence id, licence file) beside it. Project brand and icon sources are MIT; see [app icons](../../assets/app-icon/README.md).

## Model weights

Weights are never bundled: the user starts every download from the model manager. Only MIT, Apache-2.0 or BSD weights are suggested as defaults. Other weights are opt-in after showing their exact licence, source and restrictions. Downloads are hash-verified.

## Privacy

Local-first by default. Remote AI runs only after explicit consent that shows what is sent, to which provider (linking its own policy page) and estimated cost; the provider and model are the user's choice, so we do not restate their terms. Logs follow [standards](standards.md#logging). Keys and every other non-public project file (private datasets, recordings, account notes) live in git-ignored `.privates/` (keys in `.privates/.env`); their contents stay out of reports, logs and commits. Report vulnerabilities [privately](../../SECURITY.md).

Remote model discovery is an explicit **Refresh models** action: one authenticated
provider model-list GET sends only the selected key, with no project, transcript,
audio or other user content. Opening settings never requests the list. Offline or
without a valid key, defaults, recent IDs and custom entry remain usable. Model-list
responses are bounded and transient; keys, bodies and approvals are never preferences.
