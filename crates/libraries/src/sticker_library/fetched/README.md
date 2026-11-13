# Sticker collections

Downloadable emoji and icon stickers. The Stickers panel's Emoji and Icons categories come from three pinned collections; the Flags category is the bundled set in the parent module and Shapes are graphic definitions. Nothing here is bundled and nothing is fetched until the user starts a download.

| Module | Responsibility | Contract |
|---|---|---|
| `collections` | The collection table and its owned form. | Each collection records id, name, category, author, repository, full commit, licence id, licence-text path, size and SHA-256. `validate` rejects malformed ids, paths, hashes and duplicate values; `StickerStore::new` refuses such a catalog. |
| `entries` | Generated table of the 420 curated files. | One row per file: id value, name, keywords, repository path, size, SHA-256, all at the collection's commit. |
| `sanitize` | `check_svg`, the admission check for sticker bytes. | Deny by default; bytes are never rewritten. |
| `store` | `StickerStore`: cache state, verified reads, offline test. | `state` is cheap (names and sizes); `read` checks size, SHA-256, sidecar and `check_svg`. |
| `downloads` | `submit_download`, the explicit transfer job. | `JobKind::DownloadSticker` over staging, verification and one rename; cancel leaves nothing. |
| `render_source` | `StickerStore` as render's `StickerSource`, and `native_store`. | `svg` returns `read` bytes with a white root `color` added when the root names none (so `currentColor` draws white); a missing file is `NotDownloaded`, changed or unsafe bytes `Corrupt`, an id outside the catalog `Unknown`. `native_store` opens the per-user store once and lends it to render. |
| `search` | Category search over flags, emoji and icons. | Name and keyword match; collections that are not `Ready` are reported, never listed. |

## Collections

Licences were read from each collection's own licence file at the pinned commit (2026-11-13).

| Id | Collection | Category | Licence | Commit | Files |
|---|---|---|---|---|---|
| `noto` | [Noto Emoji](https://github.com/googlefonts/noto-emoji) `2D/svg` by Google | Emoji | Apache-2.0 (`2D/svg/LICENSE`) | `99701e07d3d1fe1bdbcdb14964547f0fd592966f` | 206 |
| `lucide` | [Lucide](https://github.com/lucide-icons/lucide) | Icons | ISC (`LICENSE`) | `64502ae56389321a922bd498709d4184b0e61543` | 109 |
| `phosphor` | [Phosphor Icons](https://github.com/phosphor-icons/core) `assets/regular` | Icons | MIT (`LICENSE`) | `2b75f3ad12b420c9504ef05df8d2564a28f8500e` | 105 |

- Noto: the images carry the Apache-2.0 notice of `2D/svg/LICENSE`; the repository's fonts are OFL-1.1 and are not used. The region-flag folder (`third_party/region-flags`, its own public-domain notice) is not curated: flags come from the bundled flag-icons set, and no regional-indicator or flag-sequence emoji is listed.
- Lucide: the licence file is ISC plus an MIT block for icons derived from Feather. The curated icons exclude every name that block lists, so only the ISC terms apply.
- Not used: Fluent Emoji (MIT, admitted) because Noto covers the category with one style and the repository's `assets/` is 260 MB; Twemoji (CC BY 4.0) and OpenMoji (CC BY-SA 4.0) per the coordinator decision of 2026-11-10 (a sticker cannot carry a credit on the picture); no CC BY collection is built.
- Fourteen Noto files that start with a DTD or processing instruction fail `check_svg` and are not in the table.

## Cache and records

`<cache>/VibeCut/stickers/<collection id>/` holds `LICENSE`, `<value>.svg` and, beside each, a `.attribution` record: the asset sidecar keys of [open-source](../../../../../docs/engineering/open-source.md#assets) plus `collection` and `commit`. `state` is `Ready` only when the directory holds exactly the pinned files, each a regular file of the pinned size with a complete sidecar; a missing, resized, foreign or unrecorded file makes it `Invalid` and nothing is served. The next explicit download replaces the directory. Bundled flags follow the same rule under [assets/stickers](../../../../../assets/stickers/flags/README.md): a test fails on a file without a complete sidecar.

## Download

`submit_download(runner, collection, accepted)` needs the acknowledgement of the displayed source and licence (false is `ConsentRequired`). One job per collection runs at a time. Every file streams into a `.partial-*` staging directory, is checked against its pinned size and SHA-256 and by `check_svg`, and the finished directory is renamed into place inside the job's publication fence. Progress is bytes over `Collection::download_bytes`. A network failure surfaces as the transient `NetworkTransient` error (`is_offline`), is retried like model transfers, and leaves no entry. The job output is empty; poll the job, then `state`.

## Admission check

`check_svg` refuses: more than 64 KiB, invalid UTF-8, malformed XML, any DTD, a processing instruction, a root that is not an SVG element, any element outside `svg g defs path circle ellipse rect line polyline polygon linearGradient radialGradient stop clipPath mask use title desc` or outside the SVG namespace (so `script`, `foreignObject`, `image`, `style`, `a`, `animate` and `set` are refused), more than 20,000 nodes or 64 levels, an `on*` attribute, an `href` that does not start with `#`, a `url(` that does not start with `#`, and `javascript:`, `@import`, `expression(` or `data:` in any attribute value.

## Search and ids

An id is `<collection id>:<value>` (`noto:1f600`, `lucide:heart`) and is what a project stores. Fetched stickers record the intrinsic size 256x256. `StickerStore::search(query, category, limit)` matches every word of the query against the name and the keywords (Noto: the Unicode name; Lucide: its tags; Phosphor: the file name); `None` searches Emoji, Flags, Icons in that order. Recent ids are the parent module's `Recents`.

## Attribution

Each provider id has a credit line in `project::StickerParams::credit`; the export writes the lines of the providers a project uses into the container `comment` tag beside the sound and music records. A test checks that every collection here has a credit naming its author and licence id.

## Regenerating `entries`

Pick files from the pinned trees, download each at its commit, record size and SHA-256, and run `check_svg` on it; the table holds only files that pass. Raise a commit only together with the licence file read again at the new commit.
