# Optional project metadata

`WriterLease::metadata` reads `<canonical project filename>.meta.json`, bounded to
512 KiB, with metadata schema version 3. Missing, unreadable, corrupt, oversized or
unsupported metadata defaults without blocking the project. `Metadata` contains
created/modified Unix milliseconds, the current `ViewState` and up to 256 scene-ID-keyed views (positive zoom at most 1000, horizontal
scroll 0–1e9 logical pixels, nonnegative integer-tick playhead). Nonfinite values
are refused. This sidecar is outside the editable project envelope and does not
change its schema. Thumbnail is Missing, Unavailable or Ready with 1..=128-pixel dimensions and
exactly width × height × 4 sRGB RGBA bytes (at most 64 KiB). Invalid dimensions,
byte counts and oversized sidecars default on read and refuse on write. No image
decoder runs on metadata. The 8-character display ID is derived from the project
ID by engine, never duplicated here. No project index is stored here.
`read_metadata(path)` supports bounded read-only catalog display; mutations remain
under WriterLease. `WriterLease::validate` checks its baseline and stable lock
identity without requiring a project file to exist.

`WriterLease::save_metadata` validates then atomically replaces the sidecar under
the existing canonical project lease and content precondition. Sidecar symlinks
are refused. Project writer destinations reserve the `.meta.json` suffix, including
case variants and canonical aliases, so saving a project cannot replace another
project's metadata. A sidecar write error is returned; it does not roll back a
previously successful project publication. Callers acknowledge a save only after
both writes succeed. `tests/it/metadata.rs` covers bounds, fallback, native paths,
symlink refusal, external project replacement and round-trip independence.
