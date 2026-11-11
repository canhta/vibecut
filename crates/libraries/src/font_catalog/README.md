# Font catalog

## Font catalog

`fonts::FontStore` owns the font catalog and download cache; `text` owns resolution and raster. A store lists families from four origins and loads them on demand:

- Bundled families come from `text::families` with their OFL-1.1 licence and source.
- Custom families are explicitly imported user files stored by content hash under the same root.
- System families come from `fontdb` (`scan_system`; `scan_directory` for a fixture directory). The scan runs once, on first use, and records family, weight, italic, path and face index only; no font file is admitted to `text` until `load`.
- Google families are a curated table (`curated_google_families`) of OFL-1.1 fonts with Vietnamese coverage (Bebas Neue excepted). Each file is pinned by URL, size and SHA-256 to one upstream commit, plus the family's licence text. Static faces only: variable axes are outside `text`'s limits.

`FontStore::native()` takes its named font path from
[engine runtime's `CacheRoot`](../../../engine/src/runtime/README.md#default-cache-location),
without reading, creating or fetching anything at construction. Unavailable cache discovery
returns `EditError::Unsupported` directly; explicit `FontStore::new` roots remain caller-owned.

`list(query)` returns one entry per family name, `query` matched case-insensitively as a substring, prefix matches first, then alphabetical. Precedence for a name that appears more than once: bundled, verified custom import, cached download, system install, download not cached yet. `load(family)` registers the family's faces with `text::register` and returns a typed `FontLoad` state (`loaded`, `not_downloaded`, `cache_invalid`, `unknown`) with the `effective_family` layout will use: the resolved family name when loaded, otherwise Inter. Unknown and not-downloaded families are states, not errors, so a project that names a missing font still opens and renders deterministically in Inter; IO failures are errors, and a family whose faces are all unreadable fails. Registration is append-only per process and bounded (256 faces, 32 MiB per file, 256 MiB total); a second file under the same family and style keeps the first.

Downloads never run implicitly. `submit_download(runner, family, accepted)` needs the caller's acknowledgement of the displayed source and licence (`fonts.download` carries it as `accept_license`; false is `permission`) and a catalog family. It is a network job (`JobKind::DownloadFont`, retried like model transfers): every file streams into a private `.partial-*` staging directory, is verified against its pinned size and SHA-256, then the finished directory is renamed into `<cache>/VibeCut/fonts/<slug>` inside the job's publication fence. A failed, cancelled or hash-mismatched job removes the staging directory; one job per family runs at a time. Each entry holds the font files, the family's `OFL.txt` and a `<file>.attribution` record in the [asset sidecar format](../../../../docs/engineering/open-source.md#assets) with the file URL, licence id and `license-file`.

Rendering never uses the network. `list` and `load` verify any cached download's font and licence files against the catalog pins (existence, regular file, size and SHA-256; each font's attribution sidecar must exist). `list` reports one entry per family name by precedence: bundled, verified custom import, verified cached download, available system install, then a missing or invalid download reported as not downloaded. `load` returns an early bundled match, otherwise a verified custom import, otherwise accepts a verified cached download, otherwise tries installed system faces; a missing Google cache is `not_downloaded` and a damaged one `cache_invalid` only when neither bundled nor system faces can satisfy the request. Invalid downloaded bytes are never served; the next explicit download replaces the directory.

`fonts.list`, `fonts.load` and `fonts.download` are registry entries (effects `read`, `read` and `job_control`) executed against the store the composition root lends on each request as `Caller::fonts`; a caller without one gets `FontError::Unavailable`. They are outside project history and persistence. `fonts.download` returns `job_accepted` with no published paths; poll `job_status` and call `fonts.load` afterwards. The CLI control session lends `FontStore::native()`, which uses the shared HTTPS transport, the `fontdb` system scan and the per-user cache.

## Custom import

`submit_import(runner, path, accept_license)` / `fonts.import` copies one regular
TTF, OTF or WOFF2 only after the human acknowledges usage rights. Source and decoded
files are bounded to 32 MiB with at most 128 tables; directory ranges, unique
tags and required metadata/outline tables are checked. WOFF2 admits single-font
TrueType/CFF containers and 4 MiB of transformed data. Collections, WOFF1, corrupt or
unsupported input refuse as `FontError::InvalidImport`; expanded directory lengths
are checked before decode allocation. Existing `fontdb` and `skrifa` parse face/name
metadata, while MIT `wuff` supplies the missing WOFF2 decoder with only its safe
Brotli feature. Test-only Brotli encoding derives containers from admitted fixtures.

One import per store plus an OS file lock serializes publication across processes.
The job reserves 512 MiB for source, decoder and reconstruction scratch and checks
cancellation before reads and the publication fence. The store admits 128 files,
64 faces per family and 256 MiB of decoded bytes. Exact decoded SHA-256 duplicates
succeed without another entry; reads enforce the same family/aggregate quotas before
opening face data, including for externally changed metadata. Different bytes for the
same Unicode-lowercase family and ASCII-insensitive style or a bundled/system/catalog family refuse `Conflict`. Private staging
is removed on failure or cancellation; publication renames the complete directory
without replacing an existing entry. Arbitrary ancestor replacement races remain
subject to the filesystem owner's ordinary path limitations.

Each `custom-<decoded-sha256>` directory contains `font.sfnt` and a bounded version-1
`import.json` with normalized family/style, weight, italic, original and decoded
SHA-256, source `local:sha256:<original-hash>`, embedded licence description (or an
explicit unknown description) and acknowledged usage rights. Embedded metadata is
provenance, not a verified licence grant. Original paths are never persisted or
returned. A font moved after copying remains available. Foreign entries are retained.
`list` advertises only hash-verified custom files; `load` verifies all requested
family faces before registration and reports `cache_invalid` for a missing/damaged
file. Custom load precedes catalog/system load; bundled names stay reserved.

Imports are human-granted job controls outside project history, with no output paths
or font bytes. An MCP client cannot grant itself source-file permission. Selection
uses the existing text command and stores only the family/style key. `load_project`
loads referenced custom text and caption families when the app/control session opens
or the CLI exports; preview and export share the text resolver. Missing families
keep their authored key and the resolver's deterministic bundled fallback; missing
glyphs follow the existing text fallback chain. Registration remains process-lifetime.
