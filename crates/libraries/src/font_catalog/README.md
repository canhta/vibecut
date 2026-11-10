# Font catalog

## Font catalog

`fonts::FontStore` owns the font catalog and download cache; `text` owns resolution and raster. A store lists families from three origins and loads them on demand:

- Bundled families come from `text::families` with their OFL-1.1 licence and source.
- System families come from `fontdb` (`scan_system`; `scan_directory` for a fixture directory). The scan runs once, on first use, and records family, weight, italic, path and face index only; no font file is admitted to `text` until `load`.
- Google families are a curated table (`curated_google_families`) of OFL-1.1 fonts with Vietnamese coverage (Bebas Neue excepted). Each file is pinned by URL, size and SHA-256 to one upstream commit, plus the family's licence text. Static faces only: variable axes are outside `text`'s limits.

`FontStore::native()` takes its named font path from
[engine runtime's `CacheRoot`](../../../engine/src/runtime/README.md#default-cache-location),
without reading, creating or fetching anything at construction. Unavailable cache discovery
returns `EditError::Unsupported` directly; explicit `FontStore::new` roots remain caller-owned.

`list(query)` returns one entry per family name, `query` matched case-insensitively as a substring, prefix matches first, then alphabetical. Precedence for a name that appears more than once: bundled, cached download, system install, download not cached yet. `load(family)` registers the family's faces with `text::register` and returns a typed `FontLoad` state (`loaded`, `not_downloaded`, `cache_invalid`, `unknown`) with the `effective_family` layout will use: the resolved family name when loaded, otherwise Inter. Unknown and not-downloaded families are states, not errors, so a project that names a missing font still opens and renders deterministically in Inter; IO failures are errors, and a family whose faces are all unreadable fails. Registration is append-only per process and bounded (256 faces, 32 MiB per file, 256 MiB total); a second file under the same family and style keeps the first.

Downloads never run implicitly. `submit_download(runner, family, accepted)` needs the caller's acknowledgement of the displayed source and licence (`fonts.download` carries it as `accept_license`; false is `permission`) and a catalog family. It is a network job (`JobKind::DownloadFont`, retried like model transfers): every file streams into a private `.partial-*` staging directory, is verified against its pinned size and SHA-256, then the finished directory is renamed into `<cache>/VibeCut/fonts/<slug>` inside the job's publication fence. A failed, cancelled or hash-mismatched job removes the staging directory; one job per family runs at a time. Each entry holds the font files, the family's `OFL.txt` and a `<file>.attribution` record in the [asset sidecar format](../../../../docs/engineering/open-source.md#assets) with the file URL, licence id and `license-file`.

Rendering never uses the network. `list` and `load` verify any cached download's font and licence files against the catalog pins (existence, regular file, size and SHA-256; each font's attribution sidecar must exist). `list` reports one entry per family name by precedence: bundled, a verified cached download, an available system install, then a missing or invalid download reported as not downloaded. `load` returns an early bundled match, otherwise accepts a verified cached download, otherwise tries installed system faces; a missing Google cache is `not_downloaded` and a damaged one `cache_invalid` only when neither bundled nor system faces can satisfy the request. Invalid downloaded bytes are never served; the next explicit download replaces the directory.

`fonts.list`, `fonts.load` and `fonts.download` are registry entries (effects `read`, `read` and `job_control`) executed against the store the composition root lends on each request as `Caller::fonts`; a caller without one gets `FontError::Unavailable`. They are outside project history and persistence. `fonts.download` returns `job_accepted` with no published paths; poll `job_status` and call `fonts.load` afterwards. The CLI control session lends `FontStore::native()`, which uses the shared HTTPS transport, the `fontdb` system scan and the per-user cache.
