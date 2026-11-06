# Font catalog

## Font catalog

`fonts::FontStore` owns the font catalog and download cache; `text` owns resolution and
raster. A store lists families from three origins and loads them on demand:

- **Bundled** families come from `text::families` with their OFL-1.1 licence and source.
- **System** families come from `fontdb` (`scan_system`; `scan_directory` for a fixture
  directory). The scan runs once, on first use, and records family, weight, italic, path and
  face index only; no font bytes are read until `load`.
- **Google** families are a curated table (`curated_google_families`) of OFL-1.1 fonts with
  Vietnamese coverage (Bebas Neue excepted), each file pinned by URL, size and SHA-256 to one
  commit of the upstream repository, plus the family's licence text. Static faces only:
  variable axes are outside `text`'s limits.

`list(query)` returns one entry per family name, `query` matched case-insensitively as a
substring with prefix matches first and then alphabetical order. Precedence for a name that
appears more than once: bundled, then a cached download, then a system install, then a
download that is not cached yet. `load(family)` registers the family's faces with
`text::register` and returns a typed `FontLoad` state (`loaded`, `not_downloaded`,
`cache_invalid`, `unknown`) with the `effective_family` layout will use: the request when
loaded, otherwise Inter. Unknown and not-downloaded families are states, not errors, so a
project that names a font the machine lacks still opens and renders deterministically in
Inter; IO failures and unreadable font files are errors. Registration is append-only per
process and bounded (256 faces, 32 MiB per file, 256 MiB total); a second file under the
same family and style keeps the first.

Downloads never run implicitly. `submit_download(runner, family, accepted)` needs the
caller's acknowledgement of the displayed source and licence (`fonts.download` carries it as
`accept_license`; a false value is `permission`) and a catalog family. It is a network job
(`JobKind::DownloadFont`, retried like model transfers): every file is streamed into a
private `.partial-*` staging directory, verified against its pinned size and SHA-256, then
the finished directory is renamed into `<cache>/VibeCut/fonts/<slug>` inside the job's
publication fence. A failed, cancelled or hash-mismatched job removes the staging
directory, so no partial cache entry exists; one job per family runs at a time. Each entry
holds the font files, the family's `OFL.txt` and a `<file>.attribution` record in the
[asset sidecar format](../../../../../docs/engineering/open-source.md#assets) with the file URL,
licence id and `license-file`.

Rendering never uses the network. Every `list` and `load` verifies a cached family's files
against the catalog pins (existence, regular file, size, SHA-256, licence and attribution
files); any difference reports `cache_invalid` and the files are not served until the next
explicit download replaces the directory. A new pin therefore invalidates older cached
bytes the same way a tampered file does.

`fonts.list`, `fonts.load` and `fonts.download` are registry entries (effects `read`,
`read` and `job_control`) executed against the store the composition root lends on each request as `Caller::fonts`; a
caller without one gets `FontError::Unavailable`. They are outside project history and persistence.
`fonts.download` returns `job_accepted` with no published paths; poll `job_status` and call
`fonts.load` afterwards. The CLI control session lends `FontStore::native()`, which uses the
shared HTTPS transport, the `fontdb` system scan and the per-user cache.

Tests (`cargo test -p engine --test it font`): directory enumeration with nested, broken and
non-font files; search ordering and duplicate collapse; lazy scan and file reads with the
`text` fallback for unknown families; cold, downloaded and offline warm cache with a counting
transport; hash, size, truncation and missing-record invalidation; acceptance, catalog and
single-job refusals; failed, truncated, oversized, mid-stream-cancelled and late-failing
downloads that leave no entry; hostile catalog data; command results and refusals.
